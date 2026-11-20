//! The conversions of `ListAttentionItems`: the request to a query, and the page of open items to
//! the response.

use scylla_domain::projects::jobs::MAX_LIST_IDS;
use scylla_domain::projects::triggers::FireSource;
use scylla_domain::tenancy::dashboards::attention::{
    AttentionCursor, AttentionItem, AttentionKind, AttentionList, AttentionQuery, AttentionReach,
    ItemNode, KindCount, Subject,
};
use scylla_domain::{PageRequest, PageSize};
use scylla_proto::common::v1 as common;
use scylla_proto::dashboards::v1 as pb;
use scylla_proto::pipelines::v1::FireSource as ProtoSource;

use crate::api::parse::{self, FromProto, Invalid, ToProto};
use crate::features::projects::jobs::convert::status;
use crate::Error;

/// Reads a request. The query reads nothing: the handler sets what the caller reads.
pub(super) fn query(request: pb::ListAttentionItemsRequest) -> Result<AttentionQuery, Error> {
    if request.project_ids.len() > MAX_LIST_IDS {
        let reason = format!("must have at most {MAX_LIST_IDS} ids");
        return Err(Error::invalid("project_ids", reason));
    }
    let kinds = request
        .kinds
        .into_iter()
        .map(kind_of)
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| Error::invalid("kinds", "must be known kinds"))?;
    Ok(AttentionQuery {
        workspace: parse::required(request.workspace_id, "workspace_id")?,
        projects: request
            .project_ids
            .into_iter()
            .map(|id| parse::field(id, "project_ids"))
            .collect::<Result<_, _>>()?,
        kinds,
        since: request
            .since
            .map(|at| parse::field(at, "since"))
            .transpose()?,
        page: parse::optional(request.page, "page")?,
        reach: AttentionReach::default(),
    })
}

/// Makes the response of a page.
pub(super) fn response(list: AttentionList) -> pb::ListAttentionItemsResponse {
    pb::ListAttentionItemsResponse {
        items: list.items.into_iter().map(item).collect(),
        counts: list.counts.into_iter().map(count).collect(),
        open_count: list.open,
        new_count: list.new,
        last_cleared_at: list.last_cleared_at.map(parse::timestamp),
        page: Some(list.next_after.to_proto()),
    }
}

impl FromProto<common::PageRequest> for PageRequest<AttentionCursor> {
    fn from_proto(value: common::PageRequest) -> Result<Self, Invalid> {
        let size = PageSize::parse(value.size).map_err(|error| Invalid::at("size", error))?;
        let after = match value.token.as_str() {
            "" => None,
            token => Some(
                AttentionCursor::parse(token)
                    .ok_or_else(|| Invalid::at("token", "is not a token of this list"))?,
            ),
        };
        Ok(Self { after, size })
    }
}

impl ToProto<common::PageResponse> for Option<AttentionCursor> {
    fn to_proto(self) -> common::PageResponse {
        common::PageResponse {
            next_token: self.map(|key| key.text()).unwrap_or_default(),
        }
    }
}

/// Reads a kind of a filter, or `None` for an unknown kind.
fn kind_of(raw: i32) -> Option<AttentionKind> {
    match pb::AttentionKind::try_from(raw).ok()? {
        pb::AttentionKind::JobFailed => Some(AttentionKind::JobFailed),
        pb::AttentionKind::JobOrphaned => Some(AttentionKind::JobOrphaned),
        pb::AttentionKind::FireFailed => Some(AttentionKind::FireFailed),
        pb::AttentionKind::FireMissed => Some(AttentionKind::FireMissed),
        pb::AttentionKind::AgentOffline => Some(AttentionKind::AgentOffline),
        pb::AttentionKind::ApprovalWaiting => Some(AttentionKind::ApprovalWaiting),
        pb::AttentionKind::Unspecified => None,
    }
}

/// Returns the proto of a kind.
fn kind(kind: AttentionKind) -> pb::AttentionKind {
    match kind {
        AttentionKind::JobFailed => pb::AttentionKind::JobFailed,
        AttentionKind::JobOrphaned => pb::AttentionKind::JobOrphaned,
        AttentionKind::FireFailed => pb::AttentionKind::FireFailed,
        AttentionKind::FireMissed => pb::AttentionKind::FireMissed,
        AttentionKind::AgentOffline => pb::AttentionKind::AgentOffline,
        AttentionKind::ApprovalWaiting => pb::AttentionKind::ApprovalWaiting,
    }
}

/// Returns the proto of the count of a kind.
fn count(count: KindCount) -> pb::AttentionCount {
    pb::AttentionCount {
        kind: kind(count.kind).into(),
        open_count: count.open,
        new_count: count.new,
    }
}

/// Returns the proto of an item.
fn item(item: AttentionItem) -> pb::AttentionItem {
    let place = item.place;
    pb::AttentionItem {
        kind: kind(item.kind).into(),
        at: Some(parse::timestamp(item.at)),
        is_new: item.is_new,
        count: item.count,
        project_id: place.as_ref().map(|place| place.project_id.to_proto()),
        pipeline_id: place.as_ref().map(|place| place.pipeline_id.to_proto()),
        project_name: place
            .as_ref()
            .map(|place| place.project_name.clone())
            .unwrap_or_default(),
        pipeline_name: place.map(|place| place.pipeline_name).unwrap_or_default(),
        subject: Some(subject(item.subject)),
    }
}

/// Returns the proto of what an item is about.
fn subject(subject: Subject) -> pb::attention_item::Subject {
    use pb::attention_item::Subject as Proto;
    match subject {
        Subject::Job(job) => {
            let (agent_id, agent_name) = job.agent.unzip();
            Proto::Job(pb::AttentionJob {
                id: Some(job.id.to_proto()),
                status: status(job.status).into(),
                error: job.error.unwrap_or_default(),
                branch: job.branch.unwrap_or_default(),
                agent_id: agent_id.map(ToProto::to_proto),
                agent_name: agent_name.unwrap_or_default(),
                node: job.node.map(node),
            })
        }
        Subject::Fire(fire) => Proto::Fire(pb::AttentionFire {
            id: Some(fire.id.to_proto()),
            trigger_id: Some(fire.trigger_id.to_proto()),
            trigger_name: fire.trigger_name,
            schedule: fire.schedule.unwrap_or_default(),
            source: source(fire.source).into(),
            error: fire.error.unwrap_or_default(),
        }),
        Subject::Agent(agent) => Proto::Agent(pb::AttentionAgent {
            id: Some(agent.id.to_proto()),
            name: agent.name,
            last_seen_at: Some(parse::timestamp(agent.last_seen_at)),
        }),
    }
}

/// Returns the proto of the node of a job.
fn node(node: ItemNode) -> pb::AttentionNode {
    pb::AttentionNode {
        node_id: node.id.as_str().to_owned(),
        error: node.error.unwrap_or_default(),
        attempt: node.attempt,
        max_attempts: node.max_attempts,
        timeout_seconds: node.timeout_seconds,
    }
}

/// Returns the proto of the source of a fire.
fn source(source: FireSource) -> ProtoSource {
    match source {
        FireSource::Schedule => ProtoSource::Schedule,
        FireSource::Webhook => ProtoSource::Webhook,
        FireSource::Manual => ProtoSource::Manual,
    }
}
