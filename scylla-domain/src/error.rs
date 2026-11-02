#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("project name must not be empty")]
    EmptyProjectName,
    #[error("project name must be at most {max} characters")]
    ProjectNameTooLong { max: usize },
}
