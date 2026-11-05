//! The roles of the platform level that an actor holds: `Grant` and `Revoke` on the platform.

use scylla_domain::authz::{Object, Resource};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::admins::UserAdminHolder;
use scylla_domain::identity::users;
use scylla_domain::platform::Platform;
use scylla_domain::tenancy::catalog::{self, Level};
use scylla_domain::tenancy::roles::{self, RoleId, INSTANCE_ADMIN, WORKSPACE_CREATOR_ROW};
use scylla_extension::{Gate, Principal, Tx};
use sqlx::PgPool;

use super::service::{check_escalation, owner};
use super::store;
use crate::features::identity::accounts::refused;
use crate::features::identity::{actors, admins};
use crate::Error;

/// A role of the platform level, by its name in `Grant`.
enum PlatformRole {
    /// The role of the instance admins.
    Admins,
    /// A role that is a row, with its permissions.
    Row(RoleId, Vec<&'static str>),
}

/// Finds a role of the platform level by its name in `Grant`. Another name is a request error.
async fn platform_role(pool: &PgPool, name: &str) -> Result<PlatformRole, Error> {
    let not_a_role = || Error::invalid("role", "is not a role of the platform");
    if name == INSTANCE_ADMIN {
        return Ok(PlatformRole::Admins);
    }
    let id = match roles::builtin(name) {
        Some(_) if name == roles::WORKSPACE_CREATOR => WORKSPACE_CREATOR_ROW,
        Some(_) => return Err(not_a_role()),
        None => RoleId::parse(name).map_err(|_| not_a_role())?,
    };
    let row = owner(pool, id).await?.ok_or_else(not_a_role)?;
    if row.level != Level::Platform {
        return Err(not_a_role());
    }
    let permissions = row
        .permissions
        .iter()
        .filter_map(|id| catalog::find(id).map(|(_, permission)| permission.id))
        .collect();
    Ok(PlatformRole::Row(row.id, permissions))
}

/// Refuses a change of a role of the platform level that the caller cannot give.
async fn check_can_give(
    gate: &dyn Gate,
    caller: &Principal,
    role: &PlatformRole,
) -> Result<(), Error> {
    let platform = Object::of(Platform::ID);
    match role {
        PlatformRole::Admins => {
            let only_admins = [UserAdminHolder::GRANT_INSTANCE_ADMIN.name()];
            let answers = gate
                .permissions(caller, &platform, None, &only_admins)
                .await?;
            if answers.first().copied().unwrap_or(false) {
                return Ok(());
            }
            Err(refused(
                "ESCALATION",
                "only an instance admin gives this role",
            ))
        }
        PlatformRole::Row(_, permissions) => {
            check_escalation(gate, caller, &platform, None, permissions).await
        }
    }
}

/// Gives a role of the platform level to a user.
pub(crate) async fn give_on_platform(
    pool: &PgPool,
    gate: &dyn Gate,
    caller: &Principal,
    name: &str,
    actor: ActorId,
) -> Result<(), Error> {
    // Read the role, and check who gives it and who gets it.
    let role = platform_role(pool, name).await?;
    let grantee = actors::find(pool, actor).await?;
    if grantee.kind != users::KIND {
        return Err(refused(
            "ACTOR_NOT_A_USER",
            "only a user holds a role of the platform",
        ));
    }
    check_can_give(gate, caller, &role).await?;

    // Write the role where it lives.
    match role {
        PlatformRole::Admins => admins::grant(pool, actor).await?,
        PlatformRole::Row(id, _) => {
            let mut tx = pool.begin().await?;
            store::grant_on_platform(&mut tx, actor, id, Some(caller.actor.id)).await?;
            tx.commit().await?;
        }
    }
    tracing::info!(event = "role.granted", role = name, object = Platform::NAME, grantee = %actor);
    Ok(())
}

/// Takes back a role of the platform level. The last active instance admin stays.
pub(crate) async fn take_back_on_platform(
    pool: &PgPool,
    gate: &dyn Gate,
    caller: &Principal,
    name: &str,
    actor: ActorId,
) -> Result<(), Error> {
    let role = platform_role(pool, name).await?;
    check_can_give(gate, caller, &role).await?;
    match role {
        PlatformRole::Admins => admins::revoke(pool, actor).await?,
        PlatformRole::Row(id, _) => {
            let mut tx = pool.begin().await?;
            store::revoke_on_platform(&mut tx, actor, id).await?;
            tx.commit().await?;
        }
    }
    tracing::info!(event = "role.revoked", role = name, object = Platform::NAME, grantee = %actor);
    Ok(())
}

/// Gives the role that creates workspaces to a user, in the transaction of the caller.
pub(crate) async fn make_workspace_creator(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    store::grant_on_platform(tx, user, WORKSPACE_CREATOR_ROW, None).await
}
