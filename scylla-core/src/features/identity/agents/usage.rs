//! The handlers of `AgentService` for the use of the agents: the busy job slots of a workspace,
//! and the samples of the machine of one agent.

use scylla_domain::identity::agents::{
    AgentAccess, AgentHolder, AgentMetrics, MetricsBucket, MetricsFigures, MetricsQuery,
    SlotBucket, SlotFigures, SlotQuery, SlotUsage,
};
use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::FiguresPeriod;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Span;
use scylla_extension::AccessError;
use scylla_proto::agents::v1 as pb;
use scylla_proto::pipelines::v1::JobStatsBucket;
use time::OffsetDateTime;
use tonic::Request;

use super::grpc::AgentsGrpc;
use super::samples;
use crate::api::parse::{self, ToProto};
use crate::features::identity::apps;
use crate::features::projects::jobs::slots;
use crate::Error;

impl AgentsGrpc {
    /// Handles `GetSlotUsage`: the busy slots of the agents of a workspace for a period, and the
    /// slots of its connected agents.
    pub(super) async fn slot_usage(
        &self,
        request: Request<pb::GetSlotUsageRequest>,
    ) -> Result<pb::GetSlotUsageResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let now = OffsetDateTime::now_utc();
        let request = request.into_inner();
        let bucket = span(request.bucket());
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let agent: Option<AppId> = request
            .agent_id
            .map(|id| parse::field(id, "agent_id"))
            .transpose()?;
        let period = period(request.from, request.to, now)?;

        // Check the route permission on the workspace.
        self.gate
            .authorize(&caller, AgentHolder::GET_SLOT_USAGE.on(workspace))
            .await?;

        // Add the busy time of the jobs, then the slots of the connected agents.
        let query = SlotQuery {
            workspace,
            agent,
            period,
            bucket,
            now,
        };
        let mut usage = slots::usage(&self.pool, query).await?;
        let connected: Vec<AppId> = self
            .vcs
            .registry
            .connected_in(workspace)
            .into_iter()
            .filter(|id| agent.is_none_or(|agent| agent == *id))
            .collect();
        usage.capacity = apps::capacity(&self.pool, workspace, &connected).await?;
        Ok(usage.to_proto())
    }

    /// Handles `GetAgentMetrics`: the samples of the machine of one agent for a period. An
    /// unknown App gets the answer of a refusal.
    pub(super) async fn agent_metrics(
        &self,
        request: Request<pb::GetAgentMetricsRequest>,
    ) -> Result<pb::GetAgentMetricsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let now = OffsetDateTime::now_utc();
        let request = request.into_inner();
        let bucket = span(request.bucket());
        let agent: AppId = parse::required(request.agent_id, "agent_id")?;
        let period = period(request.from, request.to, now)?;

        // Check the route permission in the workspace of the App.
        let check = AgentAccess::GET_AGENT_METRICS.on(agent);
        let Some(app) = apps::get(&self.pool, agent).await? else {
            return Err(AccessError::denied(&caller, &check).into());
        };
        self.gate
            .authorize(&caller, check.within(app.workspace_id))
            .await?;

        // Add the samples of the period.
        let query = MetricsQuery {
            agent,
            period,
            bucket,
        };
        let metrics = samples::metrics(&self.pool, query).await?;
        Ok(metrics.to_proto())
    }
}

/// Reads the period of a request at the time `now`: `from` is required, and `to` is now when it
/// is not set.
fn period(
    from: Option<prost_types::Timestamp>,
    to: Option<prost_types::Timestamp>,
    now: OffsetDateTime,
) -> Result<FiguresPeriod, Error> {
    let from: OffsetDateTime = parse::required(from, "from")?;
    let to: Option<OffsetDateTime> = to.map(|to| parse::field(to, "to")).transpose()?;
    FiguresPeriod::new(from, to, now).map_err(|error| Error::invalid("to", error.to_string()))
}

/// Returns the size of the buckets of a request, or `None` for no bucket.
fn span(bucket: JobStatsBucket) -> Option<Span> {
    match bucket {
        JobStatsBucket::Unspecified => None,
        JobStatsBucket::Hour => Some(Span::Hour),
        JobStatsBucket::Day => Some(Span::Day),
        JobStatsBucket::Week => Some(Span::Week),
    }
}

impl ToProto<pb::GetSlotUsageResponse> for SlotUsage {
    fn to_proto(self) -> pb::GetSlotUsageResponse {
        pb::GetSlotUsageResponse {
            totals: Some(slot_figures(self.totals)),
            buckets: self.buckets.into_iter().map(slot_bucket).collect(),
            capacity: self.capacity,
            time_zone: self.time_zone.as_str().to_owned(),
        }
    }
}

impl ToProto<pb::GetAgentMetricsResponse> for AgentMetrics {
    fn to_proto(self) -> pb::GetAgentMetricsResponse {
        pb::GetAgentMetricsResponse {
            totals: Some(metrics_figures(self.totals)),
            buckets: self.buckets.into_iter().map(metrics_bucket).collect(),
            time_zone: self.time_zone.as_str().to_owned(),
        }
    }
}

/// Makes the message of busy slots.
fn slot_figures(figures: SlotFigures) -> pb::SlotFigures {
    pb::SlotFigures {
        busy_slots_avg: figures.busy_avg,
        busy_slots_max: figures.busy_max,
    }
}

/// Makes the message of a bucket of busy slots.
fn slot_bucket(bucket: SlotBucket) -> pb::SlotBucket {
    pb::SlotBucket {
        start: Some(parse::timestamp(bucket.start)),
        figures: Some(slot_figures(bucket.figures)),
    }
}

/// Makes the message of the samples of a machine.
fn metrics_figures(figures: MetricsFigures) -> pb::MetricsFigures {
    pb::MetricsFigures {
        samples: figures.samples,
        cpu_avg_percent: figures.cpu_avg,
        cpu_max_percent: figures.cpu_max,
        memory_avg_bytes: figures.memory_avg,
        memory_max_bytes: figures.memory_max,
    }
}

/// Makes the message of a bucket of samples.
fn metrics_bucket(bucket: MetricsBucket) -> pb::MetricsBucket {
    pb::MetricsBucket {
        start: Some(parse::timestamp(bucket.start)),
        figures: Some(metrics_figures(bucket.figures)),
    }
}
