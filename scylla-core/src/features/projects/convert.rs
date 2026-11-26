//! The proto values of the project list: the order, the health, the section, the ids and the
//! counts.

use scylla_domain::projects::lists::{ProjectCounts, ProjectHealth, ProjectOrder, ProjectSection};
use scylla_domain::projects::ProjectId;
use scylla_proto::common::v1::Id;
use scylla_proto::projects::v1 as pb;

use crate::api::parse::{self, ToProto};
use crate::Error;

/// The largest number of project ids in the filter of a list.
const MAX_PROJECT_IDS: usize = 100;

/// Reads the order of a request. Not set gives the order of the workspace, and an unknown order
/// gives `INVALID_ARGUMENT`.
pub(super) fn order(raw: i32) -> Result<ProjectOrder, Error> {
    match pb::ProjectOrder::try_from(raw) {
        Ok(pb::ProjectOrder::Unspecified | pb::ProjectOrder::Position) => {
            Ok(ProjectOrder::Position)
        }
        Ok(pb::ProjectOrder::Name) => Ok(ProjectOrder::Name),
        Ok(pb::ProjectOrder::LastJob) => Ok(ProjectOrder::LastJob),
        Ok(pb::ProjectOrder::FailedFirst) => Ok(ProjectOrder::FailedFirst),
        Err(_) => Err(Error::invalid("order", "is not an order")),
    }
}

/// Reads the health of a request. Not set gives each project, and an unknown health gives
/// `INVALID_ARGUMENT`.
pub(super) fn health(raw: i32) -> Result<ProjectHealth, Error> {
    match pb::ProjectHealth::try_from(raw) {
        Ok(pb::ProjectHealth::Unspecified | pb::ProjectHealth::All) => Ok(ProjectHealth::All),
        Ok(pb::ProjectHealth::Failed) => Ok(ProjectHealth::Failed),
        Ok(pb::ProjectHealth::Running) => Ok(ProjectHealth::Running),
        Err(_) => Err(Error::invalid("health", "is not a health")),
    }
}

/// Reads the section of a request: a group, or the projects with no group. Not set gives each
/// project.
pub(super) fn section(raw: Option<pb::ProjectSection>) -> Result<Option<ProjectSection>, Error> {
    let Some(section) = raw else {
        return Ok(None);
    };
    Ok(Some(match section.group_id {
        None => ProjectSection::Ungrouped,
        Some(id) => ProjectSection::Group(parse::field(id, "section.group_id")?),
    }))
}

/// Reads the project ids of a request. More than `MAX_PROJECT_IDS` give `INVALID_ARGUMENT`.
pub(super) fn ids(raw: Vec<Id>) -> Result<Vec<ProjectId>, Error> {
    if raw.len() > MAX_PROJECT_IDS {
        return Err(Error::invalid("project_ids", "must have at most 100 ids"));
    }
    raw.into_iter()
        .map(|id| parse::field(id, "project_ids"))
        .collect()
}

impl ToProto<pb::ProjectCounts> for ProjectCounts {
    fn to_proto(self) -> pb::ProjectCounts {
        pb::ProjectCounts {
            total: self.total,
            failed: self.failed,
            running: self.running,
            pipelines: self.pipelines,
        }
    }
}
