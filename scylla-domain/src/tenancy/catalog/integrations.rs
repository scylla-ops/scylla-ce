//! The areas of the catalog for the Git integration: the GitHub App of the instance, the
//! connections of a workspace, and the repository of a project. Also the approval of the jobs
//! from a fork, in the area of the jobs.

use super::{Area, CatalogPermission, Level, Route};
use crate::integrations::apps::VcsAppHolder;
use crate::integrations::connections::VcsConnectionHolder;
use crate::integrations::links::{RepositoryAccess, RepositoryHolder};
use crate::projects::pipelines::{PipelineAccess, PipelineHolder};

/// Makes one permission of an area.
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

/// The areas on the instance.
pub(super) const INSTANCE: &[Area] = &[Area {
    id: "git_providers",
    level: Level::Platform,
    machine: false,
    permissions: &[cell(
        "git_providers.manage",
        "manage",
        &[],
        &[
            Route::of(VcsAppHolder::GET_VCS_APP),
            Route::of(VcsAppHolder::START_GIT_HUB_MANIFEST),
            Route::of(VcsAppHolder::COMPLETE_GIT_HUB_MANIFEST),
            Route::of(VcsAppHolder::DELETE_VCS_APP),
        ],
    )],
}];

/// The areas in a workspace.
pub(super) const WORKSPACE: &[Area] = &[Area {
    id: "integrations",
    level: Level::Workspace,
    machine: false,
    permissions: &[
        cell(
            "integrations.view",
            "view",
            &["workspace.view"],
            &[
                Route::of(VcsConnectionHolder::GET_VCS_PROVIDERS),
                Route::of(VcsConnectionHolder::LIST_VCS_CONNECTIONS),
                Route::of(VcsConnectionHolder::GET_VCS_CONNECTION),
            ],
        ),
        cell(
            "integrations.manage",
            "manage",
            &["integrations.view"],
            &[
                Route::of(VcsConnectionHolder::START_GIT_HUB_INSTALL),
                Route::of(VcsConnectionHolder::COMPLETE_GIT_HUB_INSTALL),
                Route::of(VcsConnectionHolder::CONNECT_GIT_LAB),
                Route::of(VcsConnectionHolder::REPLACE_GIT_LAB_TOKEN),
                Route::of(VcsConnectionHolder::DELETE_VCS_CONNECTION),
            ],
        ),
    ],
}];

/// The areas in a project. The edit of the repository also gives the list of the connections of
/// the workspace, for the link picker.
pub(super) const PROJECT: &[Area] = &[Area {
    id: "repository",
    level: Level::Project,
    machine: false,
    permissions: &[
        cell(
            "repository.view",
            "view",
            &["project.view"],
            &[
                Route::of(RepositoryHolder::GET_REPOSITORY_LINK),
                Route::of(RepositoryHolder::LIST_REFS),
                Route::of(RepositoryHolder::LIST_MERGE_BLOCKS),
                Route::of(RepositoryAccess::GET_REPOSITORY_LINK),
                Route::of(RepositoryAccess::LIST_REFS),
                Route::of(RepositoryAccess::LIST_MERGE_BLOCKS),
            ],
        ),
        cell(
            "repository.edit",
            "edit",
            &["repository.view", "pipelines.view"],
            &[
                Route::of(VcsConnectionHolder::LIST_VCS_CONNECTIONS),
                Route::of(RepositoryHolder::LIST_CONNECTION_REPOSITORIES),
                Route::of(RepositoryHolder::LINK_REPOSITORY),
                Route::of(RepositoryHolder::UPDATE_REPOSITORY_LINK),
                Route::of(RepositoryHolder::UNLINK_REPOSITORY),
                Route::of(RepositoryHolder::TEST_DELIVERY),
                Route::of(RepositoryHolder::SET_MERGE_BLOCK),
                Route::of(PipelineHolder::SET_PIPELINE_SOURCE),
                Route::of(RepositoryAccess::LIST_CONNECTION_REPOSITORIES),
                Route::of(RepositoryAccess::LINK_REPOSITORY),
                Route::of(RepositoryAccess::UPDATE_REPOSITORY_LINK),
                Route::of(RepositoryAccess::UNLINK_REPOSITORY),
                Route::of(RepositoryAccess::TEST_DELIVERY),
                Route::of(RepositoryAccess::SET_MERGE_BLOCK),
                Route::of(PipelineAccess::SET_PIPELINE_SOURCE),
            ],
        ),
    ],
}];

/// The approval of a job from a fork: a cell of the area of the jobs.
pub(super) const JOBS_APPROVE: CatalogPermission = cell(
    "jobs.approve",
    "approve",
    &["jobs.view"],
    &[
        Route::of(PipelineHolder::APPROVE_JOB),
        Route::of(PipelineHolder::REJECT_JOB),
        Route::of(PipelineAccess::APPROVE_JOB),
        Route::of(PipelineAccess::REJECT_JOB),
    ],
);
