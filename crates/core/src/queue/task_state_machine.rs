#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Active,
    Paused,
    RefreshRequired,
    Completed,
    Failed,
}

pub struct TaskStateMachine;

impl TaskStateMachine {
    pub fn transition(from: &TaskState, to: TaskState) -> Result<TaskState, &'static str> {
        match (from, &to) {
            (TaskState::Queued, TaskState::Active) => Ok(to),
            (TaskState::Active, TaskState::Paused) => Ok(to),
            (TaskState::Paused, TaskState::Active) => Ok(to),
            (TaskState::Active, TaskState::RefreshRequired) => Ok(to),
            (TaskState::RefreshRequired, TaskState::Active) => Ok(to),
            (TaskState::Active, TaskState::Completed) => Ok(to),
            (TaskState::Active, TaskState::Failed) => Ok(to),
            _ => Err("invalid transition"),
        }
    }
}
