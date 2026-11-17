//! The canonical archive (K01): cross-language vectors, tamper detection, bounded unpack,
//! path and entry-kind rejection, and staging into a directory.

use std::fs;
use std::io::Read;

use orange_packages::{
    ARCHIVE_MAGIC, ArchiveError, ArchiveFiles, ArchiveLimits, ArchivePath, DirectorySink,
    DiscardSink, MemoryJournal, MemorySink, PackageStore, PathRule, TransactionError,
    VerifiedArchive, read_archive,
};
use serde_json::{Value, json};

use crate::common::{HOST, TempDir, base};

const CANARY: &str = "CANARY-7f3a";

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).unwrap())
        .collect()
}

fn path(text: &str) -> ArchivePath {
    text.parse().unwrap()
}

fn verify(bytes: &[u8]) -> Result<VerifiedArchive, ArchiveError> {
    read_archive(bytes, &ArchiveLimits::default(), &mut DiscardSink)
}

/// A manifest for `id` with a component entry and an `en` catalogue, digest placeholder.
fn manifest(id: &str, dependencies: Value) -> Value {
    json!({
        "format": "orange.plugin", "format_version": 1,
        "id": id, "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "execution": {"kind": "wasm-component", "entry": "component.wasm",
                      "world": "orange:expansion/plugin@1.0.0"},
        "localization": {"format_version": 1, "format": "fluent", "default_locale": "en",
                         "catalogs": {"en": ["locales/en.ftl"]}},
        "dependencies": dependencies,
        "integrity": {"algorithm": "sha256", "content_digest": "0".repeat(64)},
        "license": "MIT"
    })
}

const COMPONENT: &[u8] = b"\0asm\x0d\x00\x01\x00rest";
const CATALOG: &[u8] = b"hello = Hello\n";

/// Fills in the content digest and encodes. Returns the manifest bytes and archive.
fn pack(mut manifest: Value, files: &[(&str, &[u8])]) -> (Vec<u8>, Vec<u8>) {
    let files = ArchiveFiles::new(
        files
            .iter()
            .map(|(name, bytes)| (path(name), *bytes))
            .collect(),
    )
    .unwrap();
    let draft = serde_json::to_vec(&manifest).unwrap();
    let digest = files.content_digest(&draft).unwrap();
    manifest["integrity"]["content_digest"] = json!(digest.strip_prefix("sha256:").unwrap());
    let bytes = serde_json::to_vec(&manifest).unwrap();
    let archive = files.encode(&bytes);
    (bytes, archive)
}

fn sample() -> (Vec<u8>, Vec<u8>) {
    pack(
        manifest("org.example.device", json!([])),
        &[("component.wasm", COMPONENT), ("locales/en.ftl", CATALOG)],
    )
}

#[test]
fn reused_memory_sink_replaces_the_whole_archive_and_payloads_require_authentication() {
    let (_, first) = pack(
        manifest("org.example.device", json!([])),
        &[
            ("component.wasm", COMPONENT),
            ("locales/en.ftl", CATALOG),
            ("old.txt", b"old"),
        ],
    );
    let changed = b"\0asm\x0d\x00\x01\x00different";
    let (_, second) = pack(
        manifest("org.example.device", json!([])),
        &[("component.wasm", changed), ("locales/en.ftl", CATALOG)],
    );
    let mut sink = MemorySink::new();
    let a = read_archive(&first[..], &ArchiveLimits::default(), &mut sink).unwrap();
    a.verify_file(
        &path("component.wasm"),
        sink.file("component.wasm").unwrap(),
    )
    .unwrap();
    let b = read_archive(&second[..], &ArchiveLimits::default(), &mut sink).unwrap();
    assert_ne!(a.content_digest(), b.content_digest());
    assert_eq!(sink.file("component.wasm"), Some(&changed[..]));
    assert_eq!(sink.files().len(), 3);
    assert!(sink.file("old.txt").is_none());
    b.verify_file(&path("component.wasm"), changed).unwrap();
    for wrong in [COMPONENT, &changed[..changed.len() - 1]] {
        let error = b.verify_file(&path("component.wasm"), wrong).unwrap_err();
        assert!(matches!(error, ArchiveError::FileMismatch(_)));
        assert_eq!(error.message_key(), "packages/archive-file-mismatch");
    }
    assert!(matches!(
        a.verify_file(&path("component.wasm"), changed),
        Err(ArchiveError::FileMismatch(_))
    ));
    assert!(matches!(
        a.verify_file(&path("absent"), changed),
        Err(ArchiveError::MissingFile(_))
    ));
    assert!(
        read_archive(
            &second[..second.len() - 1],
            &ArchiveLimits::default(),
            &mut sink
        )
        .is_err()
    );
    assert!(sink.files().is_empty());
}

