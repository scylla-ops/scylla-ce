//! The conversions of the filters of a job list and of the origin of a job.

use scylla_domain::integrations::sources::GitRef;
use scylla_domain::projects::jobs::{JobFilter, JobOrigin, JobStatus};
use scylla_proto::pipelines::v1 as pb;
use time::OffsetDateTime;

use super::convert;
use crate::api::parse::{FromProto, Invalid};
use crate::features::integrations::sources;

/// The longest login of an author in a filter, in characters.
const MAX_LOGIN: usize = 255;

impl FromProto<pb::JobFilter> for JobFilter {
    fn from_proto(value: pb::JobFilter) -> Result<Self, Invalid> {
        // Read the filters of the source.
        let event = sources::convert::event_of(value.event());
        let branch = match value.branch.is_empty() {
            true => None,
            false => match GitRef::branch(value.branch) {
                Ok(GitRef::Branch(name)) => Some(name),
                Ok(_) => None,
                Err(error) => return Err(Invalid::at("branch", error)),
            },
        };
        if value.author_login.chars().count() > MAX_LOGIN {
            let reason = format!("must be at most {MAX_LOGIN} characters");
            return Err(Invalid::at("author_login", reason));
        }

        // Read the filters of the job.
        let statuses = value
            .statuses
            .into_iter()
            .map(status_of)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Invalid::at("statuses", "must be known states"))?;
        let origins = value
            .origins
            .into_iter()
            .map(origin_of)
            .collect::<Option<Vec<_>>>()
            .ok_or_else(|| Invalid::at("origins", "must be known origins"))?;
        let time = |at: Option<prost_types::Timestamp>, field| {
            at.map(OffsetDateTime::from_proto)
                .transpose()
                .map_err(|_| Invalid::at(field, "must be a valid time"))
        };
        Ok(Self {
            branch,
            event,
            author_login: Some(value.author_login).filter(|login| !login.is_empty()),
            pull_request: Some(value.pull_request).filter(|number| *number > 0),
            statuses,
            created_after: time(value.created_after, "created_after")?,
            created_before: time(value.created_before, "created_before")?,
            origins,
        })
    }
}

/// Returns the proto of the origin of a job.
pub(super) fn origin(origin: JobOrigin) -> pb::JobOrigin {
    match origin {
        JobOrigin::Manual => pb::JobOrigin::Manual,
        JobOrigin::Api => pb::JobOrigin::Api,
        JobOrigin::Rerun => pb::JobOrigin::Rerun,
        JobOrigin::Cron => pb::JobOrigin::Cron,
        JobOrigin::Webhook => pb::JobOrigin::Webhook,
        JobOrigin::Push => pb::JobOrigin::Push,
        JobOrigin::PullRequest => pb::JobOrigin::PullRequest,
        JobOrigin::Tag => pb::JobOrigin::Tag,
        JobOrigin::ProviderRerun => pb::JobOrigin::ProviderRerun,
    }
}

/// Reads an origin of a filter, or `None` for an unknown or unset origin.
pub(super) fn origin_of(value: i32) -> Option<JobOrigin> {
    let proto = pb::JobOrigin::try_from(value).ok()?;
    JobOrigin::ALL
        .iter()
        .copied()
        .find(|one| origin(*one) == proto)
}

/// Reads a state of a filter, or `None` for an unknown or unset state.
fn status_of(value: i32) -> Option<JobStatus> {
    let proto = pb::JobStatus::try_from(value).ok()?;
    [
        JobStatus::AwaitingApproval,
        JobStatus::Pending,
        JobStatus::Running,
        JobStatus::Completed,
        JobStatus::Failed,
        JobStatus::Cancelled,
        JobStatus::Orphaned,
    ]
    .into_iter()
    .find(|one| convert::status(*one) == proto)
}
