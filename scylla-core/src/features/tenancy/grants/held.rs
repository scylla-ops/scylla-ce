//! Who holds which role in a workspace or on a project: the owners, the instance admins, the roles
//! of the schema from the gate, and the roles that are rows.

use std::collections::HashMap;

use scylla_domain::authz::{Object, Resource};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::Platform;
use scylla_domain::projects::{Project, ProjectId};
use scylla_domain::tenancy::catalog::Level;
use scylla_domain::tenancy::grants::{AccessEntry, GrantScope, HeldRole, Member};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{Gate, Tx};
use sqlx::PgPool;
use uuid::Uuid;

use super::held_values::{admin_role, owner_role, relation_role, row_role, scope_of};
use super::store;
use crate::Error;

/// The objects of a workspace with their names.
pub(crate) struct Scopes {
    /// The workspace.
    pub(crate) workspace: GrantScope,
    /// The projects.
    pub(crate) projects: Vec<GrantScope>,
}

impl Scopes {
    /// Returns the ids of the scopes, for the SQL.
    fn ids(&self) -> Vec<Uuid> {
        std::iter::once(&self.workspace)
            .chain(&self.projects)
            .filter_map(|scope| Uuid::parse_str(&scope.id).ok())
            .collect()
    }
}

/// Makes a scope.
fn scope(level: Level, id: String, name: String) -> GrantScope {
    GrantScope { level, id, name }
}

/// Reads a workspace and its projects. An unknown workspace has no name and no project.
pub(crate) async fn scopes(tx: &mut Tx<'_>, workspace: WorkspaceId) -> Result<Scopes, Error> {
    let name = store::workspace_name(tx, workspace).await?;
    let projects = store::projects_of(tx, workspace).await?;
    Ok(Scopes {
        workspace: scope(Level::Workspace, workspace.to_string(), name),
        projects: projects
            .into_iter()
            .map(|(id, name)| scope(Level::Project, id.to_string(), name))
            .collect(),
    })
}

/// Returns the roles on a workspace and on its projects, with the owners. The instance admins are
/// not in the answer.
pub(crate) async fn in_workspace(
    pool: &PgPool,
    gate: &dyn Gate,
    workspace: WorkspaceId,
) -> Result<Vec<HeldRole>, Error> {
    read(pool, gate, workspace, None, true).await
}

/// Returns the roles on one project. With `with_workspace`, also the roles on its workspace, with
/// the owners.
pub(crate) async fn on_project(
    pool: &PgPool,
    gate: &dyn Gate,
    workspace: WorkspaceId,
    project: ProjectId,
    with_workspace: bool,
) -> Result<Vec<HeldRole>, Error> {
    read(pool, gate, workspace, Some(project), with_workspace).await
}

/// Reads the roles of the scopes.
async fn read(
    pool: &PgPool,
    gate: &dyn Gate,
    workspace: WorkspaceId,
    only: Option<ProjectId>,
    with_workspace: bool,
) -> Result<Vec<HeldRole>, Error> {
    // Read the scopes, and what Postgres knows about them.
    let mut tx = pool.begin().await?;
    let mut scopes = scopes(&mut tx, workspace).await?;
    if let Some(project) = only {
        let id = project.to_string();
        scopes.projects.retain(|scope| scope.id == id);
    }
    let ids = scopes.ids();
    let owners = store::owners_since(&mut tx, workspace).await?;
    let rows = store::rows_in(&mut tx, &ids).await?;
    let times = store::relation_times(&mut tx, &ids).await?;
    tx.commit().await?;

    // Add the owners and the roles that are rows.
    let known: Vec<&GrantScope> = std::iter::once(&scopes.workspace)
        .filter(|_| with_workspace)
        .chain(&scopes.projects)
        .collect();
    let mut held = Vec::new();
    if with_workspace {
        held.extend(
            owners
                .into_iter()
                .map(|(actor, since)| owner_role(&scopes.workspace, actor, since)),
        );
    }
    for grant in rows {
        if let Some(scope) = scope_of(&known, &grant) {
            held.push(row_role(grant, scope));
        }
    }

    // Add the roles of the schema.
    if with_workspace {
        for holder in gate.all_roles(&Object::of(workspace)).await? {
            let role = relation_role(&scopes.workspace, holder.actor, holder.role, &times);
            held.push(role);
        }
    }
    for scope in &scopes.projects {
        let Ok(project) = ProjectId::parse(&scope.id) else {
            continue;
        };
        for holder in gate.all_roles(&Object::of(project)).await? {
            held.push(relation_role(scope, holder.actor, holder.role, &times));
        }
    }
    Ok(held)
}

