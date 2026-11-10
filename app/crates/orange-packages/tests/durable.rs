//! The file-backed journal (K01): appends survive reopen, a torn append is truncated so
//! recovery sees a complete committed set, and compaction replaces the log atomically.

use std::fs;
use std::process::Command;

use orange_packages::{
    FileJournal, JOURNAL_FILE, JOURNAL_TEMP_FILE, Journal, JournalEntry, PackageKey, PackageState,
    PackageStore, Recovery,
};

use crate::common::{HEX_1, HEX_2, HEX_3, HOST, PACKAGE, TempDir, app, base, device, pkg};

fn store(dir: &TempDir) -> PackageStore<FileJournal> {
    let opened = FileJournal::open(dir.path()).unwrap();
    assert!(opened.entries.is_empty());
    PackageStore::new(base(), HOST, opened.journal)
}

fn recover(dir: &std::path::Path) -> (Recovery, u64) {
    let opened = FileJournal::open(dir).unwrap();
    (
        Recovery::from_entries(&opened.entries),
        opened.discarded_tail_bytes,
    )
}

fn log(dir: &TempDir) -> Vec<u8> {
    fs::read(dir.path().join(JOURNAL_FILE)).unwrap()
}

#[test]
fn appends_survive_reopen_and_restore_the_committed_set() {
    let dir = TempDir::new("reopen");
    {
        let mut store = store(&dir);
        store.install(device("1.0.0", HEX_1)).unwrap();
        store
            .update(&pkg(PACKAGE, "1.0.0"), device("2.0.0", HEX_2))
            .unwrap();
        store
            .install(app("org.example.app", HEX_3, &[("2.0.0", HEX_2)]))
            .unwrap();
        store.remove(&pkg(PACKAGE, "1.0.0")).unwrap();
    }
    let opened = FileJournal::open(dir.path()).unwrap();
    assert_eq!(opened.discarded_tail_bytes, 0);
    let recovery = Recovery::from_entries(&opened.entries);
    let active = vec![pkg("org.example.app", "1.0.0"), pkg(PACKAGE, "2.0.0")];
    assert_eq!(recovery.committed.as_deref(), Some(active.as_slice()));
    assert_eq!(recovery.removed, [pkg(PACKAGE, "1.0.0")]);
    assert!(recovery.abandoned.is_empty());

    let restored = PackageStore::restore(
        base(),
        HOST,
        opened.journal,
        &recovery,
        vec![
            device("2.0.0", HEX_2),
            app("org.example.app", HEX_3, &[("2.0.0", HEX_2)]),
        ],
    )
    .unwrap();
    drop(restored);
    // The restore's single checkpoint append is durable too.
    let (again, _) = recover(dir.path());
    assert_eq!(again.committed, recovery.committed);
    assert_eq!(again.published, recovery.published);
}

/// Cuts the log throughout one install transaction: recovery always finds the old or
/// the new complete committed set, never a mix, and reports the half-staged version.
#[test]
fn every_torn_prefix_recovers_a_complete_committed_set() {
    let dir = TempDir::new("torn");
    let mut store = store(&dir);
    store.install(device("1.0.0", HEX_1)).unwrap();
    let before = log(&dir);
    store.install(app("org.example.app", HEX_3, &[])).unwrap();
    drop(store);
    let after = log(&dir);
    let old = vec![pkg(PACKAGE, "1.0.0")];
    let new = vec![pkg("org.example.app", "1.0.0"), pkg(PACKAGE, "1.0.0")];

    // Cut inside every record's frame header, at the start, middle and end of its JSON, and
    // at every boundary: each torn shape a crash can leave.
    let mut cuts = Vec::new();
    let mut start = before.len();
    while start < after.len() {
        let length = u32::from_le_bytes(after[start..start + 4].try_into().unwrap()) as usize;
        let end = start + 12 + length;
        cuts.extend(start..start + 13);
        cuts.extend([start + 12 + length / 2, end - 1]);
        start = end;
    }
    cuts.push(after.len());

    let probe = TempDir::new("cut");
    let mut boundaries = Vec::new();
    let mut seen_new = false;
    for cut in cuts {
        fs::write(probe.path().join(JOURNAL_FILE), &after[..cut]).unwrap();
        let (recovery, discarded) = recover(probe.path());
        let committed = recovery.committed.clone().unwrap();
        assert!(committed == old || committed == new, "cut {cut}");
        // Once the commit record is whole, every longer prefix keeps it.
        if seen_new {
            assert_eq!(committed, new, "cut {cut}");
        }
        seen_new |= committed == new;
        // Open truncated the torn tail to the last complete record.
        let kept = fs::read(probe.path().join(JOURNAL_FILE)).unwrap().len();
        assert_eq!(kept as u64 + discarded, cut as u64, "cut {cut}");
        // Before the commit, the version is abandoned once any of its records is whole.
        if committed == old {
            let expected = if kept > before.len() {
                vec![pkg("org.example.app", "1.0.0")]
            } else {
                vec![]
            };
            assert_eq!(recovery.abandoned, expected, "cut {cut}");
        }
        if discarded == 0 {
            boundaries.push(cut);
        }
    }
    assert!(seen_new);
    // Discovered, Staged, Validated, Granted, Activating, Committed and Active: one record
    // each, so eight complete prefixes counting the starting one.
    assert_eq!(boundaries.len(), 8);
}

