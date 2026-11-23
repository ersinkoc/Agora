//! `ConfinedDir`: folder-relative create, replace and refusal of links (issue #847). The same tests run on every OS; where a link
//! needs a privilege or a call that one OS lacks, [`agora_platform::link_dir`] is the folder link both can make.
#![allow(clippy::unwrap_used)]

use agora_platform::{not_confined, ConfinedDir, EntryKind};
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

fn tmp() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "agora-confined-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    fs::canonicalize(&d).unwrap()
}

#[test]
fn creates_folders_and_a_file_then_renames_it_over_another() {
    let d = tmp();
    let root = ConfinedDir::open_root(&d).unwrap();
    assert_eq!(root.real_path().unwrap(), d);
    let sub = root.subdir(OsStr::new("a"), true).unwrap();
    assert!(root.subdir(OsStr::new("missing"), false).is_err());
    assert_eq!(sub.entry(OsStr::new("f")).unwrap(), None);

    let mut first = sub.create_new(OsStr::new("t1"), 0o644).unwrap();
    std::io::Write::write_all(&mut first, b"one").unwrap();
    sub.rename_over(&first, OsStr::new("t1"), OsStr::new("f"))
        .unwrap();
    drop(first);
    let mut second = sub.create_new(OsStr::new("t2"), 0o644).unwrap();
    std::io::Write::write_all(&mut second, b"two").unwrap();
    // A second create over an existing name is refused; the rename replaces.
    assert!(sub.create_new(OsStr::new("t2"), 0o644).is_err());
    sub.rename_over(&second, OsStr::new("t2"), OsStr::new("f"))
        .unwrap();
    drop(second);
    assert_eq!(fs::read(d.join("a/f")).unwrap(), b"two");
    assert_eq!(
        sub.entry(OsStr::new("f")).unwrap().unwrap().kind,
        EntryKind::File
    );
    sub.remove_file(OsStr::new("f")).unwrap();
    assert_eq!(sub.entry(OsStr::new("f")).unwrap(), None);
}

#[test]
fn names_that_are_not_one_plain_component_are_refused() {
    let d = tmp();
    let root = ConfinedDir::open_root(&d).unwrap();
    for bad in ["..", ".", "a/b", "", "a\\b"] {
        let e = root.subdir(OsStr::new(bad), true).unwrap_err();
        assert!(not_confined(&e), "{bad:?}");
        assert!(root.create_new(OsStr::new(bad), 0o600).is_err(), "{bad:?}");
    }
}

#[test]
fn a_folder_link_is_refused_never_followed() {
    let d = tmp();
    let outside = tmp();
    if agora_platform::link_dir(&outside, &d.join("l")).is_err() {
        return; // this machine cannot make a folder link
    }
    let root = ConfinedDir::open_root(&d).unwrap();
    for create in [false, true] {
        let e = root.subdir(OsStr::new("l"), create).unwrap_err();
        assert!(not_confined(&e), "create={create}: {e}");
    }
    assert_eq!(
        root.entry(OsStr::new("l")).unwrap().unwrap().kind,
        EntryKind::Link
    );
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn a_file_link_is_not_followed_by_create_or_rename() {
    let d = tmp();
    let outside = tmp().join("target.txt");
    fs::write(&outside, "keep").unwrap();
    std::os::unix::fs::symlink(&outside, d.join("l")).unwrap();
    let root = ConfinedDir::open_root(&d).unwrap();
    assert_eq!(
        root.entry(OsStr::new("l")).unwrap().unwrap().kind,
        EntryKind::Link
    );
    assert!(root.create_new(OsStr::new("l"), 0o600).is_err());
    let file = root.create_new(OsStr::new("t"), 0o600).unwrap();
    root.rename_over(&file, OsStr::new("t"), OsStr::new("l"))
        .unwrap();
    assert_eq!(fs::read(&outside).unwrap(), b"keep");
    assert!(fs::symlink_metadata(d.join("l")).unwrap().is_file());
}

#[cfg(unix)]
#[test]
fn a_fifo_is_described_as_other_and_a_regular_file_in_the_way_is_not_a_folder() {
    let d = tmp();
    assert!(std::process::Command::new("mkfifo")
        .arg(d.join("p"))
        .status()
        .unwrap()
        .success());
    fs::write(d.join("f"), "x").unwrap();
    let root = ConfinedDir::open_root(&d).unwrap();
    assert_eq!(
        root.entry(OsStr::new("p")).unwrap().unwrap().kind,
        EntryKind::Other
    );
    assert!(not_confined(
        &root.subdir(OsStr::new("f"), true).unwrap_err()
    ));
}

/// Windows spellings that name a stream, a device or a short alias are refused before any call.
#[cfg(windows)]
#[test]
fn windows_second_meanings_are_refused_on_the_handle_api() {
    let d = tmp();
    let root = ConfinedDir::open_root(&d).unwrap();
    for bad in [".env:stream", "NUL", "ID_RSA~1", "x."] {
        assert!(root.create_new(OsStr::new(bad), 0).is_err(), "{bad}");
    }
}

/// macOS: a file system that folds case still reports the real spelling of the held folder.
#[cfg(target_os = "macos")]
#[test]
fn the_real_path_of_a_held_folder_is_the_kernels_spelling() {
    let d = tmp();
    fs::create_dir(d.join("Sub")).unwrap();
    let root = ConfinedDir::open_root(&d).unwrap();
    let sub = root.subdir(OsStr::new("Sub"), false).unwrap();
    assert_eq!(sub.real_path().unwrap(), d.join("Sub"));
}

/// #1000: a file is read through the held folder; nothing there is `NotFound`, anything but a regular file is refused, not followed.
#[test]
fn a_file_is_read_through_the_handle_and_anything_else_is_refused() {
    let d = tmp();
    fs::write(d.join("f"), "one").unwrap();
    fs::create_dir(d.join("sub")).unwrap();
    let root = ConfinedDir::open_root(&d).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut root.open_read(OsStr::new("f")).unwrap(), &mut text)
        .unwrap();
    assert_eq!(text, "one");
    assert_eq!(
        root.open_read(OsStr::new("missing")).unwrap_err().kind(),
        std::io::ErrorKind::NotFound
    );
    let e = root.open_read(OsStr::new("sub")).unwrap_err();
    assert_ne!(e.kind(), std::io::ErrorKind::NotFound, "{e}");
    for bad in ["..", "a/b", ""] {
        assert!(
            not_confined(&root.open_read(OsStr::new(bad)).unwrap_err()),
            "{bad:?}"
        );
    }
    let outside = tmp();
    if agora_platform::link_dir(&outside, &d.join("l")).is_ok() {
        let e = root.open_read(OsStr::new("l")).unwrap_err();
        assert_ne!(e.kind(), std::io::ErrorKind::NotFound, "{e}");
    }
}

