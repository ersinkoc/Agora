//! Node daemon: supervises terminals and agents, serves files and git, enforces leases, buffers events while the hub is unreachable.

pub mod coalesce;
pub mod control;
pub mod daemon;
pub mod event_buffer;
pub mod files;
pub mod git;
pub mod supervisor;
pub mod terminal;
