//! The logic of the sessions: open, refresh with rotation, and close. A refresh token is
//! `<session id>.<secret>`, and the database keeps the hash of the secret only.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{SessionId, UserStatus};
use scylla_extension::{AccessError, Tx};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::{store, Client, SessionInfo};
use crate::infra::secret;
use crate::Error;

/// The refresh token of a session, new or rotated.
pub struct Opened {
    /// The session.
    pub session: SessionId,
    /// The refresh token, to give to the client one time.
    pub refresh: String,
}

/// Opens a session for a user, in the transaction of the sign-in.
pub(crate) async fn open(
    tx: &mut Tx<'_>,
    user: ActorId,
    methods: &[String],
    client: &Client,
    ttl: Duration,
) -> Result<Opened, Error> {
    let session = SessionId::generate();
    let secret = secret::token();
    let row = store::NewSession {
        id: session,
        user,
        refresh_hash: secret::hash(&secret),
        methods,
        client,
        expires_at: OffsetDateTime::now_utc() + ttl,
    };
    store::insert(tx, &row).await?;
    Ok(Opened {
        session,
        refresh: format!("{session}.{secret}"),
    })
}

/// Rotates the refresh token of a session, and returns the user and the new token. A refresh
/// token that is used a second time closes its session: someone else has a copy of it.
pub(crate) async fn refresh(pool: &PgPool, refresh: &str) -> Result<(ActorId, Opened), Error> {
    // Read and lock the session of the token.
    let (session, secret) = parse(refresh)?;
    let mut tx = pool.begin().await?;
    let row = store::lock(&mut tx, session).await?.ok_or_else(closed)?;
    if row.closed || row.status != UserStatus::Active {
        return Err(closed());
    }

    // Close the session if the secret is an old one.
    if secret::hash(secret) != row.refresh_hash {
        store::revoke(&mut tx, row.user, session).await?;
        tx.commit().await?;
        let user = row.user;
        tracing::warn!(event = "session.revoked", %user, %session, scope = "reuse");
        return Err(closed());
    }

    // Give a new secret, and stop the old one.
    let next = secret::token();
    store::rotate(&mut tx, session, &secret::hash(&next)).await?;
    tx.commit().await?;
    let opened = Opened {
        session,
        refresh: format!("{session}.{next}"),
    };
    Ok((row.user, opened))
}

/// Returns the user of a valid refresh token, with no rotation: the session cookie of a
/// browser shows who signed in.
pub(crate) async fn user_of(pool: &PgPool, refresh: &str) -> Result<Option<ActorId>, Error> {
    let Ok((session, secret)) = parse(refresh) else {
        return Ok(None);
    };
    let mut tx = pool.begin().await?;
    let row = store::lock(&mut tx, session).await?;
    tx.commit().await?;
    Ok(row
        .filter(|row| !row.closed && row.status == UserStatus::Active)
        .filter(|row| secret::hash(secret) == row.refresh_hash)
        .map(|row| row.user))
}

/// Closes the session of a refresh token. An unknown or closed session is not an error.
pub(crate) async fn close(pool: &PgPool, refresh: &str) -> Result<(), Error> {
    let (session, secret) = parse(refresh)?;
    let mut tx = pool.begin().await?;
    let mut revoked = None;
    if let Some(row) = store::lock(&mut tx, session).await? {
        if secret::hash(secret) == row.refresh_hash
            && store::revoke(&mut tx, row.user, session).await?
        {
            revoked = Some(row.user);
        }
    }
    tx.commit().await?;
    if let Some(user) = revoked {
        tracing::info!(event = "session.revoked", %user, %session, scope = "sign_out");
    }
    Ok(())
}

/// Returns the open sessions of a user, newest first.
pub(crate) async fn of_user(pool: &PgPool, user: ActorId) -> Result<Vec<SessionInfo>, Error> {
    let mut tx = pool.begin().await?;
    let open = store::list(&mut tx, user).await?;
    tx.commit().await?;
    Ok(open)
}

/// Closes one session of a user. A session that is not an open session of the user gives
/// `NotFound("session")`.
pub(crate) async fn close_one(pool: &PgPool, user: ActorId, id: SessionId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !store::revoke(&mut tx, user, id).await? {
        return Err(Error::NotFound("session"));
    }
    tx.commit().await?;
    tracing::info!(event = "session.revoked", %user, session = %id, scope = "one");
    Ok(())
}

/// Reads the session and the secret of a refresh token.
fn parse(refresh: &str) -> Result<(SessionId, &str), Error> {
    let (session, secret) = refresh.split_once('.').ok_or_else(closed)?;
    let session = SessionId::parse(session).map_err(|_| closed())?;
    Ok((session, secret))
}

/// The error of a refresh token that does not give a session.
fn closed() -> Error {
    AccessError::Unauthenticated("the session is closed or the refresh token is invalid".to_owned())
        .into()
}
