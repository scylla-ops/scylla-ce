//! The errors of the agent.

use tonic_types::StatusExt;

/// The reason of the refusal of a disabled App.
const APP_DISABLED: &str = "APP_DISABLED";

/// An error that stops the agent, or one stream of it.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// The server refuses the agent for good: a revoked key, a deleted App, or an App that does
    /// not run jobs. The agent stops.
    #[error("the server refuses the agent: {0}")]
    Refused(String),
    /// An admin disabled the App. The agent stops its jobs, and connects again until an admin
    /// enables the App.
    #[error("the app is disabled: {0}")]
    Disabled(String),
    /// The server or the network failed. The agent connects again.
    #[error("the connection failed: {0}")]
    Connection(String),
    /// The file of the agent cannot be read or written.
    #[error("the file of the agent: {0}")]
    File(String),
    /// The system refuses a need of the process: its runtime, its log file, or the service
    /// control manager.
    #[error("the system: {0}")]
    System(String),
}

impl AgentError {
    /// Sorts a status of the server: a refusal for good, a disabled App, or a failure to try
    /// again.
    pub(crate) fn of_status(status: &tonic::Status) -> Self {
        use tonic::Code;
        let text = format!("{:?}: {}", status.code(), status.message());
        let reason = status.get_details_error_info().map(|info| info.reason);
        match status.code() {
            Code::FailedPrecondition if reason.as_deref() == Some(APP_DISABLED) => {
                Self::Disabled(text)
            }
            Code::Unauthenticated
            | Code::PermissionDenied
            | Code::FailedPrecondition
            | Code::NotFound
            | Code::InvalidArgument => Self::Refused(text),
            _ => Self::Connection(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tonic::{Code, Status};
    use tonic_types::{ErrorDetails, StatusExt};

    use super::{AgentError, APP_DISABLED};

    /// Returns a status with an `ErrorInfo` of a reason.
    fn refusal(code: Code, reason: &str) -> Status {
        let details = ErrorDetails::with_error_info(reason, "scylla", HashMap::new());
        Status::with_error_details(code, "refused", details)
    }

    /// A disabled App waits, another refusal stops the agent, and a failure tries again.
    #[test]
    fn a_disabled_app_waits_and_a_refusal_stops() {
        // given
        let disabled = refusal(Code::FailedPrecondition, APP_DISABLED);
        let not_a_runner = refusal(Code::FailedPrecondition, "NOT_A_RUNNER");
        let unavailable = Status::unavailable("down");

        // when
        let sorted =
            [disabled, not_a_runner, unavailable].map(|status| AgentError::of_status(&status));

        // then
        assert!(
            matches!(sorted[0], AgentError::Disabled(_)),
            "{:?}",
            sorted[0]
        );
        assert!(
            matches!(sorted[1], AgentError::Refused(_)),
            "{:?}",
            sorted[1]
        );
        assert!(
            matches!(sorted[2], AgentError::Connection(_)),
            "{:?}",
            sorted[2]
        );
    }
}
