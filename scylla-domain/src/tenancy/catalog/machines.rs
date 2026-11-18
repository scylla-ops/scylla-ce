//! The area of the machines in a project: the jobs that an agent takes. Only a role for apps can
//! hold it. Each route is on the project and on its workspace.

use super::{Area, CatalogPermission, Level, Route};
use crate::projects::pipelines::{PipelineAccess, PipelineHolder};

/// The routes of the agents that run the jobs.
const RUN_JOBS: &[Route] = &[
    Route::of(PipelineHolder::RUN_JOBS),
    Route::of(PipelineAccess::RUN_JOBS),
];

/// The areas of this level.
pub(super) const AREAS: &[Area] = &[Area {
    id: "job_execution",
    level: Level::Project,
    machine: true,
    permissions: &[
        CatalogPermission {
            id: "job_execution.edit",
            action: "edit",
            depends_on: &[],
            implicit: false,
            routes: RUN_JOBS,
        },
        CatalogPermission {
            id: "job_execution.run",
            action: "run",
            depends_on: &["job_execution.edit"],
            implicit: false,
            routes: RUN_JOBS,
        },
    ],
}];
