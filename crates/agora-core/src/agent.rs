//! Agents and terminals.

use crate::ids::{AgentId, NodeId, TerminalId, WorktreeId};
use crate::project::ProjectId;

/// Reported or detected state of an agent. `Done` means finished and not yet seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    Working,
    Blocked,
    Idle,
    Done,
    Unknown,
}

/// How deeply an agent kind is integrated (ADR-0004).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntegrationLevel {
    L0Generic,
    L1Manifest,
    L2Acp,
    L3Adapter,
}

/// Where a state came from. Reported state outranks screen-derived state (architecture §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSource {
    Screen,
    Reported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    pub id: AgentId,
    pub project: ProjectId,
    pub node: NodeId,
    /// `None` for non-git directories (L0 terminal without per-agent change tracking).
    pub worktree: Option<WorktreeId>,
    pub kind: String,
    pub level: IntegrationLevel,
    state: AgentState,
    state_source: StateSource,
    /// The agent's own session id, used for resume.
    pub session: Option<String>,
}

impl Agent {
    pub fn new(
        id: AgentId,
        project: ProjectId,
        node: NodeId,
        worktree: Option<WorktreeId>,
        kind: impl Into<String>,
        level: IntegrationLevel,
    ) -> Self {
        Self {
            id,
            project,
            node,
            worktree,
            kind: kind.into(),
            level,
            state: AgentState::Unknown,
            state_source: StateSource::Screen,
            session: None,
        }
    }

    pub fn state(&self) -> AgentState {
        self.state
    }

    /// Apply an observed state. A screen-derived state is ignored while a reported state is in effect;
    /// a reported state always wins. Returns true if the state changed.
    pub fn observe(&mut self, state: AgentState, source: StateSource) -> bool {
        if source == StateSource::Screen && self.state_source == StateSource::Reported {
            return false;
        }
        let changed = self.state != state;
        self.state = state;
        self.state_source = source;
        changed
    }

    /// A new process now backs this agent (restart or resume): forget the old process's state and let
    /// screen detection speak again. The session id is kept. Returns true if the state changed.
    pub fn process_restarted(&mut self) -> bool {
        let changed = self.state != AgentState::Unknown;
        self.state = AgentState::Unknown;
        self.state_source = StateSource::Screen;
        changed
    }

    /// The user looked at a finished agent: `Done` becomes `Idle`.
    pub fn mark_seen(&mut self) {
        if self.state == AgentState::Done {
            self.state = AgentState::Idle;
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalPurpose {
    Agent,
    Shell,
    Service,
}

/// A PTY-backed terminal on a node. Runtime handles live elsewhere (agora-node).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    pub id: TerminalId,
    pub node: NodeId,
    pub purpose: TerminalPurpose,
    /// Set when `purpose` is `Agent`.
    pub agent: Option<AgentId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent() -> Agent {
        Agent::new(
            AgentId::new("a"),
            ProjectId::new(None, &"b".repeat(40)).unwrap_or_else(|_| unreachable!()),
            NodeId::new("n"),
            None,
            "claude-code",
            IntegrationLevel::L1Manifest,
        )
    }

    #[test]
    fn starts_unknown() {
        assert_eq!(agent().state(), AgentState::Unknown);
    }

    #[test]
    fn reported_beats_screen() {
        let mut a = agent();
        assert!(a.observe(AgentState::Working, StateSource::Screen));
        assert!(a.observe(AgentState::Blocked, StateSource::Reported));
        assert!(!a.observe(AgentState::Idle, StateSource::Screen));
        assert_eq!(a.state(), AgentState::Blocked);
        assert!(a.observe(AgentState::Idle, StateSource::Reported));
    }

    #[test]
    fn a_new_process_lets_screen_detection_speak_again() {
        let mut a = agent();
        a.observe(AgentState::Done, StateSource::Reported);
        assert!(!a.observe(AgentState::Working, StateSource::Screen));
        assert!(a.process_restarted());
        assert_eq!(a.state(), AgentState::Unknown);
        assert!(a.observe(AgentState::Working, StateSource::Screen));
        assert!(!a.process_restarted() || a.state() == AgentState::Unknown);
    }

    #[test]
    fn done_becomes_idle_when_seen() {
        let mut a = agent();
        a.observe(AgentState::Done, StateSource::Reported);
        a.mark_seen();
        assert_eq!(a.state(), AgentState::Idle);
        a.observe(AgentState::Working, StateSource::Reported);
        a.mark_seen();
        assert_eq!(a.state(), AgentState::Working);
    }

    #[test]
    fn levels_are_ordered() {
        assert!(IntegrationLevel::L2Acp > IntegrationLevel::L1Manifest);
    }
}
