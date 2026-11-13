//! Node daemon: supervises terminals and agents, serves files and git, enforces leases, buffers events while the hub is unreachable.

pub mod approvals;
pub mod chat;
pub mod coalesce;
pub mod control;
pub mod daemon;
pub mod device_trust;
pub mod doctor;
pub mod event_buffer;
pub mod files;
pub mod git;
pub mod grants;
pub mod limits;
pub mod mesh_server;
pub mod sandbox;
pub mod spawn;
pub mod supervisor;
pub mod terminal;
pub mod uplink;
pub mod uplink_client;
pub mod usage_files;

pub mod directory;
