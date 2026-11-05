//! The SQL of the grants of the custom roles.

use scylla_domain::authz::Resource;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::roles::RoleId;
use scylla_extension::Tx;

use crate::Error;

/// Gives a role on the instance to an actor. A role that the actor has stays.
pub(crate) async fn grant_on_platform(
    tx: &mut Tx<'_>,
    actor: ActorId,
    role: RoleId,
    created_by: ActorId,
) -> Result<(), Error> {
    grant_row(
        tx,
        actor,
        role,
        (Platform::NAME, Platform::ID.to_db()),
        created_by,
    )
    .await
}

/// Gives a role on a scope to an actor. A role that the actor has stays.
pub(crate) async fn grant_row(
    tx: &mut Tx<'_>,
    actor: ActorId,
    role: RoleId,
    (kind, scope): (&str, uuid::Uuid),
    created_by: ActorId,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO role_grants (actor_id, role_id, scope_kind, scope_id, created_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT DO NOTHING",
        actor.to_db(),
        role.to_db(),
        kind,
        scope,
        created_by.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Takes back a role on a scope from an actor. A role that the actor does not have is not an error.
pub(crate) async fn revoke_row(
    tx: &mut Tx<'_>,
    actor: ActorId,
    role: RoleId,
    (kind, scope): (&str, uuid::Uuid),
) -> Result<(), Error> {
    sqlx::query!(
        "DELETE FROM role_grants
         WHERE actor_id = $1 AND role_id = $2 AND scope_kind = $3 AND scope_id = $4",
        actor.to_db(),
        role.to_db(),
        kind,
        scope,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Takes back a role on the instance from an actor. A role that the actor does not have is not
/// an error.
pub(crate) async fn revoke_on_platform(
    tx: &mut Tx<'_>,
    actor: ActorId,
    role: RoleId,
) -> Result<(), Error> {
    revoke_row(tx, actor, role, (Platform::NAME, Platform::ID.to_db())).await
}

/// Deletes each grant of a role that is a row of an actor, and the times of its other grants.
pub(crate) async fn delete_grants_of(tx: &mut Tx<'_>, actor: ActorId) -> Result<(), Error> {
    sqlx::query!("DELETE FROM role_grants WHERE actor_id = $1", actor.to_db())
        .execute(&mut **tx)
        .await?;
    sqlx::query!(
        "DELETE FROM relation_grants WHERE actor_id = $1",
        actor.to_db()
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the grants of a role that is a row.
pub(crate) async fn grants_of_role(
    tx: &mut Tx<'_>,
    role: RoleId,
) -> Result<Vec<(ActorId, String, uuid::Uuid)>, Error> {
    let rows = sqlx::query!(
        "SELECT actor_id, scope_kind, scope_id FROM role_grants WHERE role_id = $1",
        role.to_db(),
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (ActorId::from_db(row.actor_id), row.scope_kind, row.scope_id))
        .collect())
}
