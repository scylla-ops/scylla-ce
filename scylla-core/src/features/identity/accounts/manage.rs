//! The changes of an account: its profile, its email, its password, its identities and its
//! sessions. `ProfileService` and the console of the users share them.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::admins::{ListUsers, UserPage};
use scylla_domain::identity::users::{Email, Password, ProfileChange, SessionId, User};
use scylla_domain::Page;
use sqlx::PgPool;
use tracing::Instrument;

use super::one_time::{self, Purpose};
use super::{deletion, mails, password, refused, store, AccountContext};
use crate::features::identity::providers::{self, IdentityInfo};
use crate::features::identity::sessions;
use crate::features::notifications::{self, kinds};
use crate::features::tenancy::invitations;
use crate::infra::logs::mask_email;
use crate::Error;

/// Returns a user. An unknown user gives `NotFound("user")`.
pub(crate) async fn get_user(pool: &PgPool, user: ActorId) -> Result<User, Error> {
    let mut tx = pool.begin().await?;
    let found = store::find_user(&mut tx, user).await?;
    tx.commit().await?;
    found.ok_or(Error::NotFound("user"))
}

/// Returns one page of the users that match a query, in id order.
pub(crate) async fn list_users(pool: &PgPool, query: ListUsers) -> Result<UserPage, Error> {
    let mut tx = pool.begin().await?;
    let users = store::list_users(&mut tx, &query).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(users, query.page.size, |user| user.id))
}

/// Changes the display name, the handle, and the email if one is given. A new email replaces
/// the old one, is verified, and stops each open link of the user. Returns the user after the
/// change.
pub(crate) async fn change_profile(
    pool: &PgPool,
    user: ActorId,
    change: &ProfileChange,
    email: Option<&Email>,
) -> Result<User, Error> {
    // Lock the user, and change its profile.
    let mut tx = pool.begin().await?;
    if !store::lock_user(&mut tx, user).await? {
        return Err(Error::NotFound("user"));
    }
    store::update_profile(&mut tx, user, change).await?;

    // Replace the email, unless another user has it, then read the user after the change.
    if let Some(email) = email {
        if store::user_of_email(&mut tx, email)
            .await?
            .is_some_and(|owner| owner != user)
        {
            return Err(store::email_in_use());
        }
        store::replace_email(&mut tx, user, email).await?;
        one_time::cancel_all(&mut tx, user).await?;
        notifications::emit(&mut tx, kinds::about(kinds::EMAIL_CHANGED, user)).await?;
        invitations::announce_open(&mut tx, user, email.as_str()).await?;
    }
    let changed = store::find_user(&mut tx, user).await?;
    tx.commit().await?;
    if let Some(email) = email {
        tracing::info!(event = "email.changed", %user, email = mask_email(email.as_str()));
    }
    changed.ok_or(Error::NotFound("user"))
}

/// Changes a password. It needs the current password if the user has one, and closes the
/// sessions of the user, except one if it is given.
pub(crate) async fn change_password(
    ctx: &AccountContext,
    user: ActorId,
    keep: Option<SessionId>,
    current: String,
    new: Password,
) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    check_password(&mut tx, user, current, "current_password").await?;
    let hash = password::hash(new.as_str().to_owned()).await?;
    store::set_password(&mut tx, user, &hash).await?;
    sessions::revoke_all(&mut tx, user, keep).await?;
    notifications::emit(&mut tx, kinds::about(kinds::PASSWORD_CHANGED, user)).await?;
    let to = store::find_user(&mut tx, user)
        .await?
        .and_then(|user| user.email);
    tx.commit().await?;
    tracing::info!(event = "password.changed", %user, via = "profile");

    // Tell the owner. A mail that fails does not stop the change.
    if let Some(to) = to {
        let mail = mails::password_changed(ctx, &to);
        mails::send(ctx, "password-changed", mail).await;
    }
    Ok(())
}

/// Returns the identities of a user, and `true` if the user also has a password.
pub(crate) async fn sign_in_ways(
    pool: &PgPool,
    user: ActorId,
) -> Result<(Vec<IdentityInfo>, bool), Error> {
    let mut tx = pool.begin().await?;
    let identities = providers::identities_of(&mut tx, user).await?;
    let has_password = store::password_hash(&mut tx, user).await?.is_some();
    tx.commit().await?;
    Ok((identities, has_password))
}

/// Removes an identity of a user, and tells the user by mail. The last way to sign in stays.
pub(crate) async fn unlink_identity(
    ctx: &AccountContext,
    user: ActorId,
    provider: &str,
    subject: &str,
) -> Result<(), Error> {
    // Refuse to remove the last way to sign in.
    let mut tx = ctx.pool.begin().await?;
    let identities = providers::identities_of(&mut tx, user).await?;
    let has_password = store::password_hash(&mut tx, user).await?.is_some();
    if identities.len() <= 1 && !has_password {
        let message = "the last way to sign in cannot be removed";
        return Err(refused("LAST_SIGN_IN_METHOD", message));
    }

    // Remove the identity.
    if !providers::unlink(&mut tx, user, provider, subject).await? {
        return Err(Error::NotFound("identity"));
    }
    let name = providers::by_slug(&mut tx, provider)
        .await?
        .map_or_else(|| provider.to_owned(), |row| row.display_name);
    let to = store::find_user(&mut tx, user)
        .await?
        .and_then(|user| user.email);
    let fact = kinds::about(kinds::IDENTITY_UNLINKED, user).param("provider", name.clone());
    notifications::emit(&mut tx, fact).await?;
    tx.commit().await?;
    tracing::info!(event = "account.identity_unlinked", %user, provider);

    // Tell the owner. A mail that fails does not stop the unlink.
    if let Some(to) = to {
        let mail = mails::identity_unlinked(ctx, &to, &name);
        mails::send(ctx, "identity-unlinked", mail).await;
    }
    Ok(())
}

