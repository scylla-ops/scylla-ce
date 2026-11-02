//! What the core tells a hook about the write. Use `new()`: the structs are non-exhaustive.

use scylla_domain::identity::actors::Actor;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::ResourceKind;

/// A creation that a `BeforeCreate` hook can refuse.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CreateContext {
    /// The kind of the new resource.
    pub kind: ResourceKind,
    /// The actor that creates the resource.
    pub actor: Actor,
    /// The workspace of the new resource.
    pub workspace_id: WorkspaceId,
}

impl CreateContext {
    /// Makes the context of a creation.
    pub fn new(kind: ResourceKind, actor: Actor, workspace_id: WorkspaceId) -> Self {
        Self {
            kind,
            actor,
            workspace_id,
        }
    }
}

/// A deletion that an `AfterDelete` hook follows.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DeleteContext {
    /// The kind of the deleted resource.
    pub kind: ResourceKind,
    /// The actor that deletes the resource.
    pub actor: Actor,
    /// The workspace of the deleted resource.
    pub workspace_id: WorkspaceId,
}

impl DeleteContext {
    /// Makes the context of a deletion.
    pub fn new(kind: ResourceKind, actor: Actor, workspace_id: WorkspaceId) -> Self {
        Self {
            kind,
            actor,
            workspace_id,
        }
    }
}
