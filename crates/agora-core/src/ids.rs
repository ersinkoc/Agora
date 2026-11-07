//! Opaque identifiers. Each is a distinct type so they cannot be mixed up.

macro_rules! id_type {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

id_type!(/// A machine running a node.
    NodeId);
id_type!(/// A project's working copy on one node.
    CheckoutId);
id_type!(/// A git worktree created for an agent.
    WorktreeId);
id_type!(/// A running agent instance.
    AgentId);
id_type!(/// A PTY-backed terminal.
    TerminalId);
id_type!(/// A task on the shared board.
    TaskId);
