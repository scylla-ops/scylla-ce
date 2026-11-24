//! The conversions of the summaries of the triggers, of their fires and of their counts to the
//! responses of the lists of a workspace.

use scylla_domain::projects::triggers::{
    FireFigures, FireGroup, FireGroupKey, FireId, FireSource, FireStats, FireStatus, TriggerFire,
    TriggerId, TriggerSummary,
};
use scylla_domain::{Page, TimeZone};
use scylla_proto::pipelines::v1 as pb;
use time::OffsetDateTime;

use crate::api::parse::{self, ToProto};
use crate::features::projects::jobs::convert as jobs_convert;

/// Makes the response of `ListWorkspaceTriggers`.
pub(super) fn summaries_response(
    page: Page<TriggerSummary, TriggerId>,
    zone: TimeZone,
) -> pb::ListWorkspaceTriggersResponse {
    pb::ListWorkspaceTriggersResponse {
        triggers: page.items.into_iter().map(ToProto::to_proto).collect(),
        page: Some(page.next_after.to_proto()),
        time_zone: zone.as_str().to_owned(),
    }
}

/// Makes the response of `ListTriggerFires`.
pub(super) fn fires_response(page: Page<TriggerFire, FireId>) -> pb::ListTriggerFiresResponse {
    pb::ListTriggerFiresResponse {
        fires: page.items.into_iter().map(ToProto::to_proto).collect(),
        page: Some(page.next_after.to_proto()),
    }
}

impl ToProto<pb::TriggerSummary> for TriggerSummary {
    fn to_proto(self) -> pb::TriggerSummary {
        pb::TriggerSummary {
            trigger: Some(self.trigger.to_proto()),
            pipeline_name: self.pipeline_name,
            project_name: self.project_name,
            upcoming_fires: self.upcoming.into_iter().map(parse::timestamp).collect(),
            last_job_status: self
                .last_job_status
                .map_or(pb::JobStatus::Unspecified, jobs_convert::status)
                .into(),
            last_fire: self.last_fire.map(ToProto::to_proto),
            failed_fires: self.failed_fires,
            missed_times: self.missed_times,
        }
    }
}

impl ToProto<pb::TriggerFire> for TriggerFire {
    fn to_proto(self) -> pb::TriggerFire {
        let source = match self.source {
            FireSource::Schedule => pb::FireSource::Schedule,
            FireSource::Webhook => pb::FireSource::Webhook,
            FireSource::Manual => pb::FireSource::Manual,
        };
        let result = match self.status {
            FireStatus::Ok => pb::FireStatus::Ok,
            FireStatus::Error => pb::FireStatus::Error,
            FireStatus::Missed => pb::FireStatus::Missed,
            FireStatus::Skipped => pb::FireStatus::Skipped,
        };
        pb::TriggerFire {
            id: Some(self.id.to_proto()),
            trigger_id: Some(self.trigger_id.to_proto()),
            trigger_name: self.trigger_name,
            pipeline_id: Some(self.pipeline_id.to_proto()),
            project_id: Some(self.project_id.to_proto()),
            source: source.into(),
            due_at: Some(parse::timestamp(self.due_at)),
            fired_at: self.fired_at.map(parse::timestamp),
            status: result.into(),
            error: self.error.unwrap_or_default(),
            job_id: self.job_id.map(ToProto::to_proto),
            job_status: self
                .job_status
                .map_or(pb::JobStatus::Unspecified, jobs_convert::status)
                .into(),
        }
    }
}

impl ToProto<pb::GetTriggerFireStatsResponse> for FireStats {
    fn to_proto(self) -> pb::GetTriggerFireStatsResponse {
        let bucket = |(start, figures): (OffsetDateTime, FireFigures)| pb::TriggerFireBucket {
            start: Some(parse::timestamp(start)),
            figures: Some(figures.to_proto()),
        };
        let group = |group: FireGroup| pb::TriggerFireGroup {
            id: Some(match group.key {
                FireGroupKey::Trigger(id) => id.to_proto(),
                FireGroupKey::Pipeline(id) => id.to_proto(),
                FireGroupKey::Project(id) => id.to_proto(),
            }),
            figures: Some(group.figures.to_proto()),
        };
        pb::GetTriggerFireStatsResponse {
            totals: Some(self.totals.to_proto()),
            buckets: self.buckets.into_iter().map(bucket).collect(),
            groups: self.groups.into_iter().map(group).collect(),
            time_zone: self.time_zone.as_str().to_owned(),
        }
    }
}

impl ToProto<pb::TriggerFireFigures> for FireFigures {
    fn to_proto(self) -> pb::TriggerFireFigures {
        pb::TriggerFireFigures {
            total: self.total,
            ok: self.ok,
            failed_fires: self.failed,
            missed_times: self.missed,
            skipped: self.skipped,
            p50_delay: self.p50_delay.map(parse::duration),
            p95_delay: self.p95_delay.map(parse::duration),
        }
    }
}
