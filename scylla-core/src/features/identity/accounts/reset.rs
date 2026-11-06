//! The reset of a password: a link by mail, made and sent after the answer, at most one each
//! `COOLDOWN` for one account, then a new password with the token of the link.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{Email, Password};
use tracing::Instrument;

use super::one_time::{self, Purpose};
use super::{mails, password, store, AccountContext};
use crate::features::identity::sessions;
use crate::features::notifications::{self, kinds};
use crate::features::tenancy::invitations;
use crate::infra::logs::mask_email;
use crate::Error;

/// Answers at once, then sends a reset link to the email in a task, if the email has an
/// account. The answer is the same for each email, and an error goes to the log only.
pub(super) fn request_reset(ctx: &AccountContext, email: Email) {
    let masked = mask_email(email.as_str());
    tracing::info!(event = "password.reset_requested", email = masked);
    let ctx = ctx.clone();
    let task = async move {
        let sent = async {
            // Find the user of the email.
            let mut tx = ctx.pool.begin().await?;
            let user = store::user_of_email(&mut tx, &email).await?;
            tx.commit().await?;

            // Send a link to a known email only.
            match user {
                Some(user) => send_link(&ctx, user, email.as_str()).await,
                None => Ok(()),
            }
        };
        if let Err(error) = sent.await {
            tracing::warn!(%error, "a reset link was not made");
        }
    };
    tokio::spawn(task.in_current_span());
}

/// Sends a reset link to the primary email of a user in a task, after the answer. An unknown
/// user gives `NotFound("user")`.
pub(crate) async fn send_reset(ctx: &AccountContext, user: ActorId) -> Result<(), Error> {
    // Read the primary email of the user.
    let mut tx = ctx.pool.begin().await?;
    let found = store::find_user(&mut tx, user).await?;
    tx.commit().await?;
    let email = found
        .ok_or(Error::NotFound("user"))?
        .email
        .ok_or(Error::NotFound("email"))?;

    // Make and send the link in a task, after the answer.
    tracing::info!(event = "password.reset_requested", %user);
    let ctx = ctx.clone();
    let task = async move {
        if let Err(error) = send_link(&ctx, user, &email).await {
            tracing::warn!(%error, "a reset link was not made");
        }
    };
    tokio::spawn(task.in_current_span());
    Ok(())
}

/// Makes a reset link and sends it, unless the user got one less than `COOLDOWN` ago. The new
/// link replaces the older ones.
async fn send_link(ctx: &AccountContext, user: ActorId, email: &str) -> Result<(), Error> {
    // Lock the user, so two requests at the same time make one link only.
    let mut tx = ctx.pool.begin().await?;
    if !store::lock_user(&mut tx, user).await?
        || one_time::is_recent(&mut tx, Purpose::ResetPassword, user).await?
    {
        return Ok(());
    }

    // Replace the older links, then send the new one.
    one_time::cancel(&mut tx, Purpose::ResetPassword, user).await?;
    let token = one_time::create(&mut tx, Purpose::ResetPassword, user, email).await?;
    tx.commit().await?;
    mails::send(
        ctx,
        "reset-password",
        mails::reset_password(ctx, email, &token),
    )
    .await;
    Ok(())
}

/// Sets a new password with the token of a reset link. The link proves the email, and each
/// session of the user closes.
pub(super) async fn reset_password(
    ctx: &AccountContext,
    token: &str,
    password: Password,
) -> Result<(), Error> {
    let hash = password::hash(password.as_str().to_owned()).await?;
    let mut tx = ctx.pool.begin().await?;
    let redeemed = one_time::redeem(&mut tx, Purpose::ResetPassword, token, None)
        .await?
        .ok_or_else(|| Error::invalid("token", "is not a valid link"))?;
    store::set_password(&mut tx, redeemed.user, &hash).await?;
    store::verify_email(&mut tx, redeemed.user, &redeemed.email).await?;
    sessions::revoke_all(&mut tx, redeemed.user, None).await?;
    let fact = kinds::about(kinds::PASSWORD_CHANGED, redeemed.user);
    notifications::emit(&mut tx, fact).await?;
    invitations::announce_open(&mut tx, redeemed.user, &redeemed.email).await?;
    tx.commit().await?;
    tracing::info!(event = "password.changed", user = %redeemed.user, via = "reset");
    let mail = mails::password_changed(ctx, &redeemed.email);
    mails::send(ctx, "password-changed", mail).await;
    Ok(())
}