/// Returns the instance admins.
pub(crate) async fn instance_admins(pool: &PgPool) -> Result<Vec<HeldRole>, Error> {
    let mut tx = pool.begin().await?;
    let admins = store::admins_since(&mut tx).await?;
    tx.commit().await?;
    Ok(admins
        .into_iter()
        .map(|(actor, since)| admin_role(actor, since))
        .collect())
}

/// Returns each role of one actor in a workspace and on the instance, with no lists of the others.
pub(crate) async fn of_actor(
    pool: &PgPool,
    gate: &dyn Gate,
    actor: ActorId,
    workspace: WorkspaceId,
) -> Result<Vec<HeldRole>, Error> {
    // Read the scopes, and what Postgres knows about the actor.
    let mut tx = pool.begin().await?;
    let scopes = scopes(&mut tx, workspace).await?;
    let ids = scopes.ids();
    let owner_since = store::owner_since(&mut tx, workspace, actor).await?;
    let admin_since = store::admin_since(&mut tx, actor).await?;
    let rows = store::rows_of(&mut tx, actor, &ids).await?;
    let on_platform = store::platform_rows_of(&mut tx, actor).await?;
    let times = store::relation_times(&mut tx, &ids).await?;
    tx.commit().await?;

    // Add the roles of the instance.
    let platform = scope(Level::Platform, Platform::ID.to_string(), String::new());
    let mut held = Vec::new();
    if let Some(since) = admin_since {
        held.push(admin_role(actor, since));
    }
    held.extend(
        on_platform
            .into_iter()
            .map(|grant| row_role(grant, &platform)),
    );

    // Add the owner and the roles on the workspace and on its projects.
    if let Some(since) = owner_since {
        held.push(owner_role(&scopes.workspace, actor, since));
    }
    let known: Vec<&GrantScope> = std::iter::once(&scopes.workspace)
        .chain(&scopes.projects)
        .collect();
    for grant in rows {
        if let Some(scope) = scope_of(&known, &grant) {
            held.push(row_role(grant, scope));
        }
    }
    let on_workspace = gate.roles_of(actor, Workspace::NAME).await?;
    for role in on_workspace {
        if role.object == scopes.workspace.id {
            let role = relation_role(&scopes.workspace, actor, role.role, &times);
            held.push(role);
        }
    }
    let on_projects = gate.roles_of(actor, Project::NAME).await?;
    for role in on_projects {
        if let Some(scope) = scopes.projects.iter().find(|scope| scope.id == role.object) {
            held.push(relation_role(scope, actor, role.role, &times));
        }
    }
    Ok(held)
}

/// Puts the roles in members, in name order, with the profiles of their actors.
pub(crate) async fn members(pool: &PgPool, held: Vec<HeldRole>) -> Result<Vec<Member>, Error> {
    let mut by_actor: HashMap<ActorId, Vec<HeldRole>> = HashMap::new();
    for role in held {
        by_actor.entry(role.actor).or_default().push(role);
    }
    let ids: Vec<ActorId> = by_actor.keys().copied().collect();
    let mut tx = pool.begin().await?;
    let profiles: Vec<AccessEntry> = store::profiles(&mut tx, &ids).await?;
    tx.commit().await?;
    let mut members: Vec<Member> = profiles
        .into_iter()
        .filter_map(|profile| {
            let roles = by_actor.remove(&profile.actor_id)?;
            Some(Member { profile, roles })
        })
        .collect();
    members.sort_by_cached_key(|member| {
        (
            member.profile.display_name.to_lowercase(),
            member.profile.actor_id.to_string(),
        )
    });
    Ok(members)
}
