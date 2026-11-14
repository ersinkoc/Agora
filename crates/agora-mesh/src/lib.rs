//! Agent coordination: messages, channels, task board, leases, handoff, and the MCP server exposed to agents.
//! See docs/specs/mesh-tools.md and ADR-0008.
//!
//! [`broker::Mesh`] is the broker's logic as plain data: no sockets, no clock, no threads. The hub feeds it calls from
//! agents (already bound to an identity by the node's MCP connection) and the current time, and gets back results and
//! audit events. The MCP server that exposes these as tools is a thin layer on top.

pub mod awareness;
pub mod broker;
pub mod mcp;
