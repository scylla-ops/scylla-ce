//! The SQL of the grants that are rows, and of the time of the grants of the schema.

use std::collections::HashMap;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::roles::RoleId;
use scylla_extension::Tx;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// A grant of a role that is a row.
pub(crate) struct RowGrant {
    /// The actor.
    pub(crate) actor: ActorId,
    /// The role.
    pub(crate) role: RoleId,
    /// The name of the role.
    pub(crate) name: String,
    /// The id of the scope.
    pub(crate) scope_id: Uuid,
    /// The time of the grant.
    pub(crate) created_at: OffsetDateTime,
    /// The actor that gave the role.
    pub(crate) created_by: Option<ActorId>,
}

/// Makes a value of a row of the grants.
macro_rules! row_grant {
    ($row:expr) => {
        RowGrant {
            actor: ActorId::from_db($row.actor_id),
            role: RoleId::from_db($row.role_id),
            name: $row.name,
            scope_id: $row.scope_id,
            created_at: $row.created_at,
            created_by: $row.created_by.map(ActorId::from_db),
        }
    };
}

/// Returns the grants of the roles that are rows on some scopes.
pub(crate) async fn rows_in(tx: &mut Tx<'_>, scopes: &[Uuid]) -> Result<Vec<RowGrant>, Error> {
    let rows = sqlx::query!(
        "SELECT g.actor_id, g.role_id, r.name, g.scope_id, g.created_at, g.created_by
         FROM role_grants g JOIN roles r ON r.id = g.role_id
         WHERE g.scope_id = ANY($1)",
        scopes,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|row| row_grant!(row)).collect())
}

/// Returns the grants of the roles that are rows of one actor on some scopes.
pub(crate) async fn rows_of(
    tx: &mut Tx<'_>,
    actor: ActorId,
    scopes: &[Uuid],
) -> Result<Vec<RowGrant>, Error> {
    let rows = sqlx::query!(
        "SELECT g.actor_id, g.role_id, r.name, g.scope_id, g.created_at, g.created_by
         FROM role_grants g JOIN roles r ON r.id = g.role_id
         WHERE g.actor_id = $1 AND g.scope_id = ANY($2)",
        actor.to_db(),
        scopes,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|row| row_grant!(row)).collect())
}

/// Returns the grants of the roles of the platform level of one actor.
pub(crate) async fn platform_rows_of(
    tx: &mut Tx<'_>,
    actor: ActorId,
) -> Result<Vec<RowGrant>, Error> {
    let rows = sqlx::query!(
        "SELECT g.actor_id, g.role_id, r.name, g.scope_id, g.created_at, g.created_by
         FROM role_grants g JOIN roles r ON r.id = g.role_id
         WHERE g.actor_id = $1 AND g.scope_kind = 'platform'",
        actor.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|row| row_grant!(row)).collect())
}

/// Returns at most `limit` grants of the roles that are rows on one object, after a grant, in the
/// order of the actor and of the role.
pub(crate) async fn rows_on(
    tx: &mut Tx<'_>,
    kind: &str,
    scope: Uuid,
    after: Option<(ActorId, RoleId)>,
    limit: usize,
) -> Result<Vec<RowGrant>, Error> {
    let rows = sqlx::query!(
        r#"SELECT g.actor_id, g.role_id, r.name, g.scope_id, g.created_at,
                  g.created_by
           FROM role_grants g JOIN roles r ON r.id = g.role_id
           WHERE g.scope_kind = $1 AND g.scope_id = $2
             AND ($3::uuid IS NULL OR (g.actor_id, g.role_id) > ($3, $4))
           ORDER BY g.actor_id, g.role_id
           LIMIT $5"#,
        kind,
        scope,
        after.map(|(actor, _)| actor.to_db()),
        after.map(|(_, role)| role.to_db()) as Option<Uuid>,
        limit as i64,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(|row| row_grant!(row)).collect())
}

/// Returns the permissions of some roles that are rows.
pub(crate) async fn permissions_of_roles(
    tx: &mut Tx<'_>,
    roles: &[Uuid],
) -> Result<HashMap<Uuid, Vec<String>>, Error> {
    let rows = sqlx::query!(
        "SELECT role_id, permission FROM role_permissions WHERE role_id = ANY($1)",
        roles,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut permissions: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in rows {
        permissions
            .entry(row.role_id)
            .or_default()
            .push(row.permission);
    }
    Ok(permissions)
}

/// Returns the time and the author of the grants of the schema on some scopes.
pub(crate) async fn relation_times(
    tx: &mut Tx<'_>,
    scopes: &[Uuid],
) -> Result<HashMap<(ActorId, Uuid, String), (OffsetDateTime, Option<ActorId>)>, Error> {
    let rows = sqlx::query!(
        "SELECT actor_id, scope_id, relation, created_by, created_at
         FROM relation_grants WHERE scope_id = ANY($1)",
        scopes,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let key = (ActorId::from_db(row.actor_id), row.scope_id, row.relation);
            (key, (row.created_at, row.created_by.map(ActorId::from_db)))
        })
        .collect())
}

/// Keeps the time and the author of a grant of the schema. A grant that has them stays.
pub(crate) async fn note_relation(
    tx: &mut Tx<'_>,
    actor: ActorId,
    (kind, scope): (&str, Uuid),
    relation: &str,
    by: ActorId,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO relation_grants (actor_id, scope_kind, scope_id, relation, created_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING",
        actor.to_db(),
        kind,
        scope,
        relation,
        by.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Forgets the time and the author of a grant of the schema.
pub(crate) async fn forget_relation(
    tx: &mut Tx<'_>,
    actor: ActorId,
    (kind, scope): (&str, Uuid),
    relation: &str,
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM relation_grants
         WHERE actor_id = $1 AND scope_kind = $2 AND scope_id = $3 AND relation = $4",
        actor.to_db(),
        kind,
        scope,
        relation,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the grants that are rows and the times of the grants on some scopes.
pub(crate) async fn delete_on_scopes(tx: &mut Tx<'_>, scopes: &[Uuid]) -> Result<(), Error> {
    sqlx::query!("DELETE FROM role_grants WHERE scope_id = ANY($1)", scopes)
        .execute(&mut **tx)
        .await?;
    sqlx::query!(
        "DELETE FROM relation_grants WHERE scope_id = ANY($1)",
        scopes
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
