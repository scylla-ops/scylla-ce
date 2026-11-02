//! The logic of the accounts: the creation of a user, sign-up, the check of an email, and the
//! sign-in with a password.

use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::users::{Email, NewUser, Password, UserStatus, KIND};
use scylla_domain::tenancy::invitations::Invitation;
use scylla_domain::tenancy::workspaces::NewWorkspace;
use scylla_domain::Name;
use scylla_extension::{AccessError, Rejection, Tx};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::one_time::{self, Purpose};
use super::{mails, password, store, AccountContext};
use crate::features::identity::actors;
use crate::features::identity::sessions::{self, Opened};
use crate::features::tenancy::invitations;
use crate::features::tenancy::workspaces;
use crate::{Error, SignUpMode};

/// The name of the workspace that a new user gets.
pub const PERSONAL: &str = "Personal";

/// The input of a sign-up with a password.
pub(super) struct SignUp {
    /// The email.
    pub(super) email: Email,
    /// The password.
    pub(super) password: Password,
    /// The display name.
    pub(super) display_name: Name,
    /// The token of an invitation link.
    pub(super) invitation: Option<String>,
}

/// Creates a user, with its email, its password and its personal workspace. A known email gives
/// `FAILED_PRECONDITION` with `EMAIL_IN_USE`.
pub async fn create_user(pool: &PgPool, new: NewUser) -> Result<Actor, Error> {
    let hash = match &new.password {
        Some(password) => Some(password::hash(password.as_str().to_owned()).await?),
        None => None,
    };
    let mut tx = pool.begin().await?;
    let actor = insert_user(&mut tx, &new, hash.as_deref()).await?;
    tx.commit().await?;
    Ok(actor)
}

/// Inserts a user and its parts in the transaction of the caller, with the hash of its password.
pub(crate) async fn insert_user(
    tx: &mut Tx<'_>,
    new: &NewUser,
    hash: Option<&str>,
) -> Result<Actor, Error> {
    // Refuse a known email.
    if store::user_of_email(tx, &new.email).await?.is_some() {
        let message = "an account uses this email";
        return Err(Error::Rejected(Rejection::new("EMAIL_IN_USE", message)));
    }

    // Insert the actor, the user, its email and its password.
    let actor = actors::insert(tx, KIND).await?;
    let status = if new.email_verified {
        UserStatus::Active
    } else {
        UserStatus::Pending
    };
    store::insert_user(tx, actor.id, new.display_name.as_str(), status).await?;
    store::insert_email(tx, actor.id, &new.email, new.email_verified).await?;
    if let Some(hash) = hash {
        store::set_password(tx, actor.id, hash).await?;
    }

    // Give the user its personal workspace.
    let personal = NewWorkspace {
        creator: actor.id,
        name: Name::parse(PERSONAL.to_owned()).expect("a valid name"),
    };
    workspaces::insert(tx, &personal).await?;
    Ok(actor)
}

/// Creates a user that signs in with a password. With an invitation for the same email, the
/// email is verified and the invitation is accepted. A known email gets the same answer, and its
/// owner gets a mail.
pub(super) async fn sign_up(ctx: &AccountContext, input: SignUp) -> Result<(), Error> {
    // Find the invitation, then check the sign-up mode.
    let invitation = match &input.invitation {
        Some(token) => Some(invitation_for(ctx, token, &input.email).await?),
        None => None,
    };
    match (ctx.settings.sign_up_mode, &invitation) {
        (SignUpMode::Closed, _) => return Err(refused("SIGN_UP_CLOSED", "sign-up is closed")),
        (SignUpMode::InviteOnly, None) => {
            return Err(refused(
                "INVITATION_REQUIRED",
                "sign-up needs an invitation",
            ));
        }
        _ => {}
    }

    // Create the user, or tell the owner of a known email.
    let hash = password::hash(input.password.as_str().to_owned()).await?;
    let new = NewUser {
        email: input.email,
        email_verified: invitation.is_some(),
        display_name: input.display_name,
        password: None,
    };
    let mut tx = ctx.pool.begin().await?;
    if store::user_of_email(&mut tx, &new.email).await?.is_some() {
        tx.commit().await?;
        mails::send(ctx, mails::account_exists(ctx, new.email.as_str())).await;
        return Ok(());
    }
    let actor = insert_user(&mut tx, &new, Some(&hash)).await?;
    let verify = match invitation {
        Some(_) => None,
        None => Some(
            one_time::create(&mut tx, Purpose::VerifyEmail, actor.id, new.email.as_str()).await?,
        ),
    };
    tx.commit().await?;

    // Accept the invitation, or send the link that verifies the email.
    if let Some(invitation) = invitation {
        invitations::accept(ctx, &invitation, actor.id).await?;
    }
    if let Some(token) = verify {
        mails::send(ctx, mails::verify_email(ctx, new.email.as_str(), &token)).await;
    }
    Ok(())
}

