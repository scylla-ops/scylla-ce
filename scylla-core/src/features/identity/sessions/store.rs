//! The SQL of the `sessions` table. Each function runs in the transaction of the caller.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{SessionId, UserStatus};
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// A session, as a refresh reads it.
pub(super) struct SessionRow {
    /// The user of the session.
    pub(super) user: ActorId,
    /// The state of the account of the user.
    pub(super) status: UserStatus,
    /// The hash of the current refresh secret.
    pub(super) refresh_hash: Vec<u8>,
    /// `true` if the session is closed or expired.
    pub(super) closed: bool,
}

/// An open session, as the user sees it.
pub struct SessionInfo {
    /// The id of the session.
    pub id: SessionId,
    /// How the user signed in.
    pub methods: Vec<String>,
    /// The user agent of the sign-in.
    pub user_agent: Option<String>,
    /// The time of the sign-in.
    pub created_at: OffsetDateTime,
    /// The time of the last refresh.
    pub last_used_at: OffsetDateTime,
}

/// Inserts a session.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    id: SessionId,
    user: ActorId,
    refresh_hash: &[u8],
    methods: &[String],
    user_agent: Option<&str>,
    expires_at: OffsetDateTime,
) -> Result<(), Error> {
    sqlx::query!(
        "INSERT INTO sessions (id, user_id, refresh_hash, methods, user_agent, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)",
        id.to_db(),
        user.to_db(),
        refresh_hash,
        methods,
        user_agent,
        expires_at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Reads and locks a session until the commit, or returns `None` if it is unknown.
pub(super) async fn lock(tx: &mut Tx<'_>, id: SessionId) -> Result<Option<SessionRow>, Error> {
    let row = sqlx::query!(
        r#"SELECT s.user_id, u.status, s.refresh_hash,
                  (s.revoked_at IS NOT NULL OR s.expires_at <= now()) AS "closed!"
           FROM sessions s JOIN users u ON u.id = s.user_id
           WHERE s.id = $1
           FOR UPDATE OF s"#,
        id.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| SessionRow {
        user: ActorId::from_db(row.user_id),
        status: UserStatus::from_name(&row.status),
        refresh_hash: row.refresh_hash,
        closed: row.closed,
    }))
}

/// Replaces the refresh secret of a session.
pub(super) async fn rotate(tx: &mut Tx<'_>, id: SessionId, hash: &[u8]) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE sessions SET refresh_hash = $2, last_used_at = now() WHERE id = $1",
        id.to_db(),
        hash,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Closes a session of a user. Returns `false` if the user has no such open session.
pub(crate) async fn revoke(tx: &mut Tx<'_>, user: ActorId, id: SessionId) -> Result<bool, Error> {
    let result = sqlx::query!(
        "UPDATE sessions SET revoked_at = now()
         WHERE id = $1 AND user_id = $2 AND revoked_at IS NULL",
        id.to_db(),
        user.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Closes each open session of a user, except one if it is given.
pub(crate) async fn revoke_all(
    tx: &mut Tx<'_>,
    user: ActorId,
    except: Option<SessionId>,
) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE sessions SET revoked_at = now()
         WHERE user_id = $1 AND revoked_at IS NULL AND ($2::uuid IS NULL OR id <> $2)",
        user.to_db(),
        except.map(SessionId::to_db),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Returns the open sessions of a user, newest first.
pub(crate) async fn list(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<SessionInfo>, Error> {
    let rows = sqlx::query!(
        "SELECT id, methods, user_agent, created_at, last_used_at FROM sessions
         WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > now()
         ORDER BY id DESC",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SessionInfo {
            id: SessionId::from_db(row.id),
            methods: row.methods,
            user_agent: row.user_agent,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        })
        .collect())
}
