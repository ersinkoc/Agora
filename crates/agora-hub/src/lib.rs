//! The hub: identity, routing, event store, approvals (docs/architecture.md).

pub mod ca;
pub mod deliver;
pub mod enroll;
pub mod hub;
pub mod notifier;
pub mod notify;
pub mod passkey;
pub mod projection;
pub mod router;
pub mod server;
pub mod session;
pub mod signin;
pub mod store;
pub mod tls;
pub mod transport;
pub mod usage;
pub mod web;