#[test]
fn appending_after_a_truncated_tail_keeps_the_log_readable() {
    let dir = TempDir::new("append-after-torn");
    let mut store = store(&dir);
    store.install(device("1.0.0", HEX_1)).unwrap();
    drop(store);
    let mut bytes = log(&dir);
    let complete = bytes.len();
    bytes.extend_from_slice(&[0x40, 0, 0, 0, 1, 2, 3]);
    fs::write(dir.path().join(JOURNAL_FILE), &bytes).unwrap();

    let mut opened = FileJournal::open(dir.path()).unwrap();
    assert_eq!(opened.discarded_tail_bytes, 7);
    let count = opened.entries.len();
    opened
        .journal
        .append(JournalEntry::Transition {
            package: pkg("org.example.next", "1.0.0"),
            state: PackageState::Discovered,
        })
        .unwrap();
    assert!(log(&dir).len() > complete);
    drop(opened.journal);
    let reopened = FileJournal::open(dir.path()).unwrap();
    assert_eq!(reopened.entries.len(), count + 1);
    assert_eq!(reopened.discarded_tail_bytes, 0);
}

#[test]
fn compaction_replaces_the_log_with_one_equivalent_checkpoint() {
    let dir = TempDir::new("compact");
    let mut store = store(&dir);
    store.install(device("1.0.0", HEX_1)).unwrap();
    store.install(device("2.0.0", HEX_2)).unwrap();
    store.revoke(&pkg(PACKAGE, "1.0.0")).unwrap();
    let (before, _) = recover_copied_log(&log(&dir));
    let long = log(&dir).len();

    store.compact_journal().unwrap();
    assert!(FileJournal::open(dir.path()).is_err());
    assert!(log(&dir).len() < long);
    let (compacted, count) = recover_copied_log(&log(&dir));
    assert_eq!(count, 1);
    assert_eq!(compacted, before);

    // The original FileJournal remains usable after replacing the log.
    store.remove(&pkg(PACKAGE, "2.0.0")).unwrap();
    drop(store);
    let (after, _) = recover(dir.path());
    assert_eq!(after.committed.as_deref(), Some(&[] as &[PackageKey]));
    assert_eq!(after.revoked, before.revoked);
}

#[test]
fn writer_lock_is_exclusive_per_directory_and_does_not_touch_recovery_files() {
    let first_dir = TempDir::new("writer-lock-first");
    let other_dir = TempDir::new("writer-lock-other");
    let first = FileJournal::open(first_dir.path()).unwrap();
    let other = FileJournal::open(other_dir.path()).unwrap();

    let stale = first_dir.path().join(JOURNAL_TEMP_FILE);
    fs::write(&stale, b"stale temp must survive refused open").unwrap();
    let error = FileJournal::open(first_dir.path()).unwrap_err();
    assert_eq!(error.reason, "journal already has a writer");
    assert_eq!(
        fs::read(&stale).unwrap(),
        b"stale temp must survive refused open"
    );
    assert_eq!(
        fs::read(first_dir.path().join(JOURNAL_FILE)).unwrap(),
        b"OJOURNAL\x02\x00"
    );

    drop(first);
    let reopened = FileJournal::open(first_dir.path()).unwrap();
    drop((reopened, other));
}

