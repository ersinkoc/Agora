//! The hub: identity, routing, event store, approvals (docs/architecture.md).
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

pub mod audit;
pub mod board;
pub mod ca;
pub mod deliver;
pub mod devices;
pub mod discovery;
pub mod dispatch;
pub mod enroll;
pub mod forgets;
mod handoff;
pub mod helpers;
pub mod hub;
pub mod leaf;
pub mod metrics;
pub mod metrics_token;
pub mod names;
pub mod notifier;
pub mod notify;
pub mod pairing;
pub mod pairing_net;
pub mod passkey;
pub mod password;
pub mod plugin_catalog;
pub mod plugin_package;
pub mod plugins;
pub mod project_names;
pub mod projection;
pub mod proxy;
pub mod router;
pub mod scope;
pub mod server;
pub mod session;
pub mod signin;
pub mod ssh;
pub mod ssh_install;
pub mod sshsig;
pub mod store;
pub mod theme_catalog;
pub mod tls;
pub mod transport;
pub mod usage;
pub mod user_settings;
pub mod users;
pub mod web;
