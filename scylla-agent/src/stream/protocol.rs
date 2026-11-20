//! The agent protocol: the level of this agent, its check against the protocols of the server,
//! and the command that updates the agent.

use scylla_proto::apps::v1::AgentProtocolRange;
use tonic_types::StatusExt;

use crate::agent::AgentError;

/// The agent protocol of this agent: it grows when the agent learns a new message or a new
/// feature. The protocol 2 has the checkout of the code.
pub const AGENT_PROTOCOL: u32 = 2;

/// The reason of the refusal of an agent older than the minimum protocol of the server.
const AGENT_TOO_OLD: &str = "AGENT_TOO_OLD";

/// Sorts a status of the server. The refusal of an old agent tells the command that updates it.
pub(super) fn refusal(status: &tonic::Status, url: &str) -> AgentError {
    let reason = status.get_details_error_info().map(|info| info.reason);
    if reason.as_deref() != Some(AGENT_TOO_OLD) {
        return AgentError::of_status(status);
    }
    AgentError::Refused(format!(
        "this agent has the protocol {AGENT_PROTOCOL}, and the server needs a newer agent ({}). \
         Update the agent on this machine: {}",
        status.message(),
        update_command(url)
    ))
}

/// Logs a warning with the command that updates the agent when the server has a newer protocol.
/// The agent then gets only the jobs that it can run.
pub(super) fn check(range: Option<&AgentProtocolRange>, url: &str) {
    let Some(current) = range.map(|range| range.current) else {
        return;
    };
    if current > AGENT_PROTOCOL {
        tracing::warn!(
            protocol = AGENT_PROTOCOL,
            server = current,
            update = %update_command(url),
            "the agent is older than the server, and it gets only the jobs that it can run: \
             update it"
        );
    }
}

/// Returns the command that updates the agent of this machine with the script of the server.
pub(crate) fn update_command(url: &str) -> String {
    let url = url.trim_end_matches('/');
    if cfg!(windows) {
        format!("& ([scriptblock]::Create((irm {url}/install.ps1))) -Update")
    } else {
        format!("curl -fsSL {url}/install.sh | sh -s -- --update")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tonic::{Code, Status};
    use tonic_types::ErrorDetails;

    use super::*;

    /// The refusal of an old agent stops it for good, with the command that updates it.
    #[test]
    fn a_refusal_of_an_old_agent_tells_the_update() {
        // given
        let details = ErrorDetails::with_error_info(AGENT_TOO_OLD, "scylla", HashMap::new());
        let status = Status::with_error_details(Code::FailedPrecondition, "too old", details);

        // when
        let error = refusal(&status, "https://ci.example.com/");

        // then
        let AgentError::Refused(text) = error else {
            panic!("{error:?}");
        };
        assert!(text.contains("https://ci.example.com/install."), "{text}");
        assert!(text.contains("Update the agent"), "{text}");
    }
}
