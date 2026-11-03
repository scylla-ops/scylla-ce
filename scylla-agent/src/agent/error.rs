//! The errors of the agent.

/// An error that stops the agent, or one stream of it.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// The server refuses the agent for good: a revoked key, a deleted App, or an App that does
    /// not run jobs. The agent stops.
    #[error("the server refuses the agent: {0}")]
    Refused(String),
    /// The server or the network failed. The agent connects again.
    #[error("the connection failed: {0}")]
    Connection(String),
    /// The file of the agent cannot be read or written.
    #[error("the file of the agent: {0}")]
    File(String),
}

impl AgentError {
    /// Sorts a status of the server: a refusal for good, or a failure to try again.
    pub(crate) fn of_status(status: &tonic::Status) -> Self {
        use tonic::Code;
        let text = format!("{:?}: {}", status.code(), status.message());
        match status.code() {
            Code::Unauthenticated
            | Code::PermissionDenied
            | Code::FailedPrecondition
            | Code::NotFound
            | Code::InvalidArgument => Self::Refused(text),
            _ => Self::Connection(text),
        }
    }
}
