//! Task board entries: open → claimed → done.

use crate::ids::{AgentId, TaskId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Open,
    Claimed(AgentId),
    Done(AgentId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskError {
    /// Already claimed or done.
    NotOpen,
    /// Only the claiming agent may complete or release.
    NotClaimedByAgent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    state: TaskState,
}

impl Task {
    pub fn new(id: TaskId, title: impl Into<String>) -> Self {
        Self {
            id,
            title: title.into(),
            state: TaskState::Open,
        }
    }

    pub fn state(&self) -> &TaskState {
        &self.state
    }

    pub fn claim(&mut self, agent: &AgentId) -> Result<(), TaskError> {
        match self.state {
            TaskState::Open => {
                self.state = TaskState::Claimed(agent.clone());
                Ok(())
            }
            _ => Err(TaskError::NotOpen),
        }
    }

    pub fn complete(&mut self, agent: &AgentId) -> Result<(), TaskError> {
        match &self.state {
            TaskState::Claimed(by) if by == agent => {
                self.state = TaskState::Done(agent.clone());
                Ok(())
            }
            _ => Err(TaskError::NotClaimedByAgent),
        }
    }

    /// Give a claimed task back (also the first half of a handoff).
    pub fn release(&mut self, agent: &AgentId) -> Result<(), TaskError> {
        match &self.state {
            TaskState::Claimed(by) if by == agent => {
                self.state = TaskState::Open;
                Ok(())
            }
            _ => Err(TaskError::NotClaimedByAgent),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle() {
        let (a, b) = (AgentId::new("a"), AgentId::new("b"));
        let mut t = Task::new(TaskId::new("t"), "do it");
        assert_eq!(t.claim(&a), Ok(()));
        assert_eq!(t.claim(&b), Err(TaskError::NotOpen));
        assert_eq!(t.complete(&b), Err(TaskError::NotClaimedByAgent));
        assert_eq!(t.release(&a), Ok(()));
        assert_eq!(t.claim(&b), Ok(()));
        assert_eq!(t.complete(&b), Ok(()));
        assert_eq!(t.state(), &TaskState::Done(b));
        assert_eq!(t.claim(&a), Err(TaskError::NotOpen));
    }
}
