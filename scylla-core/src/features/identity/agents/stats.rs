//! The handlers of `AgentService` for the job stats of the agents.

use scylla_domain::identity::apps::{App, AppHolder, AppId};
use scylla_domain::projects::jobs::{AgentStats, DayOutcomes, Outcomes, StatsPeriod};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::AccessError;
use scylla_proto::agents::v1 as pb;
use time::OffsetDateTime;
use tonic::Request;

use super::grpc::AgentsGrpc;
use crate::api::parse::{self, ToProto};
use crate::features::identity::apps;
use crate::features::projects::jobs::stats;
use crate::Error;

impl AgentsGrpc {
    /// Handles `GetAgentStats`: the stats of one agent for a period. An unknown App gets the
    /// answer of a refusal.
    pub(super) async fn get_stats(
        &self,
        request: Request<pb::GetAgentStatsRequest>,
    ) -> Result<pb::GetAgentStatsResponse, Error> {
        // Authenticate the caller, and parse the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.agent_id, "agent_id")?;
        let from: Option<OffsetDateTime> = request
            .from
            .map(|from| parse::field(from, "from"))
            .transpose()?;
        let to: Option<OffsetDateTime> = request.to.map(|to| parse::field(to, "to")).transpose()?;

        // Check the route permission in the workspace of the App.
        let check = App::GET_AGENT_STATS.on(id);
        let Some(app) = apps::get(&self.pool, id).await? else {
            return Err(AccessError::denied(&caller, &check).into());
        };
        self.gate
            .authorize(&caller, check.within(app.workspace_id))
            .await?;

        // Count the jobs of the period.
        let now = OffsetDateTime::now_utc();
        let period = StatsPeriod::of_request(from, to, app.created_at, now)
            .map_err(|error| Error::invalid("to", error.to_string()))?;
        let stats = stats::of_agent(&self.pool, app.workspace_id, id, period).await?;
        Ok(pb::GetAgentStatsResponse {
            stats: Some(stats.to_proto()),
        })
    }

    /// Handles `ListAgentStats`: the stats of each agent of a workspace with a job.
    pub(super) async fn list_stats(
        &self,
        request: Request<pb::ListAgentStatsRequest>,
    ) -> Result<pb::ListAgentStatsResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let workspace: WorkspaceId =
            parse::required(request.into_inner().workspace_id, "workspace_id")?;
        self.gate
            .authorize(&caller, AppHolder::LIST_AGENT_STATS.on(workspace))
            .await?;
        let stats = stats::of_workspace(&self.pool, workspace).await?;
        Ok(pb::ListAgentStatsResponse {
            stats: stats.into_iter().map(ToProto::to_proto).collect(),
        })
    }
}

impl ToProto<pb::AgentStats> for AgentStats {
    fn to_proto(self) -> pb::AgentStats {
        pb::AgentStats {
            agent_id: Some(self.agent_id.to_proto()),
            outcomes: Some(outcomes(self.outcomes)),
            median_duration: self.median_duration.map(parse::duration),
            p95_duration: self.p95_duration.map(parse::duration),
            days: self.days.into_iter().map(day).collect(),
        }
    }
}

/// Makes the message of the outcomes.
fn outcomes(outcomes: Outcomes) -> pb::JobOutcomes {
    pb::JobOutcomes {
        completed: outcomes.completed,
        failed: outcomes.failed,
        cancelled: outcomes.cancelled,
        orphaned: outcomes.orphaned,
    }
}

/// Makes the message of a day.
fn day(day: DayOutcomes) -> pb::DayOutcomes {
    pb::DayOutcomes {
        day: Some(parse::timestamp(day.day)),
        outcomes: Some(outcomes(day.outcomes)),
    }
}
