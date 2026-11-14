//! `open_append_private` (the supervisor's `--log`, review of #550): nothing planted in the Agora folder redirects what is written or
//! has its permissions changed. Links that need a privilege to make (Windows symbolic links) are skipped where they cannot be made.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("agora-plog-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(unix)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(windows)]
fn symlink_dir(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(target, link).is_ok()
}
#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(target, link).is_ok()
}
#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_file(target, link).is_ok()
}

#[cfg(unix)]
fn mode(p: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).unwrap().permissions().mode() & 0o777
}

#[test]
fn a_new_log_is_private_and_an_existing_one_is_appended_to() {
    let d = tmp("plain");
    let log = d.join("node.log");
    let mut f = agora_platform::open_append_private(&log).unwrap();
    writeln!(f, "one").unwrap();
    drop(f);
    #[cfg(unix)]
    assert_eq!(mode(&log), 0o600);
    let mut f = agora_platform::open_append_private(&log).unwrap();
    writeln!(f, "two").unwrap();
    drop(f);
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "one\ntwo\n");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_symlinked_logs_folder_is_refused() {
    let d = tmp("dir");
    let elsewhere = d.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("node.log"), "keep\n").unwrap();
    let logs = d.join("logs");
    if !symlink_dir(&elsewhere, &logs) {
        return;
    }
    let err = agora_platform::open_append_private(&logs.join("node.log")).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert_eq!(
        std::fs::read_to_string(elsewhere.join("node.log")).unwrap(),
        "keep\n"
    );
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_hard_linked_log_is_refused_and_left_as_it_was() {
    let d = tmp("hard");
    let target = d.join("precious.txt");
    std::fs::write(&target, "keep\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    let log = d.join("node.log");
    std::fs::hard_link(&target, &log).unwrap();
    let err = agora_platform::open_append_private(&log).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep\n");
    #[cfg(unix)]
    assert_eq!(mode(&target), 0o644);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn a_symlinked_log_is_refused_and_its_target_untouched() {
    let d = tmp("link");
    let target = d.join("precious.txt");
    std::fs::write(&target, "keep\n").unwrap();
    let log = d.join("node.log");
    if !symlink_file(&target, &log) {
        return;
    }
    let err = agora_platform::open_append_private(&log).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep\n");
    #[cfg(unix)]
    assert_ne!(mode(&target) & 0o100, 0o100, "the target became executable");
    let _ = std::fs::remove_dir_all(d);
}
