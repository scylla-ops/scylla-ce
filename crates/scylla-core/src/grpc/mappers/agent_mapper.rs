//! Wire to the inputs of a use case, its outcome to wire. The handler holds none of it.

use crate::application::{AgentStats, AgentView};
use crate::grpc::convert::{Parse, id, ts, valid, wrap};
use scylla_domain::domain::agent::AgentHost;
use scylla_domain::domain::app::{App, AppName, AppSecretLabel};
use scylla_domain::domain::ids::{AppId, OrganizationId};
use scylla_proto::agent::v1::{
    Agent as ProtoAgent, AgentHost as ProtoAgentHost, AgentStats as ProtoAgentStats,
    CreateAgentRequest, DailyOutcome as ProtoDailyOutcome, DeleteAgentRequest, GetAgentRequest,
    GetAgentStatsRequest, ListAgentsRequest,
};
use tonic::Status;

/// A freshly created agent: never connected, nothing in flight.
pub fn agent_to_proto(app: &App) -> ProtoAgent {
    ProtoAgent {
        agent_id: wrap(app.id().to_string()),
        organization_id: wrap(app.organization_id().to_string()),
        name: app.name().as_str().to_string(),
        is_active: app.is_active(),
        created_at: ts(app.created_at()),
        updated_at: ts(app.updated_at()),
        connected: false,
        last_seen: None,
        in_flight: 0,
        host: None,
    }
}

pub fn agent_view_to_proto(view: &AgentView) -> ProtoAgent {
    ProtoAgent {
        connected: view.connected,
        last_seen: view.last_seen.and_then(ts),
        in_flight: i32::try_from(view.in_flight).unwrap_or(i32::MAX),
        host: view.host.as_ref().map(host_to_proto),
        ..agent_to_proto(&view.app)
    }
}

fn host_to_proto(h: &AgentHost) -> ProtoAgentHost {
    ProtoAgentHost {
        version: h.version.clone(),
        os: h.os.clone(),
        arch: h.arch.clone(),
        hostname: h.hostname.clone(),
        cpu_count: h.cpu_count.unwrap_or(0),
        total_memory_mb: h.total_memory_mb.unwrap_or(0),
        reported_at: ts(h.reported_at),
    }
}

pub fn agent_stats_to_proto(s: &AgentStats) -> ProtoAgentStats {
    ProtoAgentStats {
        total: s.total,
        pending: s.pending,
        running: s.running,
        completed: s.completed,
        failed: s.failed,
        cancelled: s.cancelled,
        orphaned: s.orphaned,
        last_run_at: s.last_run_at.and_then(ts),
        median_duration_ms: s.median_duration_ms,
        p95_duration_ms: s.p95_duration_ms,
        daily: s
            .daily
            .iter()
            .map(|d| ProtoDailyOutcome {
                day: ts(d.day),
                completed: d.completed,
                failed: d.failed,
                cancelled: d.cancelled,
                orphaned: d.orphaned,
                median_duration_ms: d.median_duration_ms,
            })
            .collect(),
    }
}

impl Parse for CreateAgentRequest {
    type Into = (OrganizationId, AppName, Option<AppSecretLabel>);

    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = id(self.organization_id, "organization_id")?;
        let name = valid(self.name, AppName::new)?;
        let secret_label = self
            .secret_label
            .map(|label| valid(label, AppSecretLabel::new))
            .transpose()?;
        Ok((organization_id, name, secret_label))
    }
}

impl Parse for ListAgentsRequest {
    type Into = OrganizationId;

    fn parse(self) -> Result<OrganizationId, Status> {
        id(self.organization_id, "organization_id")
    }
}

impl Parse for GetAgentRequest {
    type Into = AppId;

    fn parse(self) -> Result<AppId, Status> {
        id(self.agent_id, "agent_id")
    }
}

impl Parse for GetAgentStatsRequest {
    type Into = AppId;

    fn parse(self) -> Result<AppId, Status> {
        id(self.agent_id, "agent_id")
    }
}

impl Parse for DeleteAgentRequest {
    type Into = AppId;

    fn parse(self) -> Result<AppId, Status> {
        id(self.agent_id, "agent_id")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_proto::common::v1 as common;
    use tonic::Code;

    #[test]
    fn a_create_request_becomes_the_validated_inputs() {
        let (organization_id, name, secret_label) = CreateAgentRequest {
            organization_id: wrap("acme"),
            name: "runner-1".into(),
            secret_label: Some("Build-01.example.com".into()),
        }
        .parse()
        .unwrap();

        assert_eq!(organization_id.as_str(), "acme");
        assert_eq!(name.as_str(), "runner-1");
        assert_eq!(secret_label.unwrap().as_str(), "Build-01.example.com");
    }

    #[test]
    fn an_invalid_first_label_is_an_invalid_argument() {
        let Err(err) = CreateAgentRequest {
            organization_id: wrap("acme"),
            name: "runner-1".into(),
            secret_label: Some("a".repeat(65)),
        }
        .parse() else {
            panic!("a label over 64 bytes must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn an_agent_is_deleted_as_its_app() {
        let id: AppId = DeleteAgentRequest {
            agent_id: wrap("agent-1"),
        }
        .parse()
        .unwrap();

        assert_eq!(id.as_str(), "agent-1");
        let missing = DeleteAgentRequest { agent_id: None }.parse().unwrap_err();
        assert_eq!(missing.message(), "missing agent_id");
    }

    #[test]
    fn a_missing_agent_id_is_an_invalid_argument() {
        let err = GetAgentStatsRequest {
            agent_id: None::<common::AppId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing agent_id");
    }

    #[test]
    fn a_missing_organization_id_is_an_invalid_argument() {
        let err = ListAgentsRequest {
            organization_id: None::<common::OrganizationId>,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing organization_id");
    }

    #[test]
    fn an_invalid_name_is_an_invalid_argument() {
        let Err(err) = CreateAgentRequest {
            organization_id: wrap("acme"),
            name: String::new(),
            secret_label: None,
        }
        .parse() else {
            panic!("an empty name must not parse");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }
}
