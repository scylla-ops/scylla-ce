//! What the core tells a hook about the write. Use `new()`: the structs are non-exhaustive.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::ResourceKind;
use serde_json::Value;

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
    /// The object that the new resource goes in, or the object of the role of a membership, as
    /// `kind:id`. `None` for the role owner.
    pub object: Option<String>,
    /// The role of a membership.
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

    /// Names the object that the new resource goes in, as `kind:id`.
    pub fn with_object(mut self, object: String) -> Self {
        self.object = Some(object);
        self
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

/// A user that a hook of a user follows: its deletion, or the export of its data.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct UserContext {
    /// The user.
    pub user: ActorId,
    /// The actor that acts: the user itself, or an instance admin.
    pub actor: ActorId,
}

impl UserContext {
    /// Makes the context of a user.
    pub fn new(user: ActorId, actor: ActorId) -> Self {
        Self { user, actor }
    }
}

/// The part of an edition in the export of the data of a user.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ExportPart {
    /// The key of the part in the export: lower snake case, unique in the edition.
    pub name: &'static str,
    /// The data of the part.
    pub data: Value,
}

impl ExportPart {
    /// Makes a part of the export.
    pub fn new(name: &'static str, data: Value) -> Self {
        Self { name, data }
    }
}
