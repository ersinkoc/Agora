//! Checkouts and worktrees. Each agent gets its own worktree and branch by default (ADR-0003).

use crate::ids::{CheckoutId, NodeId, WorktreeId};
use crate::project::ProjectId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeError {
    /// Branch name is empty or violates basic git ref rules.
    InvalidBranch,
}

/// A project's working copy on one node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout {
    pub id: CheckoutId,
    pub project: ProjectId,
    pub node: NodeId,
    pub path: String,
}

/// A git worktree inside a checkout, owned by one agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub id: WorktreeId,
    pub checkout: CheckoutId,
    pub node: NodeId,
    pub path: String,
    branch: String,
}

impl Worktree {
    pub fn new(
        id: WorktreeId,
        checkout: &Checkout,
        path: impl Into<String>,
        branch: &str,
    ) -> Result<Self, WorktreeError> {
        if !valid_branch(branch) {
            return Err(WorktreeError::InvalidBranch);
        }
        Ok(Self {
            id,
            checkout: checkout.id.clone(),
            node: checkout.node.clone(),
            path: path.into(),
            branch: branch.to_string(),
        })
    }

    pub fn branch(&self) -> &str {
        &self.branch
    }
}

/// A conservative subset of `git check-ref-format`.
fn valid_branch(b: &str) -> bool {
    !b.is_empty()
        && !b.starts_with(['/', '-', '.'])
        && !b.ends_with(['/', '.'])
        && !b.ends_with(".lock")
        && !b.contains("..")
        && !b.contains("//")
        && !b.contains("@{")
        && !b.chars().any(|c| c.is_control() || " ~^:?*[\\".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checkout() -> Checkout {
        Checkout {
            id: CheckoutId::new("c1"),
            project: ProjectId::new(None, &"a".repeat(40)).unwrap_or_else(|_| unreachable!()),
            node: NodeId::new("n1"),
            path: "/src/x".into(),
        }
    }

    #[test]
    fn worktree_inherits_node_from_checkout() {
        let w = Worktree::new(WorktreeId::new("w"), &checkout(), "/wt/x", "claude/42-x");
        assert_eq!(w.as_ref().map(|w| w.node.as_str()), Ok("n1"));
    }

    #[test]
    fn rejects_bad_branches() {
        for b in [
            "", "-x", "a..b", "a b", "a/", "x.lock", "a//b", "a~1", "@{x",
        ] {
            assert_eq!(
                Worktree::new(WorktreeId::new("w"), &checkout(), "/p", b),
                Err(WorktreeError::InvalidBranch),
                "{b}"
            );
        }
    }
}
