//! Node daemon: supervises terminals and agents, serves files and git, enforces leases, buffers events while the hub is unreachable.

pub mod files;
pub mod git;
pub mod terminal;