/// Closes each session of a user, except one if it is given. An unknown user gives
/// `NotFound("user")`.
pub(crate) async fn revoke_sessions(
    pool: &PgPool,
    user: ActorId,
    keep: Option<SessionId>,
) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !store::lock_user(&mut tx, user).await? {
        return Err(Error::NotFound("user"));
    }
    sessions::revoke_all(&mut tx, user, keep).await?;
    tx.commit().await?;
    let scope = if keep.is_some() { "others" } else { "all" };
    tracing::info!(event = "session.revoked", %user, scope);
    Ok(())
}

/// Checks the current password if the user has one, then answers, and sends a link to the new
/// email in a task. No link goes to an email that an account uses, and at most one link goes
/// each `COOLDOWN` for one user and for one email. An error of the task goes to the log only.
pub(crate) async fn request_email_change(
    ctx: &AccountContext,
    user: ActorId,
    email: Email,
    current: String,
) -> Result<(), Error> {
    // Check the current password.
    let mut tx = ctx.pool.begin().await?;
    check_password(&mut tx, user, current, "password").await?;
    tx.commit().await?;
    tracing::info!(event = "email.change_requested", %user, email = mask_email(email.as_str()));

    // Make and send the link in a task, after the answer.
    let ctx = ctx.clone();
    let task = async move {
        let sent = async {
            // Lock the user, then skip a used email and a link sent a short time ago.
            let mut tx = ctx.pool.begin().await?;
            if !store::lock_user(&mut tx, user).await?
                || store::user_of_email(&mut tx, &email).await?.is_some()
                || one_time::is_recent(&mut tx, Purpose::ChangeEmail, user).await?
                || one_time::is_recent_to(&mut tx, Purpose::ChangeEmail, email.as_str()).await?
            {
                return Ok::<(), Error>(());
            }

            // Replace the older links, then send the new one.
            one_time::cancel(&mut tx, Purpose::ChangeEmail, user).await?;
            let token = one_time::create(&mut tx, Purpose::ChangeEmail, user, email.as_str());
            let token = token.await?;
            tx.commit().await?;
            let mail = mails::change_email(&ctx, email.as_str(), &token);
            mails::send(&ctx, "change-email", mail).await;
            Ok(())
        };
        if let Err(error) = sent.await {
            tracing::warn!(%error, "a link of an email change was not made");
        }
    };
    tokio::spawn(task.in_current_span());
    Ok(())
}

/// Makes the email of a link the primary email of its user, with the token of the link. The
/// token must be of this user. Each other open link of the user stops, and the old email gets
/// a notice.
pub(crate) async fn confirm_email_change(
    ctx: &AccountContext,
    user: ActorId,
    token: &str,
) -> Result<User, Error> {
    // Use the token of the user, and read the old email.
    let mut tx = ctx.pool.begin().await?;
    let redeemed = one_time::redeem(&mut tx, Purpose::ChangeEmail, token, Some(user))
        .await?
        .ok_or_else(|| Error::invalid("token", "is not a valid link"))?;
    let new =
        Email::parse(redeemed.email).map_err(|_| Error::invalid("token", "is not a valid link"))?;
    let old = store::find_user(&mut tx, user)
        .await?
        .and_then(|user| user.email);

    // Replace the email, stop the other links, then tell the old email.
    store::replace_email(&mut tx, user, &new).await?;
    one_time::cancel_all(&mut tx, user).await?;
    notifications::emit(&mut tx, kinds::about(kinds::EMAIL_CHANGED, user)).await?;
    invitations::announce_open(&mut tx, user, new.as_str()).await?;
    let changed = store::find_user(&mut tx, user).await?;
    tx.commit().await?;
    tracing::info!(event = "email.changed", %user, email = mask_email(new.as_str()));
    if let Some(old) = old.filter(|old| old != new.as_str()) {
        mails::send(
            ctx,
            "email-changed",
            mails::email_changed(&old, new.as_str()),
        )
        .await;
    }
    changed.ok_or(Error::NotFound("user"))
}

/// Deletes the account of the caller. It needs the current password if the user has one.
pub(crate) async fn delete_account(
    ctx: &AccountContext,
    user: ActorId,
    current: String,
) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    check_password(&mut tx, user, current, "password").await?;
    tx.commit().await?;
    deletion::delete_user(ctx, user).await
}

/// Passes if the user has no password, or if the text matches it. A wrong text gives
/// `INVALID_ARGUMENT` on the field.
async fn check_password(
    tx: &mut scylla_extension::Tx<'_>,
    user: ActorId,
    text: String,
    field: &'static str,
) -> Result<(), Error> {
    if let Some(hash) = store::password_hash(tx, user).await? {
        if !password::verify(text, Some(hash)).await {
            return Err(Error::invalid(field, "does not match"));
        }
    }
    Ok(())
}
