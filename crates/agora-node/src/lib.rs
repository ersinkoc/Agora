//! Node daemon: supervises terminals and agents, serves files and git, enforces leases, buffers events while the hub is unreachable.
#![cfg_attr(not(test), deny(clippy::print_stderr))]

/// One line for the log (the supervisor points this process's error output at the role's log file). Never panics: `eprintln!` does
/// when the write fails, and a full disk then killed the thread that wrote it (the hub link, 2026-11-27: it stopped redialling for
/// good). A line that cannot be written is dropped.
#[macro_export]
macro_rules! log_line {
    ($($arg:tt)*) => {{
        use ::std::io::Write as _;
        let _ = ::std::writeln!(::std::io::stderr(), $($arg)*);
    }};
}

pub mod approvals;
pub mod cert_renew;
pub mod chat;
pub mod coalesce;
pub mod control;
pub mod control_token;
pub mod credentials;
pub mod daemon;
pub mod device_trust;
pub mod doctor;
pub mod event_buffer;
pub mod files;
pub mod git;
pub mod gitconfig;
pub mod grants;
mod handoff_store;
pub mod history;
pub mod holder;
pub mod holder_link;
pub mod holder_note;
pub mod launch_options;
pub mod lease_renewal;
pub mod ledger;
pub mod limits;
pub mod link_renewal;
pub mod link_state;
pub mod manifests;
pub mod mesh_server;
pub mod pairing_client;
pub mod preview;
pub mod preview_relay;
pub mod runner_folders;
pub mod sandbox;
pub mod session_files;
pub mod shared_items;
pub mod spawn;
pub mod supervisor;
pub mod task_dispatch;
pub mod terminal;
pub mod transcript;
pub mod uplink;
pub mod uplink_client;
pub mod usage_files;
pub mod usage_merge;
pub mod usage_quota;
pub mod workspaces;

pub mod directory;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::print_stderr)]
mod log_line_tests {
    /// The maintainer's blackview (2026-11-27): the disk filled, `eprintln!("hub link: …")` failed on the full log file and panicked,
    /// and the hub link's thread died with it, so the node never reconnected after the disk was freed. A log line that cannot be
    /// written is now dropped. Checked in a child process whose error output is `/dev/full` (every write fails with ENOSPC); the same
    /// child with `eprintln!` shows the panic the test guards against.
    #[test]
    fn a_log_line_never_panics_when_the_log_cannot_be_written() {
        const MODE: &str = "AGORA_LOG_LINE_CHILD";
        match std::env::var(MODE).as_deref() {
            Ok("log_line") => {
                let e = "Uplink(Io(\"No space left on device\"))";
                crate::log_line!("hub link: {e}");
                return;
            }
            Ok("eprintln") => {
                let e = "Uplink(Io(\"No space left on device\"))";
                eprintln!("hub link: {e}");
                return;
            }
            _ => {}
        }
        let full = std::path::Path::new("/dev/full");
        if !full.exists() {
            return;
        }
        let run = |mode: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "log_line_tests::a_log_line_never_panics_when_the_log_cannot_be_written",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(MODE, mode)
                .stdout(std::process::Stdio::null())
                .stderr(std::fs::OpenOptions::new().write(true).open(full).unwrap())
                .status()
                .unwrap()
        };
        assert!(
            run("log_line").success(),
            "log_line! panicked on a full log"
        );
        assert!(
            !run("eprintln").success(),
            "the guard no longer sees eprintln!'s panic"
        );
    }
}