/// One raw entry: path bytes, kind, mode, declared size and the bytes that follow.
struct Raw<'a>(&'a [u8], u8, u8, u64, &'a [u8]);

/// Encodes entries without any validation, after a valid manifest entry when `manifest` is
/// given. `count` overrides the declared entry count.
fn raw(manifest: Option<&[u8]>, entries: &[Raw<'_>], count: Option<u32>) -> Vec<u8> {
    let mut out = ARCHIVE_MAGIC.to_vec();
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    let total = entries.len() as u32 + u32::from(manifest.is_some());
    out.extend_from_slice(&count.unwrap_or(total).to_le_bytes());
    let mut put = |Raw(name, kind, mode, size, bytes): &Raw<'_>| {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name);
        out.push(*kind);
        out.push(*mode);
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(bytes);
    };
    if let Some(manifest) = manifest {
        put(&Raw(
            b"manifest.json",
            0,
            0,
            manifest.len() as u64,
            manifest,
        ));
    }
    for entry in entries {
        put(entry);
    }
    out
}

/// A data-only manifest, so raw payload entries need no particular files.
fn data_only() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "format": "orange.plugin", "format_version": 1,
        "id": "org.example.data", "version": "1.0.0", "publisher": "org.example",
        "host_api": {"major": 1, "min_minor": 0, "max_minor": 0},
        "integrity": {"algorithm": "sha256", "content_digest": "0".repeat(64)}
    }))
    .unwrap()
}

fn file<'a>(name: &'a str, bytes: &'a [u8]) -> Raw<'a> {
    Raw(name.as_bytes(), 0, 0, bytes.len() as u64, bytes)
}

fn reject(entries: &[Raw<'_>]) -> ArchiveError {
    verify(&raw(Some(&data_only()), entries, None)).unwrap_err()
}

#[test]
fn cross_language_vectors_match_the_reference_encoder() {
    let fixture: Value = serde_json::from_str(include_str!("fixtures/archive-v1.json")).unwrap();
    let manifest = fixture["manifest"].as_str().unwrap().as_bytes();
    let contents: Vec<(String, Vec<u8>)> = fixture["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            (
                file["path"].as_str().unwrap().to_owned(),
                unhex(file["hex"].as_str().unwrap()),
            )
        })
        .collect();
    let files = ArchiveFiles::new(
        contents
            .iter()
            .map(|(name, bytes)| (path(name), bytes.as_slice()))
            .collect(),
    )
    .unwrap();
    assert_eq!(
        files.content_digest(manifest).unwrap(),
        fixture["content_digest"].as_str().unwrap()
    );
    let archive = files.encode(manifest);
    assert_eq!(archive, unhex(fixture["archive_hex"].as_str().unwrap()));

    let mut sink = MemorySink::new();
    let verified = read_archive(archive.as_slice(), &ArchiveLimits::default(), &mut sink).unwrap();
    assert_eq!(verified.content_digest(), fixture["content_digest"]);
    assert_eq!(verified.archive_digest(), fixture["archive_digest"]);
    assert_eq!(sink.file("manifest.json"), Some(manifest));
    for (name, bytes) in &contents {
        assert_eq!(sink.file(name), Some(bytes.as_slice()), "{name}");
    }
    // Entries are in canonical byte order, manifest excluded.
    let order: Vec<&str> = verified
        .entries()
        .iter()
        .map(|e| e.path().as_str())
        .collect();
    assert_eq!(
        order,
        [
            "component.wasm",
            "locales/en.ftl",
            "locales/es.ftl",
            "z/empty.bin"
        ]
    );
}

