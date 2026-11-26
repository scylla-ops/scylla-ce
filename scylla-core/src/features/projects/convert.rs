//! The proto values of the project list: the order, the health and the counts.

use scylla_domain::projects::lists::{ProjectCounts, ProjectHealth, ProjectOrder};
use scylla_proto::projects::v1 as pb;

use crate::api::parse::ToProto;
use crate::Error;

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
