use super::*;

/// A fresh directory for one test; the directory above the folder is the test's own.
fn scratch(name: &str) -> PathBuf {
    let above = std::env::temp_dir().join(format!("orange-sandbox-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&above);
    std::fs::create_dir_all(above.join("folder")).unwrap();
    above
}

fn never() -> bool {
    false
}

#[test]
fn only_plain_relative_paths_stay_inside_the_directory() {
    let root = Path::new("data");
    assert_eq!(
        inside(root, "a/b.txt", false),
        Ok(root.join("a").join("b.txt"))
    );
    assert_eq!(inside(root, "", true), Ok(root.to_path_buf()));
    for bad in [
        "",
        "..",
        "a/../b",
        "./a",
        "/etc/passwd",
        "a//b",
        "a/",
        "C:/Windows",
        "a\\b",
        "nul",
        "dir/COM1.txt",
        "trailing.",
        "space ",
        "a\0b",
        "nul .txt",
        "Aux  .log",
        "COM¹",
        "lpt³.txt",
        "COM0",
        "CONIN$",
        "conout$.txt",
        "a:stream",
    ] {
        assert_eq!(
            inside(root, bad, false),
            Err(FsError::InvalidPath),
            "{bad:?}"
        );
    }
    assert_eq!(
        inside(root, &"a".repeat(MAX_PATH_BYTES + 1), false),
        Err(FsError::InvalidPath)
    );
    // Names that only look like devices are ordinary files.
    for plain in ["console", "COM10", "LPT", "nul_.txt", "comic.txt", "xnul"] {
        assert!(inside(root, plain, false).is_ok(), "{plain:?}");
    }
}

#[test]
fn a_folder_that_does_not_exist_yet_is_empty_and_made_by_the_first_write() {
    let above = scratch("new");
    std::fs::remove_dir(above.join("folder")).unwrap();
    let folder = Folder::new(above.join("folder"), false);
    assert_eq!(folder.list("", 10), Ok(vec![]));
    assert_eq!(folder.read("a.txt", 16), Err(FsError::NotFound));
    assert_eq!(folder.exists("a.txt"), Ok(false));

    folder.write("b/c.bin", &[1, 2, 3]).unwrap();
    folder.write("a.txt", b"hello").unwrap();
    folder.write("a.txt", b"again").unwrap();
    // Replacing leaves no temporary file behind.
    assert_eq!(
        folder.list("", 10),
        Ok(vec!["a.txt".to_owned(), "b/".to_owned()])
    );
    assert_eq!(folder.list("", 1), Err(FsError::TooLarge));
    assert_eq!(folder.read("b/c.bin", 3), Ok(vec![1, 2, 3]));
    assert_eq!(folder.read("b/c.bin", 2), Err(FsError::TooLarge));
    assert_eq!(folder.remove("a.txt"), Ok(()));
    assert_eq!(folder.remove("a.txt"), Err(FsError::NotFound));
    for escape in ["../x", "/x", "b/../../x"] {
        assert_eq!(folder.write(escape, b"x"), Err(FsError::InvalidPath));
        assert_eq!(folder.read(escape, 16), Err(FsError::InvalidPath));
        assert_eq!(folder.remove(escape), Err(FsError::InvalidPath));
    }
    assert!(!above.join("x").exists());

    let locked = Folder::new(above.join("folder"), true);
    assert_eq!(locked.read("b/c.bin", 3), Ok(vec![1, 2, 3]));
    assert_eq!(locked.write("b/c.bin", b"x"), Err(FsError::ReadOnly));
    assert_eq!(locked.rename("b/c.bin", "d"), Err(FsError::ReadOnly));
    assert_eq!(locked.make_directory("e"), Err(FsError::ReadOnly));
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn a_chosen_folder_that_goes_missing_is_reported_and_never_made_again() {
    let above = scratch("missing");
    let root = above.join("folder");
    let folder = Folder::existing(&root, false).unwrap();
    folder.write("a.txt", b"a").unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(folder.read("a.txt", 16), Err(FsError::FolderMissing));
    assert_eq!(folder.list("", 16), Err(FsError::FolderMissing));
    assert_eq!(folder.exists("a.txt"), Err(FsError::FolderMissing));
    assert_eq!(folder.write("a.txt", b"b"), Err(FsError::FolderMissing));
    assert_eq!(folder.make_directory("d"), Err(FsError::FolderMissing));
    assert!(!root.exists());
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn a_replacement_directory_does_not_inherit_the_grant() {
    let above = scratch("replaced-root");
    let root = above.join("folder");
    let original = above.join("original");
    let folder = Folder::existing(&root, false).unwrap();
    folder.write("a", b"original").unwrap();
    std::fs::rename(&root, &original).unwrap();
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a"), b"replacement").unwrap();
    assert_eq!(folder.read("a", 32), Err(FsError::FolderMissing));
    assert_eq!(folder.write("a", b"changed"), Err(FsError::FolderMissing));
    assert_eq!(std::fs::read(root.join("a")).unwrap(), b"replacement");
    drop(folder);
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn listing_limits_include_entries_that_are_skipped() {
    let above = scratch("entry-limit");
    let root = above.join("folder");
    let folder = Folder::new(&root, false);
    std::fs::write(root.join(".orange-skipped.tmp"), b"").unwrap();
    assert_eq!(folder.list("", 0), Err(FsError::TooLarge));
    assert_eq!(folder.list("", 1), Ok(vec![]));
    assert!(matches!(
        folder.tree("", true, 0, &never),
        Err(FsError::TooLarge)
    ));
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn concurrent_appends_enforce_the_same_limit_across_folder_instances() {
    let above = scratch("append");
    let root = above.join("folder");
    let folder = Folder::new(&root, false);
    folder.write("shared", b"").unwrap();
    let barrier = std::sync::Barrier::new(2);
    let outcomes = std::thread::scope(|scope| {
        let append = || {
            let folder = Folder::new(&root, false);
            barrier.wait();
            folder.append("shared", b"1234", 4)
        };
        let first = scope.spawn(append);
        let second = append();
        [first.join().unwrap(), second]
    });
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(outcomes.contains(&Err(FsError::TooLarge)) || outcomes.contains(&Err(FsError::Busy)));
    assert_eq!(folder.read("shared", 4).unwrap(), b"1234");
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn append_reports_a_held_file_lock_as_busy() {
    let above = scratch("append-busy");
    let root = above.join("folder");
    let folder = Folder::new(&root, false);
    folder.write("shared", b"").unwrap();
    let held = std::fs::OpenOptions::new()
        .write(true)
        .open(root.join("shared"))
        .unwrap();
    held.try_lock().unwrap();
    assert_eq!(folder.append("shared", b"a", 1), Err(FsError::Busy));
    drop(held);
    assert_eq!(folder.append("shared", b"a", 1), Ok(()));
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn a_link_planted_at_a_temporary_name_is_never_written_through() {
    let above = scratch("link");
    let inner = above.join("folder");
    let outside = above.join("outside.txt");
    std::fs::write(&outside, b"kept").unwrap();
    // Links at the names a counter alone would give the first writes.
    for count in 0..8 {
        let name = format!(".orange-{}-{count}.tmp", std::process::id());
        std::fs::hard_link(&outside, inner.join(name)).unwrap();
    }
    let folder = Folder::new(&inner, false);
    for _ in 0..8 {
        folder.write("a.txt", b"written").unwrap();
    }
    assert_eq!(std::fs::read(&outside).unwrap(), b"kept");
    assert_eq!(folder.read("a.txt", 16), Ok(b"written".to_vec()));
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn a_failed_replace_keeps_the_previous_file_and_leaves_no_temporary_file() {
    let above = scratch("replace");
    let folder = Folder::new(above.join("folder"), false);
    folder.write("dir/kept.txt", b"kept").unwrap();
    // A directory at the target name is never replaced.
    assert_eq!(folder.write("dir", b"x"), Err(FsError::InvalidPath));
    assert_eq!(
        folder.copy("dir/kept.txt", "dir", 16),
        Err(FsError::InvalidPath)
    );
    // A copy over the limit fails before the target is touched.
    folder.write("big.bin", &[0; 8]).unwrap();
    folder.write("small.bin", b"small").unwrap();
    assert_eq!(
        folder.copy("big.bin", "small.bin", 4),
        Err(FsError::TooLarge)
    );
    assert_eq!(folder.read("small.bin", 16).unwrap(), b"small");
    assert_eq!(folder.read("dir/kept.txt", 16).unwrap(), b"kept");
    let names = std::fs::read_dir(above.join("folder"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert!(
        names.iter().all(|name| !Folder::is_temporary(name)),
        "{names:?}"
    );
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn unicode_names_round_trip_and_a_case_only_rename_keeps_one_file() {
    let above = scratch("names");
    let folder = Folder::new(above.join("folder"), false);
    folder.write("café/ñandú 日本.txt", b"unicode").unwrap();
    assert_eq!(
        folder.list("café", 4),
        Ok(vec!["ñandú 日本.txt".to_owned()])
    );
    assert_eq!(folder.read("café/ñandú 日本.txt", 16).unwrap(), b"unicode");

    folder.write("a.txt", b"case").unwrap();
    folder.rename("a.txt", "A.txt").unwrap();
    assert_eq!(folder.read("A.txt", 16).unwrap(), b"case");
    let files: Vec<_> = folder
        .list("", 8)
        .unwrap()
        .into_iter()
        .filter(|name| name.ends_with(".txt"))
        .collect();
    assert_eq!(files, ["A.txt"]);

    // A folder renamed by case alone is the same folder, so the move is allowed.
    folder.make_directory("dir/inner").unwrap();
    folder.move_tree("dir", "Dir", 8, &never).unwrap();
    assert!(folder.list("", 8).unwrap().contains(&"Dir/".to_owned()));
    assert_eq!(folder.exists("Dir/inner"), Ok(true));
    let _ = std::fs::remove_dir_all(&above);
}

#[test]
fn system_errors_map_to_refusals_a_user_can_act_on() {
    use std::io::{Error, ErrorKind};
    for (kind, expected) in [
        (ErrorKind::NotFound, FsError::NotFound),
        (ErrorKind::PermissionDenied, FsError::PermissionDenied),
        (ErrorKind::ReadOnlyFilesystem, FsError::ReadOnlyLocation),
        (ErrorKind::StorageFull, FsError::NoSpace),
        (ErrorKind::QuotaExceeded, FsError::NoSpace),
        (ErrorKind::CrossesDevices, FsError::CrossDevice),
        (ErrorKind::ResourceBusy, FsError::Busy),
        (ErrorKind::Unsupported, FsError::Unsupported),
        (ErrorKind::Other, FsError::Failed),
    ] {
        assert_eq!(io(&Error::from(kind)), expected, "{kind:?}");
    }
    #[cfg(windows)]
    {
        // Sharing and lock violations, write-protected media, another volume.
        assert_eq!(io(&Error::from_raw_os_error(32)), FsError::Busy);
        assert_eq!(io(&Error::from_raw_os_error(33)), FsError::Busy);
        assert_eq!(io(&Error::from_raw_os_error(19)), FsError::ReadOnlyLocation);
        assert_eq!(io(&Error::from_raw_os_error(17)), FsError::CrossDevice);
        assert_eq!(io(&Error::from_raw_os_error(112)), FsError::NoSpace);
    }
    #[cfg(unix)]
    for (code, expected) in [
        (rustix::io::Errno::XDEV, FsError::CrossDevice),
        (rustix::io::Errno::ROFS, FsError::ReadOnlyLocation),
        (rustix::io::Errno::NOSPC, FsError::NoSpace),
        (rustix::io::Errno::ACCESS, FsError::PermissionDenied),
        (rustix::io::Errno::LOOP, FsError::InvalidPath),
        (rustix::io::Errno::BUSY, FsError::Busy),
    ] {
        assert_eq!(io(&Error::from(code)), expected, "{code:?}");
    }
}

#[test]
fn replace_file_swaps_whole_files_and_keeps_the_previous_one_on_failure() {
    let above = scratch("host-save");
    let target = above.join("document.json");
    replace_file(&target, b"one").unwrap();
    replace_file(&target, b"two").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"two");
    // A directory at the name is not replaced, and nothing is left beside it.
    let blocked = above.join("folder");
    assert!(replace_file(&blocked, b"x").is_err());
    assert!(blocked.is_dir());
    let names: Vec<_> = std::fs::read_dir(&above)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
    assert!(replace_file(Path::new(""), b"x").is_err());
    let long_name = above.join("a".repeat(240));
    replace_file(&long_name, b"long filename").unwrap();
    assert_eq!(std::fs::read(long_name).unwrap(), b"long filename");
    let _ = std::fs::remove_dir_all(&above);
}

/// The held directory, not whatever is at its path now, receives the operation: a
/// directory swapped for a link to outside after it was entered cannot redirect it.
#[cfg(unix)]
#[test]
fn a_directory_swapped_for_a_link_after_it_was_entered_cannot_redirect_the_operation() {
    let above = scratch("swap");
    let root = above.join("folder");
    let outside = above.join("outside");
    std::fs::create_dir_all(root.join("inner")).unwrap();
    std::fs::create_dir(&outside).unwrap();
    let folder = Folder::new(&root, false);
    let inner = folder.walk(&["inner"]).unwrap();
    std::fs::rename(root.join("inner"), root.join("moved")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("inner")).unwrap();
    folder
        .replace_in(&inner, "a.txt", |file| {
            file.write_all(b"held").map_err(|error| io(&error))
        })
        .unwrap();
    inner.create_dir("made").unwrap();
    assert_eq!(std::fs::read(root.join("moved/a.txt")).unwrap(), b"held");
    assert!(root.join("moved/made").is_dir());
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    // A fresh walk refuses the link.
    assert_eq!(folder.write("inner/b.txt", b"x"), Err(FsError::InvalidPath));
    assert_eq!(folder.read("inner/a.txt", 8), Err(FsError::InvalidPath));
    let _ = std::fs::remove_dir_all(&above);
}

/// Links at every position are refused by every operation, for source and
/// destination alike, and nothing outside the folder changes.
#[cfg(unix)]
#[test]
fn links_are_refused_by_every_operation() {
    let above = scratch("links");
    let root = above.join("folder");
    let outside = above.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("kept.txt"), b"kept").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("dir")).unwrap();
    std::os::unix::fs::symlink(outside.join("kept.txt"), root.join("file.txt")).unwrap();
    let folder = Folder::new(&root, false);
    folder.write("real.txt", b"real").unwrap();
    let refused = Err(FsError::InvalidPath);
    assert_eq!(folder.read("file.txt", 8), refused.map(|()| vec![]));
    assert_eq!(folder.read("dir/kept.txt", 8), refused.map(|()| vec![]));
    assert_eq!(folder.write("file.txt", b"x"), refused);
    assert_eq!(folder.write("dir/new.txt", b"x"), refused);
    assert_eq!(folder.append("file.txt", b"x", 64), refused);
    assert_eq!(folder.rename("real.txt", "dir/real.txt"), refused);
    assert_eq!(folder.rename("dir/kept.txt", "taken.txt"), refused);
    assert_eq!(folder.copy("real.txt", "dir/copy.txt", 64), refused);
    assert_eq!(folder.copy("file.txt", "copy.txt", 64), refused);
    assert_eq!(folder.make_directory("dir/sub"), refused);
    assert_eq!(folder.remove("dir/kept.txt"), refused);
    assert_eq!(folder.remove_directory("dir"), Err(FsError::InvalidPath));
    assert_eq!(folder.copy_tree("dir", "copy", 8, 64, &never), refused);
    assert_eq!(folder.remove_tree("dir", 8, &never), refused);
    assert_eq!(
        std::fs::read_dir(&outside).unwrap().count(),
        1,
        "nothing new outside"
    );
    assert_eq!(std::fs::read(outside.join("kept.txt")).unwrap(), b"kept");
    // Listings leave links out, as tree listings do: nothing can be done with them.
    assert_eq!(folder.list("", 8).unwrap(), ["real.txt"]);
    let tree: Vec<_> = folder
        .tree("", true, 8, &never)
        .unwrap()
        .into_iter()
        .map(|entry| entry.path)
        .collect();
    assert_eq!(tree, ["real.txt"]);
    let _ = std::fs::remove_dir_all(&above);
}

#[cfg(unix)]
#[test]
fn permission_denial_is_typed_and_keeps_the_previous_file() {
    use std::os::unix::fs::PermissionsExt;
    let above = scratch("denied");
    let root = above.join("folder");
    let folder = Folder::new(&root, false);
    folder.write("locked/a.txt", b"before").unwrap();
    let locked = root.join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
    // A privileged user is not refused; there is nothing to check then.
    if std::fs::write(locked.join("probe"), b"").is_err() {
        assert_eq!(
            folder.write("locked/a.txt", b"after"),
            Err(FsError::PermissionDenied)
        );
        assert_eq!(
            folder.remove("locked/a.txt"),
            Err(FsError::PermissionDenied)
        );
        assert_eq!(folder.read("locked/a.txt", 16).unwrap(), b"before");
    }
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    let _ = std::fs::remove_dir_all(&above);
}

/// A name that is not UTF-8 cannot be addressed: listings leave it out and tree
/// changes refuse the tree rather than change part of it. macOS refuses such names.
#[cfg(target_os = "linux")]
#[test]
fn a_name_that_is_not_utf8_is_skipped_by_listings_and_refused_by_tree_changes() {
    use std::os::unix::ffi::OsStrExt;
    let above = scratch("bytes");
    let root = above.join("folder");
    std::fs::create_dir(root.join("tree")).unwrap();
    let odd = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
    std::fs::write(root.join("tree").join(odd), b"latin-1").unwrap();
    std::fs::write(root.join("tree/plain.txt"), b"plain").unwrap();
    let folder = Folder::new(&root, false);
    assert_eq!(folder.list("tree", 8).unwrap(), ["plain.txt"]);
    let tree: Vec<_> = folder
        .tree("tree", false, 8, &never)
        .unwrap()
        .into_iter()
        .map(|entry| entry.path)
        .collect();
    assert_eq!(tree, ["plain.txt"]);
    assert_eq!(
        folder.remove_tree("tree", 8, &never),
        Err(FsError::InvalidPath)
    );
    assert_eq!(
        folder.copy_tree("tree", "copy", 8, 64, &never),
        Err(FsError::InvalidPath)
    );
    assert!(root.join("tree").join(odd).exists());
    let _ = std::fs::remove_dir_all(&above);
}

/// A held directory cannot be renamed away and replaced while an operation runs.
#[cfg(windows)]
#[test]
fn a_held_directory_cannot_be_renamed_until_released() {
    let above = scratch("hold");
    let root = above.join("folder");
    std::fs::create_dir(root.join("inner")).unwrap();
    let folder = Folder::new(&root, false);
    let inner = folder.walk(&["inner"]).unwrap();
    assert!(std::fs::rename(root.join("inner"), root.join("moved")).is_err());
    assert!(std::fs::rename(&root, above.join("moved-root")).is_err());
    // Files inside a held directory are still created, replaced and removed.
    folder.write("inner/a.txt", b"a").unwrap();
    folder.rename("inner/a.txt", "inner/b.txt").unwrap();
    folder.remove("inner/b.txt").unwrap();
    drop(inner);
    std::fs::rename(root.join("inner"), root.join("moved")).unwrap();
    let _ = std::fs::remove_dir_all(&above);
}

/// Junctions need no privilege to create, so they stand for every reparse point.
#[cfg(windows)]
#[test]
fn junctions_are_refused_by_every_operation() {
    let above = scratch("junction");
    let root = above.join("folder");
    let outside = above.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("kept.txt"), b"kept").unwrap();
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(root.join("dir"))
        .arg(&outside)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let folder = Folder::new(&root, false);
    folder.write("real.txt", b"real").unwrap();
    let refused = Err(FsError::InvalidPath);
    assert_eq!(folder.read("dir/kept.txt", 8), refused.map(|()| vec![]));
    assert_eq!(folder.write("dir/new.txt", b"x"), refused);
    assert_eq!(folder.append("dir/new.txt", b"x", 64), refused);
    assert_eq!(folder.rename("real.txt", "dir/real.txt"), refused);
    assert_eq!(folder.rename("dir/kept.txt", "taken.txt"), refused);
    assert_eq!(folder.copy("real.txt", "dir/copy.txt", 64), refused);
    assert_eq!(folder.make_directory("dir/sub"), refused);
    assert_eq!(folder.remove("dir/kept.txt"), refused);
    assert_eq!(folder.remove_directory("dir"), refused);
    assert_eq!(folder.copy_tree("dir", "copy", 8, 64, &never), refused);
    assert_eq!(folder.move_tree("dir", "moved", 8, &never), refused);
    assert_eq!(folder.remove_tree("dir", 8, &never), refused);
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    assert_eq!(std::fs::read(outside.join("kept.txt")).unwrap(), b"kept");
    let tree: Vec<_> = folder
        .tree("", true, 8, &never)
        .unwrap()
        .into_iter()
        .map(|entry| entry.path)
        .collect();
    assert_eq!(tree, ["real.txt"]);
    std::fs::remove_dir(root.join("dir")).unwrap();
    let _ = std::fs::remove_dir_all(&above);
}

/// A file another program holds open without sharing is reported as busy, and a
/// replace that cannot happen leaves the previous file and no temporary file.
#[cfg(windows)]
#[test]
fn a_file_held_without_sharing_is_busy_and_survives_a_refused_replace() {
    use std::os::windows::fs::OpenOptionsExt;
    let above = scratch("busy");
    let root = above.join("folder");
    let folder = Folder::new(&root, false);
    folder.write("held.txt", b"before").unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(root.join("held.txt"))
        .unwrap();
    assert_eq!(folder.read("held.txt", 16), Err(FsError::Busy));
    assert_eq!(folder.append("held.txt", b"x", 64), Err(FsError::Busy));
    let replaced = folder.write("held.txt", b"after");
    assert!(
        matches!(replaced, Err(FsError::Busy | FsError::PermissionDenied)),
        "{replaced:?}"
    );
    let removed = folder.remove("held.txt");
    assert!(
        matches!(removed, Err(FsError::Busy | FsError::PermissionDenied)),
        "{removed:?}"
    );
    drop(held);
    assert_eq!(folder.read("held.txt", 16).unwrap(), b"before");
    assert_eq!(folder.list("", 8).unwrap(), ["held.txt"]);
    let _ = std::fs::remove_dir_all(&above);
}