#[test]
fn the_content_digest_ignores_only_the_digest_field() {
    let (manifest_bytes, _) = sample();
    let files = ArchiveFiles::new(vec![
        (path("locales/en.ftl"), CATALOG),
        (path("component.wasm"), COMPONENT),
    ])
    .unwrap();
    let digest = files.content_digest(&manifest_bytes).unwrap();
    // Reformatting and reordering the manifest keeps the digest; the digest value is excluded.
    let mut value: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    value["integrity"]["content_digest"] = json!("f".repeat(64));
    let pretty = serde_json::to_vec_pretty(&value).unwrap();
    assert_eq!(files.content_digest(&pretty).unwrap(), digest);
    // Any other metadata change moves it.
    value["license"] = json!("Apache-2.0");
    assert_ne!(
        files
            .content_digest(&serde_json::to_vec(&value).unwrap())
            .unwrap(),
        digest
    );
}

#[test]
fn tampered_payload_metadata_or_dependency_lock_fails_verification() {
    let (manifest_bytes, archive) = sample();
    let verified = verify(&archive).unwrap();

    // One payload byte.
    let mut payload = archive.clone();
    let at = payload.len() - 1;
    payload[at] ^= 1;
    let mut sink = MemorySink::new();
    let error = read_archive(payload.as_slice(), &ArchiveLimits::default(), &mut sink).unwrap_err();
    assert!(matches!(error, ArchiveError::ContentDigest { .. }));
    assert_eq!(error.message_key(), "packages/archive-content-digest");
    assert!(sink.files().is_empty(), "a failed read aborts the sink");

    let mut value: Value = serde_json::from_slice(&manifest_bytes).unwrap();
    let files = ArchiveFiles::new(vec![
        (path("component.wasm"), COMPONENT),
        (path("locales/en.ftl"), CATALOG),
    ])
    .unwrap();
    // Metadata: widen the host API range, keep the declared digest.
    value["host_api"]["max_minor"] = json!(9);
    let error = verify(&files.encode(&serde_json::to_vec(&value).unwrap())).unwrap_err();
    assert!(matches!(error, ArchiveError::ContentDigest { .. }));
    // A dependency lock is metadata too.
    let mut locked = serde_json::from_slice::<Value>(&manifest_bytes).unwrap();
    locked["dependencies"] = json!([{"id": "org.example.base", "version": "1.0.0",
                                     "content_digest": format!("sha256:{}", "e".repeat(64))}]);
    let error = verify(&files.encode(&serde_json::to_vec(&locked).unwrap())).unwrap_err();
    assert!(matches!(error, ArchiveError::ContentDigest { .. }));

    // Re-signing the edit with a fresh digest makes another package, which no lock names.
    let (_, repacked) = pack(
        value,
        &[("component.wasm", COMPONENT), ("locales/en.ftl", CATALOG)],
    );
    let other = verify(&repacked).unwrap();
    assert_ne!(other.content_digest(), verified.content_digest());
    assert!(matches!(
        other.require_digest(verified.content_digest()),
        Err(ArchiveError::LockedDigest { .. })
    ));
    verified.require_digest(verified.content_digest()).unwrap();
}

#[test]
fn a_dependent_refuses_a_dependency_whose_bytes_changed() {
    let (_, base_archive) = pack(
        manifest("org.example.base", json!([])),
        &[("component.wasm", COMPONENT), ("locales/en.ftl", CATALOG)],
    );
    let original = verify(&base_archive).unwrap();
    let mut changed = manifest("org.example.base", json!([]));
    changed["license"] = json!("Apache-2.0");
    let (_, changed_archive) = pack(
        changed,
        &[("component.wasm", COMPONENT), ("locales/en.ftl", CATALOG)],
    );
    let dependent = |lock: &str| {
        let (_, archive) = pack(
            manifest(
                "org.example.app",
                json!([{"id": "org.example.base", "version": "1.0.0", "content_digest": lock}]),
            ),
            &[("component.wasm", COMPONENT), ("locales/en.ftl", CATALOG)],
        );
        verify(&archive).unwrap().into_contents(vec![])
    };

    let mut store = PackageStore::new(base(), HOST, MemoryJournal::new());
    store
        .install(verify(&changed_archive).unwrap().into_contents(vec![]))
        .unwrap();
    let error = store
        .install(dependent(original.content_digest()))
        .unwrap_err();
    assert!(matches!(error, TransactionError::DependencyDigest(_)));
}

