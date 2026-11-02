//! The roles that `AccessService` can give: for each kind of object, its roles, the checks of
//! the routes `Grant` and `Revoke`, and the search for the workspace of an object.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

use scylla_domain::actors::{Actor, ActorId};
use scylla_domain::authz::{Check, Permission, Relation, Relationship, Resource};
use scylla_domain::workspaces::WorkspaceId;
use scylla_domain::Id;
use sqlx::PgPool;

/// The answer of a search for a workspace: the workspace, or `None` if no object has this id.
pub type WorkspaceFuture =
    Pin<Box<dyn Future<Output = Result<Option<WorkspaceId>, sqlx::Error>> + Send>>;

/// Finds the workspace of an object of `R` in Postgres.
pub type FindWorkspace<R> = fn(PgPool, Id<R>) -> WorkspaceFuture;

/// Makes a check of a route on an object, from the text of its id.
type MakeCheck = Box<dyn Fn(&str) -> Option<Check> + Send + Sync>;

/// Starts the search for the workspace of an object, from the text of its id.
type StartSearch = Box<dyn Fn(PgPool, &str) -> Option<WorkspaceFuture> + Send + Sync>;

/// Makes the relationship of a role, from the text of the object id and the actor.
type MakeRole = Box<dyn Fn(&str, ActorId) -> Option<Relationship> + Send + Sync>;

/// The roles of one kind of object.
pub struct Grantable {
    /// The check of the route `Grant`.
    grant: MakeCheck,
    /// The check of the route `Revoke`.
    revoke: MakeCheck,
    /// The search for the workspace. A workspace needs none: it is the top.
    workspace: Option<StartSearch>,
    /// The roles, by name.
    roles: BTreeMap<&'static str, MakeRole>,
}

impl Grantable {
    /// Makes the roles of `R`, with no role yet.
    fn new<R: Resource>(
        grant: Permission<R>,
        revoke: Permission<R>,
        workspace: Option<FindWorkspace<R>>,
    ) -> Self {
        let search = workspace.map(|find| -> StartSearch {
            Box::new(move |pool, id| Id::<R>::parse(id).ok().map(|id| find(pool, id)))
        });
        Self {
            grant: Box::new(move |id| Id::<R>::parse(id).ok().map(|id| grant.on(id))),
            revoke: Box::new(move |id| Id::<R>::parse(id).ok().map(|id| revoke.on(id))),
            workspace: search,
            roles: BTreeMap::new(),
        }
    }

    /// Returns the check of the route `Grant` on an object, or `None` for a bad id.
    pub fn grant(&self, id: &str) -> Option<Check> {
        (self.grant)(id)
    }

    /// Returns the check of the route `Revoke` on an object, or `None` for a bad id.
    pub fn revoke(&self, id: &str) -> Option<Check> {
        (self.revoke)(id)
    }

    /// Starts the search for the workspace of an object. `None` means that the kind needs none.
    pub fn workspace(&self, pool: PgPool, id: &str) -> Option<WorkspaceFuture> {
        self.workspace.as_ref().and_then(|search| search(pool, id))
    }

    /// Returns the relationship of a role, or `None` for an unknown role or a bad id.
    pub fn relationship(&self, role: &str, id: &str, actor: ActorId) -> Option<Relationship> {
        self.roles.get(role).and_then(|make| make(id, actor))
    }
}

/// The kinds of objects that have roles, by the name of their definition.
#[derive(Default)]
pub struct Grants {
    /// The roles of each kind.
    kinds: BTreeMap<&'static str, Grantable>,
}

impl Grants {
    /// Returns the roles of a kind, if it has roles.
    pub fn get(&self, kind: &str) -> Option<&Grantable> {
        self.kinds.get(kind)
    }

    /// Adds a kind. Returns `false` if the kind is there already.
    pub(crate) fn add_kind<R: Resource>(
        &mut self,
        grant: Permission<R>,
        revoke: Permission<R>,
        workspace: Option<FindWorkspace<R>>,
    ) -> bool {
        if self.kinds.contains_key(R::NAME) {
            return false;
        }
        let grantable = Grantable::new(grant, revoke, workspace);
        self.kinds.insert(R::NAME, grantable);
        true
    }

    /// Adds a role to its kind. Returns `false` if the kind is not there or has the role already.
    pub(crate) fn add_role<R: Resource>(&mut self, role: Relation<R, Actor>) -> bool {
        let Some(kind) = self.kinds.get_mut(R::NAME) else {
            return false;
        };
        if kind.roles.contains_key(role.name()) {
            return false;
        }
        let make: MakeRole =
            Box::new(move |id, actor| Id::<R>::parse(id).ok().map(|id| role.of(id, actor)));
        kind.roles.insert(role.name(), make);
        true
    }
}
