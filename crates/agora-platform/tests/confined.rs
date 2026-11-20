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