/// #1000: a link at the name, to a file or to nothing, is refused (never `NotFound`), and a FIFO is refused without waiting.
#[cfg(unix)]
#[test]
fn a_file_link_or_a_fifo_is_not_read() {
    let d = tmp();
    let outside = tmp().join("target.txt");
    fs::write(&outside, "secret").unwrap();
    std::os::unix::fs::symlink(&outside, d.join("l")).unwrap();
    std::os::unix::fs::symlink(d.join("nothing"), d.join("dangling")).unwrap();
    assert!(std::process::Command::new("mkfifo")
        .arg(d.join("p"))
        .status()
        .unwrap()
        .success());
    let root = ConfinedDir::open_root(&d).unwrap();
    for name in ["l", "dangling", "p"] {
        let e = root.open_read(OsStr::new(name)).unwrap_err();
        assert!(not_confined(&e), "{name}: {e}");
    }
}

/// #1000: the lock file is opened in the folder the handle holds, even when that folder was renamed away and another put at its
/// path; it is private and opened again as the same file.
#[test]
fn the_lock_is_opened_in_the_held_folder_not_one_put_at_its_path() {
    let base = tmp();
    let d = base.join("agents");
    fs::create_dir(&d).unwrap();
    let root = ConfinedDir::open_root(&d).unwrap();
    let aside = base.join("held");
    if fs::rename(&d, &aside).is_err() {
        return; // this system does not rename a folder held open
    }
    fs::create_dir(&d).unwrap();
    let lock = root.open_lock(OsStr::new(".lock")).unwrap();
    assert!(lock.metadata().unwrap().is_file());
    drop(lock);
    assert!(aside.join(".lock").is_file());
    assert!(!d.join(".lock").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(aside.join(".lock"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "{mode:o}");
    }
    // An existing lock file is opened again (never truncated).
    fs::write(aside.join(".lock"), "123").unwrap();
    let mut again = root.open_lock(OsStr::new(".lock")).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut again, &mut text).unwrap();
    assert_eq!(text, "123");
    assert!(not_confined(
        &root.open_lock(OsStr::new("a/b")).unwrap_err()
    ));
}

/// #1000: a link at the lock's name is refused, never followed.
#[cfg(unix)]
#[test]
fn a_link_at_the_lock_name_is_refused() {
    let d = tmp();
    let outside = tmp().join("target");
    fs::write(&outside, "keep").unwrap();
    std::os::unix::fs::symlink(&outside, d.join(".lock")).unwrap();
    let root = ConfinedDir::open_root(&d).unwrap();
    let e = root.open_lock(OsStr::new(".lock")).unwrap_err();
    assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{e}");
    assert_eq!(fs::read(&outside).unwrap(), b"keep");
}
