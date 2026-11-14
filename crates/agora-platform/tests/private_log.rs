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

/// An existing log others could read (one name, this user's) is made the user's only through its own descriptor.
#[cfg(unix)]
#[test]
fn an_existing_readable_log_is_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let d = tmp("loose");
    let log = d.join("node.log");
    std::fs::write(&log, "old\n").unwrap();
    std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut f = agora_platform::open_append_private(&log).unwrap();
    writeln!(f, "new").unwrap();
    drop(f);
    assert_eq!(mode(&log), 0o600);
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "old\nnew\n");
    let _ = std::fs::remove_dir_all(d);
}

/// A FIFO under the log's name (Linux's review of #550): refused at once, never waiting for a reader.
#[cfg(unix)]
#[test]
fn a_fifo_log_is_refused_without_waiting() {
    let d = tmp("fifo");
    let log = d.join("node.log");
    assert!(std::process::Command::new("mkfifo")
        .arg(&log)
        .status()
        .unwrap()
        .success());
    let (tx, rx) = std::sync::mpsc::channel();
    let path = log.clone();
    std::thread::spawn(move || {
        let _ = tx.send(agora_platform::open_append_private(&path).map(|_| ()));
    });
    let got = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("open_append_private hung on a FIFO");
    assert_eq!(got.unwrap_err().kind(), ErrorKind::InvalidInput);
    let _ = std::fs::remove_dir_all(d);
}

/// The guard of a role's lock (Linux's review of #551): private from its creation, never a link or a second name, never truncated,
/// and its folder private too.
#[test]
fn a_lock_file_is_private_and_never_a_link() {
    let d = tmp("lock");
    let run = d.join("run");
    agora_platform::ensure_private_dir(&run).unwrap();
    #[cfg(unix)]
    assert_eq!(mode(&run), 0o700);
    let guard = run.join("node.lock.guard");
    let f = agora_platform::open_lock_private(&guard).unwrap();
    f.lock().unwrap();
    drop(f);
    #[cfg(unix)]
    assert_eq!(mode(&guard), 0o600);
    // Kept as it is: never truncated.
    std::fs::write(&guard, "x").unwrap();
    drop(agora_platform::open_lock_private(&guard).unwrap());
    assert_eq!(std::fs::read_to_string(&guard).unwrap(), "x");
    // A second name for another file is refused.
    let other = d.join("other.txt");
    std::fs::write(&other, "keep").unwrap();
    let hard = run.join("hub.lock.guard");
    std::fs::hard_link(&other, &hard).unwrap();
    assert_eq!(
        agora_platform::open_lock_private(&hard).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
    // A link to another file is refused, and a run folder that is a link is refused.
    let link = run.join("x.lock.guard");
    if symlink_file(&other, &link) {
        assert_eq!(
            agora_platform::open_lock_private(&link).unwrap_err().kind(),
            ErrorKind::InvalidInput
        );
    }
    let linked_run = d.join("linked-run");
    if symlink_dir(&run, &linked_run) {
        assert_eq!(
            agora_platform::ensure_private_dir(&linked_run)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidInput
        );
    }
    assert_eq!(std::fs::read_to_string(&other).unwrap(), "keep");
    let _ = std::fs::remove_dir_all(d);
}
