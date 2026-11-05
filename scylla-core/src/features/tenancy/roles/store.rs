//! The SQL of the custom roles, their permissions and their grants. Each function runs in
//! the transaction of the caller.

use scylla_domain::authz::Resource;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::catalog::Level;
use scylla_domain::tenancy::roles::{Holder, NewRole, RoleId, RoleRow};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;

use crate::features::identity::accounts::refused;
use crate::Error;

/// The unique index of the name of a role for its owner.
const NAME_KEY: &str = "roles_owner_name";

/// A row of the roles, with its permissions and the number of its grants.
struct Row {
    /// The id.
    id: uuid::Uuid,
    /// The workspace that owns the role.
    workspace_id: Option<uuid::Uuid>,
    /// The name.
    name: String,
    /// What the role does.
    description: String,
    /// The level.
    level: String,
    /// Who can hold the role.
    holder: String,
    /// The ids of the permissions.
    permissions: Vec<String>,
    /// The number of grants.
    holder_count: i64,
}

impl Row {
    /// Makes the domain value. The checks of the table give a known level and known holders.
    fn into_role(self) -> RoleRow {
        RoleRow {
            id: RoleId::from_db(self.id),
            workspace: self.workspace_id.map(WorkspaceId::from_db),
            name: self.name,
            description: self.description,
            level: Level::parse(&self.level).unwrap_or(Level::Project),
            holder: Holder::parse(&self.holder).unwrap_or(Holder::People),
            permissions: self.permissions,
            holder_count: u32::try_from(self.holder_count).unwrap_or(u32::MAX),
        }
    }
}

/// The error of a name that the owner already uses.
fn name_in_use() -> Error {
    refused("ROLE_NAME_IN_USE", "the owner has a role with this name")
}

/// Adds a role and its permissions.
pub(super) async fn insert(tx: &mut Tx<'_>, role: &NewRole) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO roles (id, workspace_id, level, holder, name, description, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
        role.id.to_db(),
        role.workspace.map(WorkspaceId::to_db),
        role.level.as_str(),
        role.holder.as_str(),
        role.name.as_str(),
        role.description,
        role.created_by.to_db(),
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(NAME_KEY) => name_in_use(),
        error => error.into(),
    })?;
    replace_permissions(tx, role.id, &role.permissions).await
}

/// Replaces the permissions of a role.
async fn replace_permissions(
    tx: &mut Tx<'_>,
    id: RoleId,
    permissions: &[&'static str],
) -> Result<(), Error> {
    let permissions: Vec<String> = permissions.iter().map(|id| (*id).to_owned()).collect();
    sqlx::query!(
        "DELETE FROM role_permissions WHERE role_id = $1",
        id.to_db()
    )
    .execute(&mut **tx)
    .await?;
    sqlx::query!(
        "INSERT INTO role_permissions (role_id, permission)
         SELECT $1, permission FROM unnest($2::text[]) AS permission",
        id.to_db(),
        &permissions,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns a role, or `None` for an unknown id.
pub(super) async fn find(tx: &mut Tx<'_>, id: RoleId) -> Result<Option<RoleRow>, Error> {
    let row = sqlx::query_as!(
        Row,
        r#"SELECT r.id, r.workspace_id, r.name, r.description, r.level, r.holder,
                  coalesce((SELECT array_agg(p.permission) FROM role_permissions p
                            WHERE p.role_id = r.id), '{}') AS "permissions!",
                  (SELECT count(*) FROM role_grants g WHERE g.role_id = r.id) AS "holder_count!"
           FROM roles r WHERE r.id = $1"#,
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_role))
}

/// Returns the custom roles of the instance, or the custom roles for a workspace: those of the
/// workspace, and those of the lower levels that the instance owns. In name order.
pub(super) async fn list_custom(
    tx: &mut Tx<'_>,
    workspace: Option<WorkspaceId>,
) -> Result<Vec<RoleRow>, Error> {
    let rows = sqlx::query_as!(
        Row,
        r#"SELECT r.id, r.workspace_id, r.name, r.description, r.level, r.holder,
                  coalesce((SELECT array_agg(p.permission) FROM role_permissions p
                            WHERE p.role_id = r.id), '{}') AS "permissions!",
                  (SELECT count(*) FROM role_grants g WHERE g.role_id = r.id) AS "holder_count!"
           FROM roles r
           WHERE ($1::uuid IS NULL AND r.workspace_id IS NULL)
              OR r.workspace_id = $1
              OR ($1::uuid IS NOT NULL AND r.workspace_id IS NULL AND r.level <> 'platform')
           ORDER BY lower(r.name)"#,
        workspace.map(WorkspaceId::to_db),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_role).collect())
}

/// Changes the name, the description and the permissions of a role. Returns `false` for an
/// unknown id.
pub(super) async fn update(
    tx: &mut Tx<'_>,
    id: RoleId,
    (name, description): (&str, &str),
    permissions: &[&'static str],
) -> Result<bool, Error> {
    let changed = sqlx::query!(
        "UPDATE roles SET name = $2, description = $3, updated_at = now() WHERE id = $1",
        id.to_db(),
        name,
        description,
    )
    .execute(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(NAME_KEY) => name_in_use(),
        error => error.into(),
    })?;
    if changed.rows_affected() == 0 {
        return Ok(false);
    }
    replace_permissions(tx, id, permissions).await?;
    Ok(true)
}

/// Deletes a role with no grant. Returns `false` if the role has a grant or does not exist.
pub(super) async fn delete_unused(tx: &mut Tx<'_>, id: RoleId) -> Result<bool, Error> {
    let deleted = sqlx::query!(
        "DELETE FROM roles r WHERE r.id = $1
         AND NOT EXISTS (SELECT 1 FROM role_grants g WHERE g.role_id = r.id)",
        id.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(deleted.rows_affected() > 0)
}

/// Returns the number of instance admins.
pub(super) async fn admin_count(tx: &mut Tx<'_>) -> Result<u32, Error> {
    let count = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM instance_admins"#)
        .fetch_one(&mut **tx)
        .await?;
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

/// Gives a role on the instance to an actor. A role that the actor has stays.
pub(super) async fn grant_on_platform(
    tx: &mut Tx<'_>,
    actor: ActorId,
    role: RoleId,
    created_by: ActorId,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO role_grants (actor_id, role_id, scope_kind, scope_id, created_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING",
        actor.to_db(),
        role.to_db(),
        Platform::NAME,
        Platform::ID.to_db(),
        created_by.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Takes back a role on the instance from an actor. A role that the actor does not have is not
/// an error.
pub(super) async fn revoke_on_platform(
    tx: &mut Tx<'_>,
    actor: ActorId,
    role: RoleId,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM role_grants
         WHERE actor_id = $1 AND role_id = $2 AND scope_kind = $3 AND scope_id = $4",
        actor.to_db(),
        role.to_db(),
        Platform::NAME,
        Platform::ID.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes each grant of a role that is a row of an actor.
pub(crate) async fn delete_grants_of(tx: &mut Tx<'_>, actor: ActorId) -> Result<(), Error> {
    sqlx::query!("DELETE FROM role_grants WHERE actor_id = $1", actor.to_db())
        .execute(&mut **tx)
        .await?;
    Ok(())
}