#[test]
fn bounded_unpack_rejects_bombs_before_reading_their_bytes() {
    let manifest = data_only();
    // A declared size far beyond every quota is refused at its header; the archive is a few
    // dozen bytes, so nothing was allocated by the declared size.
    let error = reject(&[Raw(b"big.bin", 0, 0, u64::MAX, b"")]);
    assert!(matches!(
        error,
        ArchiveError::FileTooLarge { size: u64::MAX, .. }
    ));

    let limits = ArchiveLimits {
        max_archive_bytes: 4_096,
        max_entries: 3,
        max_file_bytes: 1_024,
        max_total_bytes: 1_500,
    };
    let read = |bytes: &[u8]| read_archive(bytes, &limits, &mut DiscardSink).unwrap_err();
    let kib = vec![0u8; 1_000];
    let two = raw(
        Some(&manifest),
        &[Raw(b"a", 0, 0, 1_000, &kib), Raw(b"b", 0, 0, 1_000, &kib)],
        None,
    );
    assert!(matches!(
        read(&two),
        ArchiveError::TotalTooLarge { limit: 1_500 }
    ));
    let many = raw(Some(&manifest), &[], Some(4));
    assert_eq!(
        read(&many),
        ArchiveError::TooManyEntries { count: 4, limit: 3 }
    );
    let tight = ArchiveLimits {
        max_archive_bytes: 600,
        ..limits
    };
    let long = raw(Some(&manifest), &[Raw(b"a", 0, 0, 1_000, &kib)], None);
    assert!(matches!(
        read_archive(long.as_slice(), &tight, &mut DiscardSink).unwrap_err(),
        ArchiveError::ArchiveTooLarge { limit: 600 }
    ));

    // The manifest has its own fixed bound.
    let error = verify(&raw(
        None,
        &[Raw(b"manifest.json", 0, 0, 1 << 40, b"")],
        None,
    ))
    .unwrap_err();
    assert!(matches!(error, ArchiveError::FileTooLarge { .. }));

    // Declared sizes must be backed by bytes, and nothing may follow the last entry.
    let truncated = raw(Some(&manifest), &[Raw(b"a", 0, 0, 10, b"short")], None);
    assert_eq!(verify(&truncated).unwrap_err(), ArchiveError::Truncated);
    let mut trailing = raw(Some(&manifest), &[], None);
    trailing.push(0);
    assert_eq!(verify(&trailing).unwrap_err(), ArchiveError::TrailingData);
}

#[test]
fn headers_reject_other_formats() {
    let good = raw(Some(&data_only()), &[], None);
    let mut zip = good.clone();
    zip[..4].copy_from_slice(b"PK\x03\x04");
    assert_eq!(verify(&zip).unwrap_err(), ArchiveError::Magic);
    let mut version = good.clone();
    version[8] = 2;
    assert_eq!(verify(&version).unwrap_err(), ArchiveError::Version(2));
    // No compression or other flag exists in v1.
    let mut flags = good.clone();
    flags[10] = 1;
    assert_eq!(verify(&flags).unwrap_err(), ArchiveError::Flags(1));
    assert_eq!(verify(&good[..12]).unwrap_err(), ArchiveError::Truncated);
    assert_eq!(
        verify(&raw(None, &[], None)).unwrap_err(),
        ArchiveError::ManifestFirst
    );
    let first = raw(
        None,
        &[file("a.txt", b"x"), file("manifest.json", b"{}")],
        None,
    );
    assert_eq!(verify(&first).unwrap_err(), ArchiveError::ManifestFirst);
}

