//! The conversions of the summaries of the pipelines between the domain and the protos.

use scylla_domain::projects::pipelines::{
    NextFire, PipelineSummary, RecentJob, SummaryOrder, SummaryPage, TriggerType, MAX_FILTER_IDS,
};
use scylla_domain::Id;
use scylla_proto::pipelines::v1 as pb;

use super::files_convert;
use crate::api::parse::{self, ToProto};
use crate::features::projects::jobs::convert as jobs_convert;
use crate::Error;

/// Reads the order of a request. Not set gives the failing pipelines first, and an unknown
/// order gives `INVALID_ARGUMENT`.
pub(super) fn order(raw: i32) -> Result<SummaryOrder, Error> {
    match pb::PipelineSummaryOrder::try_from(raw) {
        Ok(pb::PipelineSummaryOrder::Unspecified | pb::PipelineSummaryOrder::FailingFirst) => {
            Ok(SummaryOrder::FailingFirst)
        }
        Ok(pb::PipelineSummaryOrder::Slowest) => Ok(SummaryOrder::Slowest),
        Ok(pb::PipelineSummaryOrder::Name) => Ok(SummaryOrder::Name),
        Err(_) => Err(Error::invalid("order", "is not an order")),
    }
}

/// Reads the ids of a filter, each one time. More than `MAX_FILTER_IDS` gives
/// `INVALID_ARGUMENT`.
pub(super) fn ids<T: ?Sized>(
    raw: Vec<scylla_proto::common::v1::Id>,
    field: &'static str,
) -> Result<Vec<Id<T>>, Error> {
    if raw.len() > MAX_FILTER_IDS {
        return Err(Error::invalid(field, "must have at most 100 ids"));
    }
    let mut ids: Vec<Id<T>> = Vec::with_capacity(raw.len());
    for id in raw {
        let id = parse::field(id, field)?;
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

/// Returns the answer of a page of summaries.
pub(super) fn response(page: SummaryPage) -> pb::ListPipelineSummariesResponse {
    pb::ListPipelineSummariesResponse {
        pipelines: page.items.into_iter().map(ToProto::to_proto).collect(),
        total: page.total,
        green: page.green,
        page: Some(page.next_after.to_proto()),
    }
}

impl ToProto<pb::PipelineSummary> for PipelineSummary {
    fn to_proto(self) -> pb::PipelineSummary {
        pb::PipelineSummary {
            pipeline_id: Some(self.pipeline_id.to_proto()),
            name: self.name,
            project_id: Some(self.project_id.to_proto()),
            project_name: self.project_name,
            recent: self.recent.into_iter().map(ToProto::to_proto).collect(),
            completed: self.completed,
            failed: self.failed,
            median_duration: self.median.map(parse::duration),
            last_ended_status: self
                .last_ended
                .map_or(pb::JobStatus::Unspecified, jobs_convert::status)
                .into(),
            active: self.active,
            trigger_kinds: self
                .trigger_kinds
                .into_iter()
                .map(|kind| trigger_kind(kind).into())
                .collect(),
            next_fire: self.next_fire.map(ToProto::to_proto),
            source: files_convert::source_of(self.source).into(),
            key_taken: self.key_taken,
        }
    }
}

impl ToProto<pb::RecentJob> for RecentJob {
    fn to_proto(self) -> pb::RecentJob {
        pb::RecentJob {
            id: Some(self.id.to_proto()),
            status: jobs_convert::status(self.status).into(),
            branch: self.branch.unwrap_or_default(),
            created_at: Some(parse::timestamp(self.created_at)),
            started_at: self.started_at.map(parse::timestamp),
            finished_at: self.finished_at.map(parse::timestamp),
        }
    }
}

impl ToProto<pb::NextFire> for NextFire {
    fn to_proto(self) -> pb::NextFire {
        pb::NextFire {
            trigger_id: Some(self.trigger_id.to_proto()),
            schedule: self.schedule,
            at: Some(parse::timestamp(self.at)),
        }
    }
}

/// Returns the proto of the kind of a trigger.
fn trigger_kind(kind: TriggerType) -> pb::TriggerKind {
    match kind {
        TriggerType::Cron => pb::TriggerKind::Cron,
        TriggerType::Webhook => pb::TriggerKind::Webhook,
        TriggerType::Push => pb::TriggerKind::Push,
        TriggerType::PullRequest => pb::TriggerKind::PullRequest,
        TriggerType::Tag => pb::TriggerKind::Tag,
    }
}
