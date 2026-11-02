//! The values that the gate gets: objects, relationships and checks.

use std::fmt;

use super::Resource;
use crate::workspaces::WorkspaceId;
use crate::Id;

/// An object of the permission model, for example `project:<id>`. Only a typed id makes one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Object {
    /// The name of its definition.
    kind: &'static str,
    /// The id, in its text form.
    id: String,
}

impl Object {
    /// Makes the object of a typed id.
    pub fn of<R: Resource>(id: Id<R>) -> Self {
        Self {
            kind: R::NAME,
            id: id.to_string(),
        }
    }

    /// Returns the name of its definition.
    pub fn kind(&self) -> &'static str {
        self.kind
    }

    /// Returns the id, in its text form.
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl fmt::Display for Object {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.kind, self.id)
    }
}

/// A relationship `object#relation@subject`. Only a typed `Relation` makes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Relationship {
    /// The object that has the relation.
    object: Object,
    /// The name of the relation.
    relation: &'static str,
    /// The subject of the relation.
    subject: Object,
}

impl Relationship {
    /// Makes a relationship.
    pub(super) fn new(object: Object, relation: &'static str, subject: Object) -> Self {
        Self {
            object,
            relation,
            subject,
        }
    }

    /// Returns the object that has the relation.
    pub fn object(&self) -> &Object {
        &self.object
    }

    /// Returns the name of the relation.
    pub fn relation(&self) -> &'static str {
        self.relation
    }

    /// Returns the subject of the relation.
    pub fn subject(&self) -> &Object {
        &self.subject
    }
}

impl fmt::Display for Relationship {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}@{}", self.object, self.relation, self.subject)
    }
}

/// A permission that a request needs on one object. Only a typed `Permission` makes one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// The object.
    object: Object,
    /// The name of the permission.
    permission: &'static str,
    /// The workspace of an object that is in a workspace, from Postgres.
    workspace: Option<WorkspaceId>,
    /// A token of a write that the answer must see.
    fresh: Option<String>,
}

impl Check {
    /// Makes a check.
    pub(super) fn new(object: Object, permission: &'static str) -> Self {
        Self {
            object,
            permission,
            workspace: None,
            fresh: None,
        }
    }

    /// Names the workspace of the object. The owners of the workspace, and a role on the
    /// workspace, pass the check: `Project::DELETE_PROJECT.on(id).within(workspace)`.
    #[must_use]
    pub fn within(mut self, workspace: WorkspaceId) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Returns the workspace of the object, if the check names one.
    pub fn workspace(&self) -> Option<WorkspaceId> {
        self.workspace
    }

    /// Returns the object.
    pub fn object(&self) -> &Object {
        &self.object
    }

    /// Returns the name of the permission.
    pub fn permission(&self) -> &'static str {
        self.permission
    }

    /// Asks for an answer that sees at least the write that returned this token.
    #[must_use]
    pub fn at_least_as_fresh(mut self, token: impl Into<String>) -> Self {
        self.fresh = Some(token.into());
        self
    }

    /// Returns the token of `at_least_as_fresh`, if there is one.
    pub fn freshness(&self) -> Option<&str> {
        self.fresh.as_deref()
    }
}

impl fmt::Display for Check {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} on {}", self.permission, self.object)
    }
}
