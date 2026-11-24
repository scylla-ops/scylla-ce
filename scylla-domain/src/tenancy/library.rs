//! The library of a workspace: the fragments that its pipelines share, their versions, and the
//! routes on them. The checks of a request are in `checks`.

pub mod checks;

use std::collections::BTreeMap;

use time::OffsetDateTime;

use crate::identity::actors::{Actor, ActorId};
use crate::projects::pipelines::{Node, NodeId, PipelineId};
use crate::projects::ProjectId;
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{Id, Page, PageRequest, ResourceKind};

/// The kind of a fragment, for the hooks.
pub const KIND: ResourceKind = ResourceKind::new("fragment");

/// The kind of a version of a fragment, for the hooks. The context names the fragment as
/// `fragment:<id>`.
pub const VERSION_KIND: ResourceKind = ResourceKind::new("fragment_version");

/// The id of a fragment.
pub type FragmentId = Id<Fragment>;

/// A fragment of the library, with its versions, the oldest first.
#[derive(Clone, Debug)]
pub struct Fragment {
    /// The id.
    pub id: FragmentId,
    /// The workspace of the fragment.
    pub workspace_id: WorkspaceId,
    /// The actor that created the fragment.
    pub author: Author,
    /// The name: the id of the block in a pipeline.
    pub name: NodeId,
    /// The description.
    pub description: String,
    /// The tags.
    pub tags: Vec<String>,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The versions, the oldest first.
    pub versions: Vec<Version>,
    /// `true` if the reader marked the fragment as a favorite.
    pub favorite: bool,
    /// The active pipelines that link the fragment.
    pub usages: Vec<Usage>,
    /// The package of an outside registry that the fragment comes from.
    pub origin: Option<Origin>,
}

impl Fragment {
    /// Returns `true` if the fragment takes its versions from its package only.
    pub fn is_linked(&self) -> bool {
        self.origin.as_ref().is_some_and(|origin| origin.linked)
    }
}

/// Where a fragment comes from: a package of an outside registry. The server never calls the
/// registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    /// The URL of the registry.
    pub registry: String,
    /// The id of the package in the registry.
    pub package_id: String,
    /// The name of the package to show.
    pub package_name: String,
    /// `true`: the versions come from the package only. `false`: a fork.
    pub linked: bool,
    /// The version of the package that the fragment started from, or that a fork copies.
    pub base_version: u32,
    /// The versions of the package that a linked fragment chose not to take.
    pub skipped: Vec<u32>,
    /// The version of the package that a fork chose not to see.
    pub dismissed: Option<u32>,
    /// The time of the add.
    pub added_at: OffsetDateTime,
}

impl Fragment {
    /// Returns `true` if the actor created the fragment: its author routes pass on the actor.
    pub fn is_created_by(&self, actor: ActorId) -> bool {
        self.author.id == actor
    }
}

/// The actor that made a fragment or a version, with the display name of a user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Author {
    /// The actor.
    pub id: ActorId,
    /// The display name of the user, or empty for an actor that is not a user.
    pub name: String,
}

/// A version of a fragment. It does not change after its creation.
#[derive(Clone, Debug)]
pub struct Version {
    /// The number, 1 or more.
    pub number: u32,
    /// The nodes. Empty when the read did not ask for them.
    pub nodes: Vec<Node>,
    /// The parameters.
    pub parameters: Vec<Parameter>,
    /// What changed, one line each.
    pub changes: Vec<String>,
    /// The actor that made the version.
    pub author: Author,
    /// The time of the version.
    pub published_at: OffsetDateTime,
}

/// A value that each pipeline gives to a fragment: an environment variable of its nodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parameter {
    /// The name of the environment variable.
    pub key: String,
    /// A literal with its default, or a secret.
    pub value: ParameterValue,
    /// What the parameter is for.
    pub description: String,
}

/// The kind of a parameter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParameterValue {
    /// A text in clear, with its default value.
    Literal(String),
    /// The name of a secret of the project of the pipeline. The server never holds its value.
    Secret,
}

/// An active pipeline that links a fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The display name of the pipeline.
    pub pipeline_name: String,
    /// The project of the pipeline.
    pub project_id: ProjectId,
    /// The display name of the project.
    pub project_name: String,
    /// The version that the pipeline links.
    pub version: u32,
    /// The values of the parameters, by key.
    pub values: BTreeMap<String, String>,
}

/// The checked content of a new version.
#[derive(Clone, Debug)]
pub struct NewVersion {
    /// The nodes.
    pub nodes: Vec<Node>,
    /// The parameters.
    pub parameters: Vec<Parameter>,
    /// What changed.
    pub changes: Vec<String>,
}

/// The input of a fragment creation, with its first version.
#[derive(Clone, Debug)]
pub struct NewFragment {
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// The actor that creates the fragment.
    pub created_by: ActorId,
    /// The name.
    pub name: NodeId,
    /// The description.
    pub description: String,
    /// The tags.
    pub tags: Vec<String>,
    /// The package that the fragment comes from: its first version keeps the number of the
    /// package. `None`: the first version is 1.
    pub origin: Option<NewOrigin>,
    /// The first version.
    pub version: NewVersion,
}

/// The origin of a new fragment, checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewOrigin {
    /// The URL of the registry.
    pub registry: String,
    /// The id of the package in the registry.
    pub package_id: String,
    /// The name of the package to show.
    pub package_name: String,
    /// The version of the package that the fragment holds.
    pub version: u32,
    /// `true` for a linked fragment, `false` for a fork.
    pub linked: bool,
}

impl NewFragment {
    /// Returns the number of the first version.
    pub fn first_number(&self) -> u32 {
        self.origin.as_ref().map_or(1, |origin| origin.version)
    }
}

/// The input of a list of the fragments of a workspace.
#[derive(Clone, Copy, Debug)]
pub struct ListFragments {
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// The reader: its favorites are in the list.
    pub reader: ActorId,
    /// The page to return.
    pub page: PageRequest<FragmentId>,
}

/// One page of fragments.
pub type FragmentPage = Page<Fragment, FragmentId>;

crate::authz! {
    /// The routes of the library on its workspace.
    extend Workspace as FragmentHolder {
        /// The route `ListFragments`, in this workspace.
        permission list_fragments = read;
        /// The route `GetFragment`, on each fragment of this workspace.
        permission get_fragment = read;
        /// The route `CreateFragment`, in this workspace.
        permission create_fragment = work;
        /// The route `SetFragmentFavorite`, on each fragment of this workspace.
        permission set_fragment_favorite = read;
        /// The route `PublishFragmentVersion`, on each fragment of this workspace.
        permission publish_fragment_version = manage;
        /// The route `DeleteFragment`, on each fragment of this workspace.
        permission delete_fragment = manage;
        /// The route `AcceptOriginVersion`, on each linked fragment of this workspace.
        permission accept_origin_version = manage;
        /// The route `SkipOriginVersion`, on each linked fragment of this workspace.
        permission skip_origin_version = manage;
        /// The route `DismissOriginVersion`, on each fork of this workspace.
        permission dismiss_origin_version = manage;
        /// The route `DetachOrigin`, on each linked fragment of this workspace.
        permission detach_origin = manage;
    }

    /// The routes of a fragment on its author: the author versions and deletes it while it reads
    /// the library.
    extend Actor as FragmentCaller {
        /// The route `PublishFragmentVersion`, on a fragment that the caller created.
        permission publish_fragment_version = nil;
        /// The route `DeleteFragment`, on a fragment that the caller created.
        permission delete_fragment = nil;
    }
}
