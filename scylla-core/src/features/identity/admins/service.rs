//! The logic of the instance admins and of the console of the users: one transaction for each
//! call. The instance keeps one active admin or more.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{NewUser, User, UserStatus};
use scylla_extension::Tx;
use sqlx::PgPool;

use super::store;
use crate::features::identity::accounts::{self, refused, AccountContext};
use crate::features::identity::sessions;
use crate::Error;

/// Refuses a change that takes the user out of the active instance admins when no other
/// active admin stays: `LAST_ADMIN`. It locks the admins until the end of the transaction, so
/// two changes at the same time run one after the other.
pub(crate) async fn keep_an_admin(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    let admins = store::lock_all(tx).await?;
    let is_admin = admins.iter().any(|admin| admin.user == user);
    let others = admins
        .iter()
        .any(|admin| admin.user != user && admin.active);
    if is_admin && !others {
        return Err(refused("LAST_ADMIN", "the instance keeps one active admin"));
    }
    Ok(())
}

/// Makes a user an instance admin. An admin stays an admin. An unknown user gives
/// `NotFound("user")`.
pub async fn grant(pool: &PgPool, user: ActorId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    store::grant(&mut tx, user).await?;
    tx.commit().await?;
    Ok(())
}

/// Makes a user an instance admin in the transaction of the caller.
pub(crate) async fn grant_in(tx: &mut Tx<'_>, user: ActorId) -> Result<(), Error> {
    store::grant(tx, user).await
}

/// Takes the instance admin right from a user. The last active admin stays.
pub async fn revoke(pool: &PgPool, user: ActorId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    keep_an_admin(&mut tx, user).await?;
    store::revoke(&mut tx, user).await?;
    tx.commit().await?;
    Ok(())
}

/// Creates an active user with a verified email, and returns it.
pub(super) async fn create(pool: &PgPool, new: NewUser) -> Result<User, Error> {
    let actor = accounts::create_user(pool, new).await?;
    accounts::get_user(pool, actor.id).await
}

/// Activates or disables a user, and returns it. The caller cannot change itself. A disabled
/// user loses each session at once, and the last active instance admin stays.
pub(super) async fn set_status(
    pool: &PgPool,
    caller: ActorId,
    user: ActorId,
    status: UserStatus,
) -> Result<User, Error> {
    // Refuse a change of the caller itself, and of an unknown user.
    if caller == user {
        return Err(not_self());
    }
    let mut tx = pool.begin().await?;
    if !accounts::lock_user(&mut tx, user).await? {
        return Err(Error::NotFound("user"));
    }

    // Keep an active admin, then disable the user and close its sessions.
    if status == UserStatus::Disabled {
        keep_an_admin(&mut tx, user).await?;
        sessions::revoke_all(&mut tx, user, None).await?;
    }
    accounts::set_status(&mut tx, user, status).await?;
    let changed = accounts::find_user(&mut tx, user).await?;
    tx.commit().await?;
    changed.ok_or(Error::NotFound("user"))
}

/// Deletes a user other than the caller, as `DeleteAccount` does.
pub(super) async fn delete(
    ctx: &AccountContext,
    caller: ActorId,
    user: ActorId,
) -> Result<(), Error> {
    if caller == user {
        return Err(not_self());
    }
    accounts::delete_user(ctx, user).await
}

/// Returns the open sessions of a user, newest first. An unknown user gives
/// `NotFound("user")`.
pub(super) async fn sessions_of(
    pool: &PgPool,
    user: ActorId,
) -> Result<Vec<sessions::SessionInfo>, Error> {
    accounts::get_user(pool, user).await?;
    sessions::of_user(pool, user).await
}

/// The refusal of a route of the console on the caller itself.
fn not_self() -> Error {
    refused(
        "SELF_NOT_ALLOWED",
        "an admin cannot do this on its own account",
    )
}