/// Verifies an email with the token of its link, and activates its user.
pub(super) async fn verify_email(ctx: &AccountContext, token: &str) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    let redeemed = one_time::redeem(&mut tx, Purpose::VerifyEmail, token)
        .await?
        .ok_or_else(|| Error::invalid("token", "is not a valid link"))?;
    store::verify_email(&mut tx, redeemed.user, &redeemed.email).await?;
    tx.commit().await?;
    Ok(())
}

/// Checks an email and a password, and opens a session. Each refusal has the same answer, and a
/// locked account is not checked.
pub(super) async fn sign_in(
    ctx: &AccountContext,
    email: &Email,
    password: String,
    user_agent: Option<&str>,
) -> Result<(Actor, Opened), Error> {
    // Read the credentials, and check the password even for an unknown email.
    let mut tx = ctx.pool.begin().await?;
    let credentials = store::credentials(&mut tx, email).await?;
    tx.commit().await?;
    let locked = credentials
        .as_ref()
        .and_then(|credentials| credentials.locked_until)
        .is_some_and(|until| until > OffsetDateTime::now_utc());
    let hash = credentials
        .as_ref()
        .and_then(|credentials| credentials.hash.clone());
    let matches = password::verify(password, hash.filter(|_| !locked)).await;

    // Count a failure, then refuse.
    let mut tx = ctx.pool.begin().await?;
    let credentials = match credentials {
        Some(credentials) if matches => credentials,
        Some(credentials) => {
            store::record_failure(&mut tx, credentials.user).await?;
            tx.commit().await?;
            return Err(invalid());
        }
        None => return Err(invalid()),
    };

    // Refuse an account that is not active, then open a session.
    match credentials.status {
        UserStatus::Active => {}
        UserStatus::Pending => {
            return Err(refused("EMAIL_NOT_VERIFIED", "verify your email first"))
        }
        UserStatus::Disabled => return Err(invalid()),
    }
    store::record_sign_in(&mut tx, credentials.user).await?;
    let methods = ["password".to_owned()];
    let ttl = ctx.settings.session_ttl;
    let opened = sessions::open(&mut tx, credentials.user, &methods, user_agent, ttl).await?;
    tx.commit().await?;
    let actor = Actor {
        id: credentials.user,
        kind: KIND,
    };
    Ok((actor, opened))
}

/// Sends a link that resets the password, if the email has an account. The answer is the same.
pub(super) async fn request_reset(ctx: &AccountContext, email: &Email) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    let Some(user) = store::user_of_email(&mut tx, email).await? else {
        return Ok(());
    };
    let token = one_time::create(&mut tx, Purpose::ResetPassword, user, email.as_str()).await?;
    tx.commit().await?;
    mails::send(ctx, mails::reset_password(ctx, email.as_str(), &token)).await;
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
    let redeemed = one_time::redeem(&mut tx, Purpose::ResetPassword, token)
        .await?
        .ok_or_else(|| Error::invalid("token", "is not a valid link"))?;
    store::set_password(&mut tx, redeemed.user, &hash).await?;
    store::verify_email(&mut tx, redeemed.user, &redeemed.email).await?;
    sessions::revoke_all(&mut tx, redeemed.user, None).await?;
    tx.commit().await?;
    Ok(())
}

/// Finds an open invitation for a sign-up. It must be for the email of the sign-up.
async fn invitation_for(
    ctx: &AccountContext,
    token: &str,
    email: &Email,
) -> Result<Invitation, Error> {
    invitations::find_open(&ctx.pool, token)
        .await?
        .filter(|invitation| invitation.email == email.as_str())
        .ok_or_else(|| {
            Error::invalid(
                "invitation_token",
                "is not an open invitation for this email",
            )
        })
}

/// Makes a refusal of the account rules: `FAILED_PRECONDITION` with a stable code.
pub(crate) fn refused(code: &'static str, message: &str) -> Error {
    Error::Rejected(Rejection::new(code, message.to_owned()))
}

/// The answer of each refused password sign-in.
fn invalid() -> Error {
    AccessError::Unauthenticated("invalid email or password".to_owned()).into()
}