#[test]
fn writer_lock_survives_failed_compaction() {
    let failing_dir = TempDir::new("writer-lock-compact-failure");
    let mut opened = FileJournal::open(failing_dir.path()).unwrap();
    fs::create_dir(failing_dir.path().join(JOURNAL_TEMP_FILE)).unwrap();
    assert!(opened.journal.compact(checkpoint()).is_err());
    fs::remove_dir(failing_dir.path().join(JOURNAL_TEMP_FILE)).unwrap();
    assert_eq!(
        FileJournal::open(failing_dir.path()).unwrap_err().reason,
        "journal already has a writer"
    );
    drop(opened);
    drop(FileJournal::open(failing_dir.path()).unwrap());
}

#[test]
fn writer_lock_survives_successful_compaction() {
    let dir = TempDir::new("writer-lock-compact");
    let mut opened = FileJournal::open(dir.path()).unwrap();
    opened.journal.compact(checkpoint()).unwrap();
    assert_eq!(
        FileJournal::open(dir.path()).unwrap_err().reason,
        "journal already has a writer"
    );
    drop(opened);
    drop(FileJournal::open(dir.path()).unwrap());
}

#[test]
fn writer_lock_is_exclusive_across_processes_and_released_on_process_exit() {
    let dir = TempDir::new("writer-lock-process");
    let opened = FileJournal::open(dir.path()).unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "durable::child_process_journal_lock_probe"])
        .env("ORANGE_JOURNAL_LOCK_PROBE_DIR", dir.path())
        .env("ORANGE_JOURNAL_LOCK_PROBE_MODE", "must-refuse")
        .status()
        .unwrap();
    assert!(status.success());
    drop(opened);

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "durable::child_process_journal_lock_probe"])
        .env("ORANGE_JOURNAL_LOCK_PROBE_DIR", dir.path())
        .env("ORANGE_JOURNAL_LOCK_PROBE_MODE", "acquire-and-exit")
        .status()
        .unwrap();
    assert!(status.success());
    drop(FileJournal::open(dir.path()).unwrap());
}

#[test]
fn child_process_journal_lock_probe() {
    let (Ok(path), Ok(mode)) = (
        std::env::var("ORANGE_JOURNAL_LOCK_PROBE_DIR"),
        std::env::var("ORANGE_JOURNAL_LOCK_PROBE_MODE"),
    ) else {
        return;
    };
    if mode == "must-refuse" {
        assert_eq!(
            FileJournal::open(std::path::Path::new(&path))
                .unwrap_err()
                .reason,
            "journal already has a writer"
        );
    } else {
        assert_eq!(mode, "acquire-and-exit");
        let _opened = FileJournal::open(std::path::Path::new(&path)).unwrap();
        // Verify the OS releases ownership on process termination without Rust Drop.
        std::process::exit(0);
    }
}

fn checkpoint() -> JournalEntry {
    JournalEntry::Committed {
        generation: 1,
        active: vec![],
        published: Default::default(),
        revoked: Default::default(),
        defaults: Default::default(),
    }
}

fn recover_copied_log(bytes: &[u8]) -> (Recovery, usize) {
    let mirror = TempDir::new("recovery-snapshot");
    fs::write(mirror.path().join(JOURNAL_FILE), bytes).unwrap();
    let opened = FileJournal::open(mirror.path()).unwrap();
    (
        Recovery::from_entries(&opened.entries),
        opened.entries.len(),
    )
}

#[test]
fn an_interrupted_compaction_leaves_the_complete_old_log() {
    let dir = TempDir::new("compact-crash");
    let mut store = store(&dir);
    store.install(device("1.0.0", HEX_1)).unwrap();
    drop(store);
    let (before, _) = recover(dir.path());
    // A crash before the rename leaves a partial temporary file beside the log.
    fs::write(dir.path().join(JOURNAL_TEMP_FILE), b"OJOURNAL\x02\x00\xff").unwrap();
    let (after, discarded) = recover(dir.path());
    assert_eq!(after, before);
    assert_eq!(discarded, 0);
    assert!(!dir.path().join(JOURNAL_TEMP_FILE).exists());
}

