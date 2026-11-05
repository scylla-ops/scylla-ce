//! What an identity of a provider does: sign in its user, make a new user, or link to the
//! signed-in user. A known email is never linked automatically.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::{Email, NewUser, UserStatus, KIND};
use scylla_domain::Name;

use super::store::{self, ProviderRow};
use super::ExternalIdentity;
use crate::features::identity::accounts::{self, refused, AccountContext};
use crate::features::identity::sessions::{self, Client, Opened};
use crate::features::tenancy::invitations::{self, Pick};
use crate::{Error, SignUpMode};

/// Signs in the user of an identity, or makes a user as the provider and the sign-up mode
/// allow. A new user needs a verified email that no account has.
pub(super) async fn sign_in(
    ctx: &AccountContext,
    provider: &ProviderRow,
    identity: ExternalIdentity,
    client: &Client,
) -> Result<(Actor, Opened), Error> {
    // Sign in the user of a known identity, if its account is active.
    let (methods, ttl) = ([provider.slug.clone()], ctx.settings.session_ttl);
    let mut tx = ctx.pool.begin().await?;
    if let Some(user) = store::user_of(&mut tx, provider.id, &identity.subject).await? {
        let status = accounts::find_user(&mut tx, user)
            .await?
            .map(|user| user.status);
        if status != Some(UserStatus::Active) {
            return Err(refused("ACCOUNT_INACTIVE", "the account is not active"));
        }
        accounts::record_sign_in(&mut tx, user).await?;
        let opened = sessions::open(&mut tx, user, &methods, client, ttl).await?;
        tx.commit().await?;
        accounts::signed_in(user, &provider.slug, opened.session);
        return Ok((
            Actor {
                id: user,
                kind: KIND,
            },
            opened,
        ));
    }

    // Check that a new user can come from this identity.
    if !provider.allow_sign_up {
        return Err(refused("NO_ACCOUNT", "no account uses this identity"));
    }
    let email = identity
        .email
        .filter(|_| identity.email_verified)
        .and_then(|email| Email::parse(email).ok())
        .ok_or_else(|| refused("EMAIL_REQUIRED", "the provider gave no verified email"))?;
    if accounts::user_of_email(&mut tx, &email).await?.is_some() {
        let message = "an account uses this email: sign in to it, then link this provider";
        return Err(refused("EMAIL_IN_USE", message));
    }
    let invitation = match ctx.settings.sign_up_mode {
        SignUpMode::Closed => return Err(refused("SIGN_UP_CLOSED", "sign-up is closed")),
        SignUpMode::InviteOnly => Some(
            invitations::open_for_email(&mut tx, email.as_str())
                .await?
                .ok_or_else(|| refused("INVITATION_REQUIRED", "sign-up needs an invitation"))?,
        ),
        SignUpMode::Open => None,
    };

    // Make the user with its identity, and open its session.
    let new = NewUser {
        display_name: display_name(identity.name, &email),
        email,
        email_verified: true,
        handle: None,
        password: None,
    };
    let actor = accounts::insert_user(&mut tx, &new, None).await?;
    let email = Some(new.email.as_str());
    store::link(&mut tx, provider.id, &identity.subject, actor.id, email).await?;
    accounts::record_sign_in(&mut tx, actor.id).await?;
    let opened = sessions::open(&mut tx, actor.id, &methods, client, ttl).await?;
    tx.commit().await?;
    accounts::created(actor.id, &new.email, false);
    accounts::signed_in(actor.id, &provider.slug, opened.session);

    // Accept the invitation of the email.
    if let Some(invitation) = invitation {
        invitations::accept(ctx, Pick::Id(invitation.id), actor.id).await?;
    }
    Ok((actor, opened))
}

/// Links an identity to a signed-in user. The identity of another user is refused.
pub(super) async fn link(
    ctx: &AccountContext,
    provider: &ProviderRow,
    identity: ExternalIdentity,
    user: ActorId,
) -> Result<(), Error> {
    let mut tx = ctx.pool.begin().await?;
    match store::user_of(&mut tx, provider.id, &identity.subject).await? {
        Some(owner) if owner == user => {}
        Some(_) => {
            return Err(refused(
                "IDENTITY_IN_USE",
                "another account uses this identity",
            ))
        }
        None => {
            let email = identity.email.as_deref();
            store::link(&mut tx, provider.id, &identity.subject, user, email).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Returns the name of the provider, or the start of the email.
fn display_name(name: Option<String>, email: &Email) -> Name {
    let local = email.as_str().split('@').next().unwrap_or("user");
    name.and_then(|name| Name::parse(name).ok())
        .or_else(|| Name::parse(local.chars().take(Name::MAX_LEN).collect()).ok())
        .unwrap_or_else(|| Name::parse("User".to_owned()).expect("a valid name"))
}
