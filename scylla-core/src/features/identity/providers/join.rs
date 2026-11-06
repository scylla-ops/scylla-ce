//! What an identity of a provider does: sign in its user, make a new user, wait for the password
//! of the account with its email, or link to the signed-in user. A known email is never linked
//! automatically.

use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::{Email, NewUser, UserStatus, KIND};
use scylla_domain::Name;

use super::pending::{self, KnownEmail};
use super::store::{self, ProviderRow};
use super::ExternalIdentity;
use crate::features::identity::accounts::{self, mails, refused, AccountContext};
use crate::features::identity::sessions::{self, Client, Opened};
use crate::features::notifications::{self, kinds};
use crate::features::tenancy::invitations::{self, Pick};
use crate::{Error, SignUpMode};

/// The end of a sign-in in a browser.
pub(super) enum Joined {
    /// A session is open.
    SignedIn(Opened),
    /// The link waits for the password of the account, with the token of its cookie.
    Waits(String),
}

/// What an identity gives before a page can ask for a password.
enum Found {
    /// A session is open.
    SignedIn(Actor, Opened),
    /// An account has the verified email of the identity.
    KnownEmail(KnownEmail),
}

/// Ends a sign-in in a browser: a known email waits for the password of its account.
pub(super) async fn browser_sign_in(
    ctx: &AccountContext,
    provider: &ProviderRow,
    identity: ExternalIdentity,
    return_to: &str,
    client: &Client,
) -> Result<Joined, Error> {
    match find(ctx, provider, identity, client).await? {
        Found::SignedIn(_, opened) => Ok(Joined::SignedIn(opened)),
        Found::KnownEmail(known) => {
            let token = pending::start(ctx, provider, known, return_to).await?;
            Ok(Joined::Waits(token))
        }
    }
}

/// Signs in the user of an identity for a native client, or makes a user as the provider and the
/// sign-up mode allow. It has no page for a password, so a known email stays refused.
pub(super) async fn sign_in(
    ctx: &AccountContext,
    provider: &ProviderRow,
    identity: ExternalIdentity,
    client: &Client,
) -> Result<(Actor, Opened), Error> {
    match find(ctx, provider, identity, client).await? {
        Found::SignedIn(actor, opened) => Ok((actor, opened)),
        Found::KnownEmail(_) => Err(pending::email_in_use()),
    }
}

/// Signs in the user of an identity, or makes a user as the provider and the sign-up mode allow. A
/// new user needs a verified email that no account has.
async fn find(
    ctx: &AccountContext,
    provider: &ProviderRow,
    identity: ExternalIdentity,
    client: &Client,
) -> Result<Found, Error> {
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
        let actor = Actor {
            id: user,
            kind: KIND,
        };
        return Ok(Found::SignedIn(actor, opened));
    }

    // An account with the verified email waits for its password: this makes no new account.
    let verified = identity
        .email
        .filter(|_| identity.email_verified)
        .and_then(|email| Email::parse(email).ok());
    if let Some(email) = &verified {
        if let Some(account) = accounts::account_of_email(&mut tx, email).await? {
            return Ok(Found::KnownEmail(KnownEmail {
                account,
                email: email.clone(),
                subject: identity.subject,
                name: identity.name,
            }));
        }
    }

    // Check that a new user can come from this identity.
    if !provider.allow_sign_up {
        return Err(refused("NO_ACCOUNT", "no account uses this identity"));
    }
    let email =
        verified.ok_or_else(|| refused("EMAIL_REQUIRED", "the provider gave no verified email"))?;
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
    Ok(Found::SignedIn(actor, opened))
}

/// Links an identity to a signed-in user, and tells the user by mail. The identity of another user
/// is refused. The same link again adds nothing, and sends no mail.
pub(super) async fn link(
    ctx: &AccountContext,
    provider: &ProviderRow,
    identity: ExternalIdentity,
    user: ActorId,
) -> Result<(), Error> {
    let mut added = false;
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
            let fact = kinds::about(kinds::IDENTITY_LINKED, user)
                .param("provider", provider.display_name.clone());
            notifications::emit(&mut tx, fact).await?;
            added = true;
        }
    }
    let to = accounts::find_user(&mut tx, user)
        .await?
        .and_then(|user| user.email);
    tx.commit().await?;
    if added {
        tracing::info!(event = "account.identity_linked", %user, provider = provider.slug);
        if let Some(to) = to {
            let mail = mails::identity_linked(ctx, &to, &provider.display_name);
            mails::send(ctx, "identity-linked", mail).await;
        }
    }
    Ok(())
}

/// Returns the name of the provider, or the start of the email.
fn display_name(name: Option<String>, email: &Email) -> Name {
    let local = email.as_str().split('@').next().unwrap_or("user");
    name.and_then(|name| Name::parse(name).ok())
        .or_else(|| Name::parse(local.chars().take(Name::MAX_LEN).collect()).ok())
        .unwrap_or_else(|| Name::parse("User".to_owned()).expect("a valid name"))
}
