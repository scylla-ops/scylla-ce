//! The logic of the accounts: the creation of a user, sign-up, the check of an email, and the
//! sign-in with a password.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::{
    Email, Handle, NewUser, Password, SessionId, UserStatus, KIND,
};
use scylla_domain::tenancy::invitations::Invitation;
use scylla_domain::tenancy::slugs::Slug;
use scylla_domain::tenancy::workspaces::{NewSlug, NewWorkspace};
use scylla_domain::Name;
use scylla_extension::{Rejection, Tx};
use sqlx::PgPool;

use super::credentials::verify_password;
use super::one_time::{self, Purpose};
use super::{mails, password, store, AccountContext};
use crate::features::identity::sessions::{self, Client, Opened};
use crate::features::identity::{actors, admins};
use crate::features::tenancy::invitations::{self, Pick};
use crate::features::tenancy::workspaces;
use crate::infra::logs::{mask_email, mask_identifier};
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

/// Creates a user, with its email, its handle, its password and its personal workspace. A known
/// email gives `FAILED_PRECONDITION` with `EMAIL_IN_USE`, a known handle `HANDLE_IN_USE`.
pub async fn create_user(pool: &PgPool, new: NewUser) -> Result<Actor, Error> {
    create(pool, new, false).await
}

/// Creates a user as `create_user` does, and makes it an instance admin in the same
/// transaction.
pub async fn create_admin(pool: &PgPool, new: NewUser) -> Result<Actor, Error> {
    create(pool, new, true).await
}

/// Creates a user, and an instance admin if `admin` is `true`, in one transaction.
async fn create(pool: &PgPool, new: NewUser, admin: bool) -> Result<Actor, Error> {
    let hash = match &new.password {
        Some(password) => Some(password::hash(password.as_str().to_owned()).await?),
        None => None,
    };
    let mut tx = pool.begin().await?;
    let actor = insert_user(&mut tx, &new, hash.as_deref()).await?;
    if admin {
        admins::grant_in(&mut tx, actor.id).await?;
    }
    tx.commit().await?;
    created(actor.id, &new.email, admin);
    Ok(actor)
}

/// Logs the event `account.created`, with the masked email.
pub(crate) fn created(user: ActorId, email: &Email, admin: bool) {
    let email = mask_email(email.as_str());
    tracing::info!(event = "account.created", %user, email, admin = admin.then_some(true));
}

/// Inserts a user and its parts in the transaction of the caller, with the hash of its password.
pub(crate) async fn insert_user(
    tx: &mut Tx<'_>,
    new: &NewUser,
    hash: Option<&str>,
) -> Result<Actor, Error> {
    // Refuse a known email.
    if store::user_of_email(tx, &new.email).await?.is_some() {
        return Err(store::email_in_use());
    }

    // Insert the actor, the user, its email and its password.
    let actor = actors::insert(tx, KIND).await?;
    let status = if new.email_verified {
        UserStatus::Active
    } else {
        UserStatus::Pending
    };
    let handle = new.handle.as_ref().map(|handle| handle.as_str());
    store::insert_user(tx, actor.id, new.display_name.as_str(), handle, status).await?;
    store::insert_email(tx, actor.id, &new.email, new.email_verified).await?;
    if let Some(hash) = hash {
        store::set_password(tx, actor.id, hash).await?;
    }

    // Give the user its personal workspace, with a slug from its handle or its name.
    let source = new
        .handle
        .as_ref()
        .map_or(new.display_name.as_str(), Handle::as_str);
    let personal = NewWorkspace {
        creator: actor.id,
        name: Name::parse(PERSONAL.to_owned()).expect("a valid name"),
        slug: NewSlug::Derived(Slug::derive(source)),
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
        handle: None,
        password: None,
    };
    let mut tx = ctx.pool.begin().await?;
    if store::user_of_email(&mut tx, &new.email).await?.is_some() {
        tx.commit().await?;
        mails::send(
            ctx,
            "account-exists",
            mails::account_exists(ctx, new.email.as_str()),
        )
        .await;
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
    created(actor.id, &new.email, false);

    // Accept the invitation of the link, or send the link that verifies the email.
    if let (Some(_), Some(token)) = (&invitation, &input.invitation) {
        invitations::accept(ctx, Pick::Token(token), actor.id).await?;
    }
    if let Some(token) = verify {
        mails::send(
            ctx,
            "verify-email",
            mails::verify_email(ctx, new.email.as_str(), &token),
        )
        .await;
    }
    Ok(())
}

/// Verifies an email with the token of its link, and activates its user.
pub(super) async fn verify_email(ctx: &AccountContext, token: &str) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    let redeemed = one_time::redeem(&mut tx, Purpose::VerifyEmail, token, None)
        .await?
        .ok_or_else(|| Error::invalid("token", "is not a valid link"))?;
    store::verify_email(&mut tx, redeemed.user, &redeemed.email).await?;
    tx.commit().await?;
    tracing::info!(event = "account.verified", user = %redeemed.user);
    Ok(())
}

/// Checks an email or a handle and a password, and opens a session.
pub(super) async fn sign_in(
    ctx: &AccountContext,
    identifier: String,
    password: String,
    client: &Client,
) -> Result<(Actor, Opened), Error> {
    let masked = mask_identifier(&identifier);
    let user = verify_password(ctx, identifier, &masked, password).await?;
    let mut tx = ctx.pool.begin().await?;
    store::record_sign_in(&mut tx, user).await?;
    let methods = ["password".to_owned()];
    let ttl = ctx.settings.session_ttl;
    let opened = sessions::open(&mut tx, user, &methods, client, ttl).await?;
    tx.commit().await?;
    signed_in(user, "password", opened.session);
    let actor = Actor {
        id: user,
        kind: KIND,
    };
    Ok((actor, opened))
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

/// Logs the event `account.signed_in`, with the method and the session.
pub(crate) fn signed_in(user: ActorId, method: &str, session: SessionId) {
    tracing::info!(event = "account.signed_in", %user, method, %session);
}
