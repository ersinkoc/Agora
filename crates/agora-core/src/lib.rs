//! Pure domain model: projects, agents, terminals, worktrees, tasks, leases. No I/O, no async, no platform code.
//!
//! Time is plain milliseconds since the Unix epoch (`u64`) passed in by the caller, so everything here is
//! deterministic and testable without clocks.

pub mod agent;
pub mod events;
pub mod ids;
pub mod lease;
pub mod project;
pub mod restart;
pub mod secrets;
pub mod task;
pub mod text;
pub mod worktree;

pub use agent::{Agent, AgentState, IntegrationLevel, StateSource, Terminal, TerminalPurpose};
pub use events::{Consumer, EventLog, ProducerId, Receive};
pub use ids::{AgentId, CheckoutId, NodeId, TaskId, TerminalId, WorktreeId};
pub use lease::{Lease, LeaseError, LeaseTable};
pub use project::{
    fold_project_id, normalize_remote, same_project, ProjectError, ProjectId,
    CASE_INSENSITIVE_HOSTS,
};
pub use task::{Task, TaskError, TaskState};
pub use worktree::{Checkout, Worktree, WorktreeError};
