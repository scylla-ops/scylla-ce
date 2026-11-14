//! The link of a project to its repository: at most one repository for each project, through a
//! connection of its workspace.

use time::OffsetDateTime;

use super::connections::{ConnectionState, DeliveryMode, VcsConnectionId};
use super::named;
use crate::identity::actors::ActorId;
use crate::projects::{Project, ProjectId};
use crate::tenancy::workspaces::{Workspace, WorkspaceId};

named! {
    /// Where the pipelines of a project, or one pipeline, come from.
    pub enum PipelineSource {
        /// The editor of Scylla.
        Editor = "editor",
        /// A file in `.scylla/` of the linked repository.
        File = "file",
    }
}

named! {
    /// The role of a GitLab service account on a project.
    pub enum GitLabRole {
        /// It can report, but not make the webhook or the merge rule.
        Developer = "developer",
        /// It can do each part of the integration.
        Maintainer = "maintainer",
        /// It can do each part of the integration.
        Owner = "owner",
    }
}

/// The link of a project, as the store returns it. No field holds a secret.
#[derive(Clone, Debug)]
pub struct RepositoryLink {
    /// The project.
    pub project_id: ProjectId,
    /// The workspace of the project.
    pub workspace_id: WorkspaceId,
    /// The connection that Scylla uses for the repository.
    pub connection_id: VcsConnectionId,
    /// The id of the repository on the provider.
    pub external_id: String,
    /// The full name, for example `acme/api`.
    pub full_name: String,
    /// The page of the repository on the provider.
    pub web_url: String,
    /// The default branch.
    pub default_branch: String,
    /// `true` for a private or internal repository.
    pub private: bool,
    /// The role of the service account on a GitLab project, or `None` on GitHub.
    pub role: Option<GitLabRole>,
    /// Where the pipelines of the project come from.
    pub pipeline_source: PipelineSource,
    /// `true` when Scylla reports the results to the provider.
    pub reports_enabled: bool,
    /// How the provider tells Scylla about the new commits.
    pub delivery_mode: DeliveryMode,
    /// The time between two polls in polling mode, in seconds.
    pub poll_interval_seconds: u32,
    /// The time of the last webhook of the repository.
    pub last_event_at: Option<OffsetDateTime>,
    /// The state.
    pub state: ConnectionState,
    /// The reason of a state that is not active, as a stable code.
    pub state_reason: Option<String>,
    /// The actor that made the link.
    pub created_by: ActorId,
    /// The creation time.
    pub created_at: OffsetDateTime,
}

crate::authz! {
    /// The routes of the repository of a project and of its merge rules, on the project.
    extend Project as RepositoryAccess {
        /// The route `GetRepositoryLink`, on this project.
        permission get_repository_link = view;
        /// The route `ListRefs`, on this project.
        permission list_refs = view;
        /// The route `ListMergeBlocks`, on this project.
        permission list_merge_blocks = view;
        /// The route `ListConnectionRepositories`, for a link of this project.
        permission list_connection_repositories = edit;
        /// The route `LinkRepository`, on this project.
        permission link_repository = edit;
        /// The route `UpdateRepositoryLink`, on this project.
        permission update_repository_link = edit;
        /// The route `UnlinkRepository`, on this project.
        permission unlink_repository = edit;
        /// The route `TestDelivery`, on this project.
        permission test_delivery = edit;
        /// The route `SetMergeBlock`, on this project.
        permission set_merge_block = edit;
    }

    /// The routes of the repository of a project and of its merge rules, on its workspace.
    extend Workspace as RepositoryHolder {
        /// The route `GetRepositoryLink`, on each project of this workspace.
        permission get_repository_link = read;
        /// The route `ListRefs`, on each project of this workspace.
        permission list_refs = read;
        /// The route `ListMergeBlocks`, on each project of this workspace.
        permission list_merge_blocks = read;
        /// The route `ListConnectionRepositories`, for a link of each project of this workspace or
        /// of a project to create in it.
        permission list_connection_repositories = work;
        /// The route `LinkRepository`, on each project of this workspace.
        permission link_repository = work;
        /// The route `UpdateRepositoryLink`, on each project of this workspace.
        permission update_repository_link = work;
        /// The route `UnlinkRepository`, on each project of this workspace.
        permission unlink_repository = work;
        /// The route `TestDelivery`, on each project of this workspace.
        permission test_delivery = work;
        /// The route `SetMergeBlock`, on each project of this workspace.
        permission set_merge_block = work;
    }
}
