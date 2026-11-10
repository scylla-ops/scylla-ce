//! The areas of the catalog in a project, and the area of the machines. Each route is on the
//! project and on its workspace.

use super::{Area, CatalogPermission, Level, Route};
use crate::projects::pipelines::{PipelineAccess, PipelineHolder};
use crate::projects::triggers::{TriggerAccess, TriggerHolder};
use crate::projects::{Project, ProjectHolder};
use crate::tenancy::grants::ProjectMembers;

/// Makes one permission of an area of this level.
const fn cell(
    id: &'static str,
    action: &'static str,
    depends_on: &'static [&'static str],
    routes: &'static [Route],
) -> CatalogPermission {
    CatalogPermission {
        id,
        action,
        depends_on,
        implicit: false,
        routes,
    }
}

/// Makes an area of this level.
const fn area(id: &'static str, permissions: &'static [CatalogPermission]) -> Area {
    Area {
        id,
        level: Level::Project,
        machine: false,
        permissions,
    }
}

/// The areas of this level.
pub(super) const AREAS: &[Area] = &[
    area(
        "project",
        &[
            cell(
                "project.view",
                "view",
                &["pipelines.view"],
                &[
                    Route::of(ProjectHolder::GET_PROJECT),
                    Route::of(ProjectHolder::LIST_PROJECTS),
                    Route::of(Project::GET_PROJECT),
                    Route::of(Project::LIST_PROJECTS),
                ],
            ),
            cell(
                "project.edit",
                "edit",
                &["project.view"],
                &[
                    Route::of(ProjectHolder::UPDATE_PROJECT),
                    Route::of(Project::UPDATE_PROJECT),
                ],
            ),
            cell(
                "project.delete",
                "delete",
                &["project.view"],
                &[
                    Route::of(ProjectHolder::DELETE_PROJECT),
                    Route::of(Project::DELETE_PROJECT),
                ],
            ),
        ],
    ),
    area(
        "project_members",
        &[
            cell(
                "project_members.view",
                "view",
                &["project.view"],
                &[
                    Route::of(Project::LIST_ACCESS),
                    Route::of(ProjectMembers::LIST_MEMBERS),
                    Route::of(ProjectMembers::GET_MEMBER_ACCESS),
                ],
            ),
            cell(
                "project_members.manage",
                "manage",
                &["project_members.view"],
                &[Route::of(Project::GRANT), Route::of(Project::REVOKE)],
            ),
        ],
    ),
    area(
        "pipelines",
        &[
            cell(
                "pipelines.view",
                "view",
                &["project.view"],
                &[
                    Route::of(PipelineHolder::GET_PIPELINE),
                    Route::of(PipelineHolder::LIST_PROJECT_PIPELINES),
                    Route::of(PipelineHolder::LIST_WORKSPACE_PIPELINES),
                    Route::of(PipelineAccess::GET_PIPELINE),
                    Route::of(PipelineAccess::LIST_PROJECT_PIPELINES),
                ],
            ),
            cell(
                "pipelines.create",
                "create",
                &["pipelines.view"],
                &[
                    Route::of(PipelineHolder::CREATE_PIPELINE),
                    Route::of(PipelineAccess::CREATE_PIPELINE),
                ],
            ),
            cell(
                "pipelines.edit",
                "edit",
                &["pipelines.view"],
                &[
                    Route::of(PipelineHolder::UPDATE_PIPELINE),
                    Route::of(PipelineAccess::UPDATE_PIPELINE),
                    Route::of(PipelineHolder::SAVE_PIPELINE_LAYOUT),
                    Route::of(PipelineAccess::SAVE_PIPELINE_LAYOUT),
                ],
            ),
            cell(
                "pipelines.delete",
                "delete",
                &["pipelines.view"],
                &[
                    Route::of(PipelineHolder::DELETE_PIPELINE),
                    Route::of(PipelineAccess::DELETE_PIPELINE),
                ],
            ),
            cell(
                "pipelines.run",
                "run",
                &["pipelines.view"],
                &[
                    Route::of(PipelineHolder::RUN_PIPELINE),
                    Route::of(PipelineAccess::RUN_PIPELINE),
                ],
            ),
        ],
    ),
    area(
        "triggers",
        &[cell(
            "triggers.manage",
            "manage",
            &["pipelines.view"],
            &[
                Route::of(TriggerHolder::CREATE_TRIGGER),
                Route::of(TriggerHolder::GET_TRIGGER),
                Route::of(TriggerHolder::UPDATE_TRIGGER),
                Route::of(TriggerHolder::DELETE_TRIGGER),
                Route::of(TriggerHolder::SET_TRIGGER_ENABLED),
                Route::of(TriggerHolder::FIRE_TRIGGER_NOW),
                Route::of(TriggerHolder::LIST_PIPELINE_TRIGGERS),
                Route::of(TriggerAccess::CREATE_TRIGGER),
                Route::of(TriggerAccess::GET_TRIGGER),
                Route::of(TriggerAccess::UPDATE_TRIGGER),
                Route::of(TriggerAccess::DELETE_TRIGGER),
                Route::of(TriggerAccess::SET_TRIGGER_ENABLED),
                Route::of(TriggerAccess::FIRE_TRIGGER_NOW),
                Route::of(TriggerAccess::LIST_PIPELINE_TRIGGERS),
            ],
        )],
    ),
    area(
        "jobs",
        &[
            cell(
                "jobs.view",
                "view",
                &["pipelines.view"],
                &[
                    Route::of(PipelineHolder::GET_JOB),
                    Route::of(PipelineHolder::LIST_PIPELINE_JOBS),
                    Route::of(PipelineHolder::LIST_WORKSPACE_JOBS),
                    Route::of(PipelineAccess::GET_JOB),
                    Route::of(PipelineAccess::LIST_PIPELINE_JOBS),
                ],
            ),
            cell(
                "jobs.edit",
                "edit",
                &["jobs.view"],
                &[
                    Route::of(PipelineHolder::CANCEL_JOB),
                    Route::of(PipelineHolder::RERUN_JOB),
                    Route::of(PipelineAccess::CANCEL_JOB),
                    Route::of(PipelineAccess::RERUN_JOB),
                ],
            ),
            cell(
                "jobs.delete",
                "delete",
                &["jobs.view"],
                &[
                    Route::of(PipelineHolder::DELETE_JOB),
                    Route::of(PipelineAccess::DELETE_JOB),
                ],
            ),
        ],
    ),
    area(
        "job_logs",
        &[cell(
            "job_logs.view",
            "view",
            &["jobs.view"],
            &[
                Route::of(PipelineHolder::TAIL_JOB_LOGS),
                Route::of(PipelineAccess::TAIL_JOB_LOGS),
            ],
        )],
    ),
    area(
        "secrets",
        &[
            cell(
                "secrets.view",
                "view",
                &["project.view"],
                &[
                    Route::of(ProjectHolder::LIST_SECRETS),
                    Route::of(Project::LIST_SECRETS),
                ],
            ),
            cell(
                "secrets.create",
                "create",
                &["secrets.view"],
                &[
                    Route::of(ProjectHolder::CREATE_SECRET),
                    Route::of(ProjectHolder::UPDATE_SECRET),
                    Route::of(Project::CREATE_SECRET),
                    Route::of(Project::UPDATE_SECRET),
                ],
            ),
            cell(
                "secrets.delete",
                "delete",
                &["secrets.view"],
                &[
                    Route::of(ProjectHolder::DELETE_SECRET),
                    Route::of(Project::DELETE_SECRET),
                ],
            ),
        ],
    ),
    Area {
        id: "job_execution",
        level: Level::Project,
        machine: true,
        permissions: &[
            cell(
                "job_execution.edit",
                "edit",
                &[],
                &[
                    Route::of(PipelineHolder::RUN_JOBS),
                    Route::of(PipelineAccess::RUN_JOBS),
                ],
            ),
            cell(
                "job_execution.run",
                "run",
                &["job_execution.edit"],
                &[
                    Route::of(PipelineHolder::RUN_JOBS),
                    Route::of(PipelineAccess::RUN_JOBS),
                ],
            ),
        ],
    },
];
