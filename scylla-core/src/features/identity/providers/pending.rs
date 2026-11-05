//! A link that waits for the password of an account. A sign-in with a provider gives a verified
//! email that an account with a password has: no link comes from the email alone, so the person
//! proves the account with its password. It lives 10 minutes in a cookie of the browser of the
//! callback, and it works one time.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{Email, UserStatus};
use scylla_extension::{AccessError, Tx};
use tonic::metadata::MetadataMap;

use super::pending_store::{self, NewPendingLink};
use super::store::{self, ProviderRow};
use crate::features::identity::accounts::{
    self, cookie_origin_is_allowed, mails, refused, verify_password, AccountContext, EmailAccount,
};
use crate::features::identity::sessions::{self, cookie, Client, Opened};
use crate::infra::secret;
use crate::Error;

/// A link that waits, as the web app sees it.
pub(crate) struct PendingView {
    /// The name of the provider.
    pub(crate) provider_name: String,
    /// The verified email that the provider gave.
    pub(crate) email: String,
    /// The name that the provider gave, or an empty text.
    pub(crate) display_name: String,
}

/// The end of a link.
pub(crate) struct Finished {
    /// The new session.
    pub(crate) opened: Opened,
    /// The page of the web app.
    pub(crate) return_to: String,
}

/// An identity with the verified email of an account.
pub(super) struct KnownEmail {
    /// The account that has the email.
    pub(super) account: EmailAccount,
    /// The email.
    pub(super) email: Email,
    /// The id of the person at the provider.
    pub(super) subject: String,
    /// The name that the provider gave.
    pub(super) name: Option<String>,
}

/// Makes the refusal of a known email that a link cannot use.
pub(super) fn email_in_use() -> Error {
    let message = "an account uses this email: sign in to it, then link this provider";
    refused("EMAIL_IN_USE", message)
}

/// Stores a link that waits, and returns the token of its cookie. An account with no password
/// stays refused, and an account that has an identity of this provider keeps that one.
pub(super) async fn start(
    ctx: &AccountContext,
    provider: &ProviderRow,
    known: KnownEmail,
    return_to: &str,
) -> Result<String, Error> {
    if !known.account.has_password {
        return Err(email_in_use());
    }
    if known.account.status == UserStatus::Disabled {
        return Err(refused("ACCOUNT_INACTIVE", "the account is not active"));
    }
    let user = known.account.user;
    let mut tx = ctx.pool.begin().await?;
    refuse_a_second_identity(&mut tx, provider, user).await?;
    let token = secret::token();
    let row = NewPendingLink {
        provider: provider.id,
        subject: &known.subject,
        user,
        email: known.email.as_str(),
        display_name: known.name.as_deref(),
        return_to,
    };
    pending_store::insert_pending_link(&mut tx, &secret::hash(&token), &row).await?;
    tx.commit().await?;
    Ok(token)
}

/// Reads the token of the cookie of a request. A cookie needs an allowed web origin, so another
/// site cannot use it. A browser with no cookie gets `NOT_FOUND`.
pub(crate) fn token_of(ctx: &AccountContext, metadata: &MetadataMap) -> Result<String, Error> {
    if !cookie_origin_is_allowed(ctx, metadata) {
        let reason = "this cookie needs an allowed web origin".to_owned();
        return Err(AccessError::Denied(reason).into());
    }
    let headers = metadata
        .get_all("cookie")
        .iter()
        .filter_map(|value| value.to_str().ok());
    cookie::read(headers, cookie::PENDING).ok_or(Error::NotFound("pending link"))
}

/// Returns the link that waits, or `NOT_FOUND`.
pub(crate) async fn read(ctx: &AccountContext, token: &str) -> Result<PendingView, Error> {
    let mut tx = ctx.pool.begin().await?;
    let row = pending_store::find_pending_link(&mut tx, &secret::hash(token)).await?;
    tx.commit().await?;
    let row = row.ok_or(Error::NotFound("pending link"))?;
    Ok(PendingView {
        provider_name: row.provider.display_name,
        email: row.email,
        display_name: row.display_name.unwrap_or_default(),
    })
}

/// Links the identity to its account with the password of the account, and opens a session. A
/// wrong password counts for the lockout of a sign-in, and the link still waits.
pub(crate) async fn link_with_password(
    ctx: &AccountContext,
    token: &str,
    password: String,
    client: &Client,
) -> Result<Finished, Error> {
    // Find the link, then check the password of its account.
    let hash = secret::hash(token);
    let mut tx = ctx.pool.begin().await?;
    let row = pending_store::find_pending_link(&mut tx, &hash).await?;
    tx.commit().await?;
    let row = row.ok_or(Error::NotFound("pending link"))?;
    let user = row.user;
    match verify_password(ctx, row.email.clone(), &user.to_string(), password).await {
        Ok(owner) if owner == user => {}
        Ok(_) | Err(Error::Access(AccessError::Unauthenticated(_))) => {
            return Err(refused("WRONG_PASSWORD", "the password is not valid"));
        }
        Err(error) => return Err(error),
    }

    // Take the link one time, link the identity, and open the session.
    let mut tx = ctx.pool.begin().await?;
    if !pending_store::delete_pending_link(&mut tx, &hash).await? {
        return Err(Error::NotFound("pending link"));
    }
    let provider = &row.provider;
    match store::user_of(&mut tx, provider.id, &row.subject).await? {
        Some(owner) if owner == user => {}
        Some(_) => {
            let message = "another account uses this identity";
            return Err(refused("IDENTITY_IN_USE", message));
        }
        None => {
            refuse_a_second_identity(&mut tx, provider, user).await?;
            let email = Some(row.email.as_str());
            store::link(&mut tx, provider.id, &row.subject, user, email).await?;
        }
    }
    accounts::record_sign_in(&mut tx, user).await?;
    let methods = ["password".to_owned(), provider.slug.clone()];
    let ttl = ctx.settings.session_ttl;
    let opened = sessions::open(&mut tx, user, &methods, client, ttl).await?;
    tx.commit().await?;
    accounts::signed_in(user, &provider.slug, opened.session);
    tracing::info!(event = "account.identity_linked", %user, provider = provider.slug);
    let mail = mails::identity_linked(ctx, &row.email, &provider.display_name);
    mails::send(ctx, "identity-linked", mail).await;
    Ok(Finished {
        opened,
        return_to: row.return_to,
    })
}

/// Refuses a new identity for an account that has an identity of this provider: the same email at
/// the provider with another subject can be another person.
async fn refuse_a_second_identity(
    tx: &mut Tx<'_>,
    provider: &ProviderRow,
    user: ActorId,
) -> Result<(), Error> {
    if store::has_identity_of(tx, user, provider.id).await? {
        let message = "this account already signs in with this provider";
        return Err(refused("ALREADY_LINKED", message));
    }
    Ok(())
}
