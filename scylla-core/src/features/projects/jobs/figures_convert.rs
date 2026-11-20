//! The conversions of `GetJobStats`: the request to a query of the figures of the jobs, and the
//! figures to the response.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{
    Bucket, Figures, FiguresPeriod, FiguresQuery, Group, GroupBy, GroupKey, GroupOrder, JobFigures,
    JobOrigin, Outcomes, WorkspaceJobs, DEFAULT_GROUPS, MAX_GROUPS,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Span;
use scylla_proto::pipelines::v1 as pb;
use time::OffsetDateTime;

use super::{filter, lists};
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Reads a request of `GetJobStats` at the time `now`. The query sees each project of the
/// workspace: the caller sets the projects that it can see.
pub(super) fn query(
    request: pb::GetJobStatsRequest,
    now: OffsetDateTime,
) -> Result<FiguresQuery, Error> {
    // Read the choices of the figures.
    let bucket = span(request.bucket());
    let group_by = group_by(request.group_by());
    let order = order(request.group_order());
    let limit = match request.group_limit {
        0 => DEFAULT_GROUPS,
        limit if limit > MAX_GROUPS => {
            let reason = format!("must be at most {MAX_GROUPS}");
            return Err(Error::invalid("group_limit", reason));
        }
        limit => limit,
    };

    // Read the period.
    let from: OffsetDateTime = parse::required(request.from, "from")?;
    let to: Option<OffsetDateTime> = request.to.map(|to| parse::field(to, "to")).transpose()?;
    let period = FiguresPeriod::new(from, to, now)
        .map_err(|error| Error::invalid("to", error.to_string()))?;

    // Read the jobs to count.
    let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
    let agent: Option<AppId> = request
        .agent_id
        .map(|id| parse::field(id, "agent_id"))
        .transpose()?;
    let origins = request
        .origins
        .into_iter()
        .map(filter::origin_of)
        .collect::<Option<Vec<JobOrigin>>>()
        .ok_or_else(|| Error::invalid("origins", "must be known origins"))?;
    let jobs = WorkspaceJobs {
        workspace,
        visible: None,
        agent,
        projects: lists::ids(request.project_ids, "project_ids")?,
        pipelines: lists::ids(request.pipeline_ids, "pipeline_ids")?,
    };
    Ok(FiguresQuery {
        jobs,
        origins,
        period,
        bucket,
        group_by,
        order,
        limit,
        compare: request.compare,
    })
}

/// Returns the size of the buckets of a request, or `None` for no bucket.
fn span(bucket: pb::JobStatsBucket) -> Option<Span> {
    match bucket {
        pb::JobStatsBucket::Unspecified => None,
        pb::JobStatsBucket::Hour => Some(Span::Hour),
        pb::JobStatsBucket::Day => Some(Span::Day),
        pb::JobStatsBucket::Week => Some(Span::Week),
    }
}

/// Returns what makes the groups of a request, or `None` for no group.
fn group_by(group: pb::JobStatsGroup) -> Option<GroupBy> {
    match group {
        pb::JobStatsGroup::Unspecified => None,
        pb::JobStatsGroup::Project => Some(GroupBy::Project),
        pb::JobStatsGroup::Pipeline => Some(GroupBy::Pipeline),
        pb::JobStatsGroup::Agent => Some(GroupBy::Agent),
        pb::JobStatsGroup::Origin => Some(GroupBy::Origin),
    }
}

/// Returns the order of the groups of a request.
fn order(order: pb::JobStatsOrder) -> GroupOrder {
    match order {
        pb::JobStatsOrder::Unspecified | pb::JobStatsOrder::Count => GroupOrder::Count,
        pb::JobStatsOrder::Failed => GroupOrder::Failed,
        pb::JobStatsOrder::P50Duration => GroupOrder::P50Duration,
        pb::JobStatsOrder::RunTime => GroupOrder::RunTime,
    }
}

impl ToProto<pb::GetJobStatsResponse> for JobFigures {
    fn to_proto(self) -> pb::GetJobStatsResponse {
        pb::GetJobStatsResponse {
            totals: Some(self.totals.to_proto()),
            buckets: self.buckets.into_iter().map(bucket).collect(),
            groups: self.groups.into_iter().map(group).collect(),
            previous: self.previous.map(ToProto::to_proto),
            time_zone: self.time_zone.as_str().to_owned(),
        }
    }
}

impl ToProto<pb::JobFigures> for Figures {
    fn to_proto(self) -> pb::JobFigures {
        pb::JobFigures {
            created: self.created,
            outcomes: Some(self.outcomes.to_proto()),
            p50_duration: self.p50_duration.map(parse::duration),
            p95_duration: self.p95_duration.map(parse::duration),
            p50_queue: self.p50_queue.map(parse::duration),
            p95_queue: self.p95_queue.map(parse::duration),
            run_time: Some(parse::duration(self.run_time)),
        }
    }
}

impl ToProto<pb::JobOutcomes> for Outcomes {
    fn to_proto(self) -> pb::JobOutcomes {
        pb::JobOutcomes {
            completed: self.completed,
            failed: self.failed,
            cancelled: self.cancelled,
            orphaned: self.orphaned,
        }
    }
}

/// Makes the message of a bucket.
fn bucket(bucket: Bucket) -> pb::JobBucket {
    pb::JobBucket {
        start: Some(parse::timestamp(bucket.start)),
        figures: Some(bucket.figures.to_proto()),
    }
}

/// Makes the message of a group.
fn group(group: Group) -> pb::JobGroup {
    let key = match group.key {
        GroupKey::Project(id) => pb::job_group::Key::Id(id.to_proto()),
        GroupKey::Pipeline(id) => pb::job_group::Key::Id(id.to_proto()),
        GroupKey::Agent(id) => pb::job_group::Key::Id(id.to_proto()),
        GroupKey::Origin(origin) => pb::job_group::Key::Origin(filter::origin(origin).into()),
    };
    pb::JobGroup {
        key: Some(key),
        figures: Some(group.figures.to_proto()),
        previous: group.previous.map(ToProto::to_proto),
    }
}
