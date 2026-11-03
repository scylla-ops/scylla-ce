//! What the core tells a hook about the write. Use `new()`: the structs are non-exhaustive.

use scylla_domain::identity::actors::{Actor, ActorId};
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
    /// The member of a membership: the user that joins the workspace.
    pub subject: Option<ActorId>,
    /// The object of the role of a membership, as `kind:id`. `None` for an owner.
    pub object: Option<String>,
    /// The role of a membership, or `owner`.
    pub role: Option<String>,
}

impl CreateContext {
    /// Makes the context of a creation.
    pub fn new(kind: ResourceKind, actor: Actor, workspace_id: WorkspaceId) -> Self {
        Self {
            kind,
            actor,
            workspace_id,
            subject: None,
            object: None,
            role: None,
        }
    }

    /// Names the member, the object and the role of a membership.
    pub fn with_member(mut self, subject: ActorId, object: Option<String>, role: &str) -> Self {
        self.subject = Some(subject);
        self.object = object;
        self.role = Some(role.to_owned());
        self
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
    /// The member of a membership that ends.
    pub subject: Option<ActorId>,
    /// The deleted object as `kind:id`, or the object of the role of a membership that ends.
    pub object: Option<String>,
    /// The role of a membership that ends.
    pub role: Option<String>,
}

impl DeleteContext {
    /// Makes the context of a deletion.
    pub fn new(kind: ResourceKind, actor: Actor, workspace_id: WorkspaceId) -> Self {
        Self {
            kind,
            actor,
            workspace_id,
            subject: None,
            object: None,
            role: None,
        }
    }

    /// Names the deleted object, as `kind:id`.
    pub fn with_object(mut self, object: String) -> Self {
        self.object = Some(object);
        self
    }

    /// Names the member, the object and the role of a membership that ends.
    pub fn with_member(mut self, subject: ActorId, object: String, role: &str) -> Self {
        self.subject = Some(subject);
        self.object = Some(object);
        self.role = Some(role.to_owned());
        self
    }
}