#[test]
fn paths_reject_traversal_and_platform_aliases() {
    let rule = |name: &'static [u8]| match reject(&[Raw(name, 0, 0, 1, b"x")]) {
        ArchiveError::Path(error) => error.reason,
        other => panic!("{other:?}"),
    };
    assert_eq!(rule(b"../evil"), PathRule::DotComponent);
    assert_eq!(rule(b"a/../../evil"), PathRule::DotComponent);
    assert_eq!(rule(b"/etc/passwd"), PathRule::EmptyComponent);
    assert_eq!(rule(b"..\\evil"), PathRule::Character);
    assert_eq!(rule(b"C:evil"), PathRule::Character);
    assert_eq!(rule(b"file.txt:stream"), PathRule::Character);
    assert_eq!(rule(b"aux.txt"), PathRule::Reserved);
    assert_eq!(rule(b"trail."), PathRule::TrailingDot);
    assert_eq!(rule(b"\xff\xfe"), PathRule::Character);
    assert_eq!(rule(b""), PathRule::Length);
}

#[test]
fn entries_must_be_unique_ordered_regular_files() {
    // Links, directories and unknown kinds are never unpacked.
    for kind in [1u8, 2, 3, 9] {
        assert_eq!(
            reject(&[Raw(b"link", kind, 0, 0, b"")]).message_key(),
            "packages/archive-entry-kind"
        );
    }
    assert!(matches!(
        reject(&[Raw(b"tool", 0, 0o7, 1, b"x")]),
        ArchiveError::EntryMode { mode: 7, .. }
    ));
    // Case-insensitive file systems would merge these.
    assert!(matches!(
        reject(&[file("A.txt", b"1"), file("a.txt", b"2")]),
        ArchiveError::DuplicatePath(_)
    ));
    assert!(matches!(
        reject(&[file("Manifest.json", b"{}")]),
        ArchiveError::DuplicatePath(_)
    ));
    assert!(matches!(
        reject(&[file("a.txt", b"1"), file("a.txt", b"2")]),
        ArchiveError::Order(_)
    ));
    assert!(matches!(
        reject(&[file("b.txt", b"1"), file("a.txt", b"2")]),
        ArchiveError::Order(_)
    ));
    // A path that is a file cannot also be a directory, whichever comes first.
    assert!(matches!(
        reject(&[file("a", b"1"), file("a/b", b"2")]),
        ArchiveError::PathConflict(_)
    ));
    assert!(matches!(
        reject(&[file("D/b", b"1"), file("d", b"2")]),
        ArchiveError::PathConflict(_)
    ));
    // The canonical encoder enforces the same rules.
    assert!(matches!(
        ArchiveFiles::new(vec![(path("x/Y"), b""), (path("X/y"), b"")]),
        Err(ArchiveError::DuplicatePath(_))
    ));
}

#[test]
fn declared_code_must_match_the_payload() {
    let files = |component: &'static [u8]| {
        pack(
            manifest("org.example.device", json!([])),
            &[("component.wasm", component), ("locales/en.ftl", CATALOG)],
        )
        .1
    };
    assert!(matches!(
        verify(&files(b"MZ\x90\x00 native")).unwrap_err(),
        ArchiveError::UndeclaredNativeCode(_)
    ));
    assert!(matches!(
        verify(&files(b"#!/bin/sh")).unwrap_err(),
        ArchiveError::ExecutionPayload(_)
    ));
    assert!(matches!(
        verify(&files(b"")).unwrap_err(),
        ArchiveError::ExecutionPayload(_)
    ));
    // Native code anywhere else is refused unless a worker target declares it.
    let (_, smuggled) = pack(
        manifest("org.example.device", json!([])),
        &[
            ("component.wasm", COMPONENT),
            ("locales/en.ftl", CATALOG),
            ("lib/helper.so", b"\x7fELF\x02\x01"),
        ],
    );
    assert!(matches!(
        verify(&smuggled).unwrap_err(),
        ArchiveError::UndeclaredNativeCode(_)
    ));
    let mut worker = manifest("org.example.device", json!([]));
    worker["execution"] = json!({"kind": "worker", "wire_version": 1, "targets": [
        {"os": "linux", "arch": "x86_64", "entry": "bin/worker"}
    ]});
    let (_, archive) = pack(
        worker,
        &[
            ("bin/worker", b"\x7fELF\x02\x01"),
            ("locales/en.ftl", CATALOG),
        ],
    );
    verify(&archive).unwrap();

    // Every referenced file must exist under exactly that path.
    let (_, missing) = pack(
        manifest("org.example.device", json!([])),
        &[("component.wasm", COMPONENT), ("Locales/en.ftl", CATALOG)],
    );
    assert!(matches!(
        verify(&missing).unwrap_err(),
        ArchiveError::MissingFile(_)
    ));
}

