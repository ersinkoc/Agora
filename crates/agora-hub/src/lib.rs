//! The hub: identity, routing, event store, approvals (docs/architecture.md).

pub mod audit;
pub mod board;
pub mod ca;
pub mod deliver;
pub mod enroll;
pub mod hub;
pub mod notifier;
pub mod notify;
pub mod passkey;
pub mod password;
pub mod projection;
pub mod router;
pub mod scope;
pub mod server;
pub mod session;
pub mod signin;
pub mod ssh;
pub mod ssh_install;
pub mod store;
pub mod tls;
pub mod transport;
pub mod usage;
pub mod users;
pub mod web;