#[test]
fn other_formats_are_refused_not_migrated() {
    let dir = TempDir::new("format");
    let path = dir.path().join(JOURNAL_FILE);
    for bytes in [
        &b"OJOURNAL\x00\x00"[..],
        b"OJOURNAL\x01\x00",
        b"{\"entries\":[]}",
        b"OJOURNAX\x01\x00",
    ] {
        fs::write(&path, bytes).unwrap();
        let error = FileJournal::open(dir.path()).unwrap_err();
        assert_eq!(error.reason, "unsupported journal format");
    }

    // A whole, CRC-valid record of an unknown shape is not a torn write.
    let payload = br#"{"entry":"checkpoint_v0","active":[]}"#;
    let mut bytes = b"OJOURNAL\x02\x00".to_vec();
    let length = (payload.len() as u32).to_le_bytes();
    bytes.extend_from_slice(&length);
    bytes.extend_from_slice(&crc32(&length).to_le_bytes());
    bytes.extend_from_slice(&crc32(payload).to_le_bytes());
    bytes.extend_from_slice(payload);
    fs::write(&path, &bytes).unwrap();
    let error = FileJournal::open(dir.path()).unwrap_err();
    assert_eq!(error.reason, "journal record has an unsupported shape");
    assert_eq!(fs::read(&path).unwrap(), bytes, "refusal must not truncate");

    // A creation that stopped inside the header starts an empty log.
    fs::write(&path, b"OJOU").unwrap();
    let opened = FileJournal::open(dir.path()).unwrap();
    assert!(opened.entries.is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"OJOURNAL\x02\x00");
}

#[test]
fn corrupt_headers_and_complete_payloads_preserve_the_log_and_later_revocations() {
    let dir = TempDir::new("corruption");
    let mut store = store(&dir);
    store.install(device("1.0.0", HEX_1)).unwrap();
    store.revoke(&pkg(PACKAGE, "1.0.0")).unwrap();
    drop(store);
    let original = log(&dir);
    let (expected, _) = recover(dir.path());
    assert!(!expected.revoked.is_empty());
    let first = 10;
    let first_length = u32::from_le_bytes(original[first..first + 4].try_into().unwrap()) as usize;
    let mut last = first;
    let mut cursor = first;
    while cursor < original.len() {
        last = cursor;
        let length = u32::from_le_bytes(original[cursor..cursor + 4].try_into().unwrap()) as usize;
        cursor += 12 + length;
    }
    // Interior payload, declared length (including a length hiding the suffix),
    // header checksum, payload checksum, and a complete final payload.
    for (offset, mask) in [
        (first + 12 + first_length / 2, 1),
        (first, 1),
        (first + 2, 1),
        (first + 4, 1),
        (first + 8, 1),
        (last + 12, 1),
    ] {
        let mut bytes = original.clone();
        bytes[offset] ^= mask;
        fs::write(dir.path().join(JOURNAL_FILE), &bytes).unwrap();
        let error = FileJournal::open(dir.path()).unwrap_err();
        assert!(
            error.reason.contains("checksum mismatch"),
            "{offset}: {error}"
        );
        assert_eq!(
            log(&dir),
            bytes,
            "corruption must not truncate the identity history"
        );
    }
    let mut oversized = original.clone();
    let length = (orange_packages::MAX_JOURNAL_RECORD_BYTES + 1).to_le_bytes();
    oversized[first..first + 4].copy_from_slice(&length);
    oversized[first + 4..first + 8].copy_from_slice(&crc32(&length).to_le_bytes());
    fs::write(dir.path().join(JOURNAL_FILE), &oversized).unwrap();
    assert_eq!(
        FileJournal::open(dir.path()).unwrap_err().reason,
        "journal record exceeds the size limit"
    );
    assert_eq!(log(&dir), oversized);
    fs::write(dir.path().join(JOURNAL_FILE), &original).unwrap();
    assert_eq!(recover(dir.path()).0, expected);
}

#[test]
fn journal_errors_name_the_operation_not_the_path() {
    let dir = TempDir::new("missing-dir-canary");
    let missing = dir.path().join("absent");
    let error = FileJournal::open(&missing).unwrap_err();
    assert!(!error.reason.contains("canary"), "{}", error.reason);
    assert!(!error.to_string().contains("canary"));
}

/// CRC-32/IEEE, written independently of the crate's implementation.
fn crc32(bytes: &[u8]) -> u32 {
    let table: Vec<u32> = (0..256u32)
        .map(|mut value| {
            for _ in 0..8 {
                value = if value & 1 == 1 {
                    0xedb8_8320 ^ (value >> 1)
                } else {
                    value >> 1
                };
            }
            value
        })
        .collect();
    !bytes.iter().fold(!0u32, |crc, byte| {
        table[((crc ^ u32::from(*byte)) & 0xff) as usize] ^ (crc >> 8)
    })
}