#[test]
fn staging_writes_a_fresh_directory_and_removes_it_on_failure() {
    let parent = TempDir::new("stage");
    let (_, archive) = sample();
    let mut sink = DirectorySink::create(parent.path(), "pkg-1").unwrap();
    let verified = read_archive(archive.as_slice(), &ArchiveLimits::default(), &mut sink).unwrap();
    let root = sink.root().to_owned();
    assert_eq!(fs::read(root.join("component.wasm")).unwrap(), COMPONENT);
    assert_eq!(
        fs::read(root.join("locales").join("en.ftl")).unwrap(),
        CATALOG
    );
    let mut manifest = Vec::new();
    fs::File::open(root.join("manifest.json"))
        .unwrap()
        .read_to_end(&mut manifest)
        .unwrap();
    assert_eq!(
        orange_packages::PackageManifest::from_json(&manifest)
            .unwrap()
            .content_digest(),
        verified.content_digest()
    );
    // A staging name is used once.
    assert!(DirectorySink::create(parent.path(), "pkg-1").is_err());
    for name in ["..", "a/b", "nul", ""] {
        assert!(
            DirectorySink::create(parent.path(), name).is_err(),
            "{name}"
        );
    }

    // A failure after files were written deletes the staging directory.
    let mut tampered = archive.clone();
    let at = tampered.len() - 1;
    tampered[at] ^= 1;
    let mut sink = DirectorySink::create(parent.path(), "pkg-2").unwrap();
    assert!(read_archive(tampered.as_slice(), &ArchiveLimits::default(), &mut sink).is_err());
    assert!(!parent.path().join("pkg-2").exists());
}

#[test]
fn staging_refuses_a_linked_parent() {
    let parent = TempDir::new("stage-link");
    let target = parent.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = parent.path().join("link");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&target, &link).is_ok();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&target, &link).is_ok() || {
        // Symbolic links need a privilege; a junction is a reparse point that does not.
        std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&target)
            .output()
            .is_ok_and(|output| output.status.success())
    };
    #[cfg(not(any(unix, windows)))]
    let made = false;
    if !made {
        // This environment cannot create a link at all; nothing to check.
        return;
    }
    assert!(DirectorySink::create(&link, "pkg").is_err());
    assert!(!target.join("pkg").exists());
}

#[test]
fn archive_errors_hide_package_paths() {
    let canary_path = format!("{CANARY}.txt");
    let cases = [
        reject(&[Raw(format!("../{CANARY}").leak().as_bytes(), 0, 0, 1, b"x")]),
        reject(&[Raw(canary_path.clone().leak().as_bytes(), 2, 0, 0, b"")]),
        reject(&[
            file(&canary_path, b"1"),
            file(&canary_path.to_lowercase(), b"2"),
        ]),
        reject(&[Raw(
            canary_path.clone().leak().as_bytes(),
            0,
            0,
            u64::MAX,
            b"",
        )]),
    ];
    for error in cases {
        assert!(!error.to_string().contains(CANARY), "{error}");
        assert!(!format!("{error:?}").contains(CANARY), "{error:?}");
        assert!(error.message_key().starts_with("packages/archive-"));
    }
}
