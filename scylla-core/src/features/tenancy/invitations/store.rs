//! The SQL of the `invitations` table. Each function runs in the transaction of the caller. The
//! times come from the clock of Postgres.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::tenancy::invitations::{Invitation, InvitationId};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::PageRequest;
use scylla_extension::{Rejection, Tx};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

/// The lifetime of a link, in days.
const LIFETIME_DAYS: i32 = 7;

/// The shortest time between two mails of one invitation, in seconds.
const COOLDOWN_SECS: f64 = 60.0;

/// The unique index of the open invitations of an email in a workspace.
const ONE_OPEN: &str = "invitations_one_open_idx";

/// The foreign key from an invitation to its workspace.
const WORKSPACE_FOREIGN_KEY: &str = "invitations_workspace_id_fkey";

/// A row of the `invitations` table, with no token.
struct Row {
    /// The id.
    id: Uuid,
    /// The workspace.
    workspace_id: Uuid,
    /// The email.
    email: String,
    /// The role.
    role: String,
    /// The actor that invited.
    invited_by: Uuid,
    /// The end of the validity of the link.
    expires_at: OffsetDateTime,
    /// The time of the last mail.
    sent_at: OffsetDateTime,
}

impl Row {
    /// Makes the invitation of the row.
    fn into_invitation(self) -> Invitation {
        Invitation {
            id: InvitationId::from_db(self.id),
            workspace_id: WorkspaceId::from_db(self.workspace_id),
            email: self.email,
            role: self.role,
            invited_by: ActorId::from_db(self.invited_by),
            expires_at: self.expires_at,
            sent_at: self.sent_at,
        }
    }
}

/// Inserts an invitation with the hash of its token, sent now, and returns it. An open
/// invitation of the email in the workspace gives `INVITATION_PENDING`, and an unknown
/// workspace `NotFound("workspace")`.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    email: &str,
    role: &str,
    invited_by: ActorId,
    token_hash: &[u8],
) -> Result<Invitation, Error> {
    let row = sqlx::query_as!(
        Row,
        "INSERT INTO invitations
             (id, workspace_id, email, role, token_hash, invited_by, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, now() + make_interval(days => $7))
         RETURNING id, workspace_id, email, role, invited_by, expires_at, sent_at",
        InvitationId::generate().to_db(),
        workspace.to_db(),
        email,
        role,
        token_hash,
        invited_by.to_db(),
        LIFETIME_DAYS,
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| match error {
        sqlx::Error::Database(db) if db.constraint() == Some(ONE_OPEN) => {
            let message = "the email has an open invitation: send it again";
            Error::Rejected(Rejection::new("INVITATION_PENDING", message))
        }
        sqlx::Error::Database(db) if db.constraint() == Some(WORKSPACE_FOREIGN_KEY) => {
            Error::NotFound("workspace")
        }
        error => error.into(),
    })?;
    Ok(row.into_invitation())
}

/// Returns the open invitation of a token hash: not accepted and not expired.
pub(super) async fn find_open(
    tx: &mut Tx<'_>,
    token_hash: &[u8],
) -> Result<Option<Invitation>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, email, role, invited_by, expires_at, sent_at FROM invitations
         WHERE token_hash = $1 AND accepted_at IS NULL AND expires_at > now()",
        token_hash
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_invitation))
}

/// Returns the open invitations for an email, oldest first.
pub(super) async fn all_open_for_email(
    tx: &mut Tx<'_>,
    email: &str,
) -> Result<Vec<Invitation>, Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, email, role, invited_by, expires_at, sent_at FROM invitations
         WHERE email = $1 AND accepted_at IS NULL AND expires_at > now()
         ORDER BY id",
        email
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_invitation).collect())
}

/// Returns the user with the verified email of an open invitation.
pub(super) async fn invitee(tx: &mut Tx<'_>, id: InvitationId) -> Result<Option<ActorId>, Error> {
    let user = sqlx::query_scalar!(
        "SELECT e.user_id FROM invitations AS i
         JOIN user_emails AS e ON e.email = i.email AND e.verified_at IS NOT NULL
         WHERE i.id = $1 AND i.accepted_at IS NULL AND i.expires_at > now()",
        id.to_db(),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(user.map(ActorId::from_db))
}

/// Returns the oldest open invitation for an email, or `None`.
pub(super) async fn open_for_email(
    tx: &mut Tx<'_>,
    email: &str,
) -> Result<Option<Invitation>, Error> {
    Ok(all_open_for_email(tx, email).await?.into_iter().next())
}

/// Returns the invitations of a workspace that nobody accepted, expired ones too, after the
/// page id, in id order, with one extra row.
pub(super) async fn list_open(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    page: PageRequest<InvitationId>,
) -> Result<Vec<Invitation>, Error> {
    let rows = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, email, role, invited_by, expires_at, sent_at FROM invitations
         WHERE workspace_id = $1 AND accepted_at IS NULL AND ($2::uuid IS NULL OR id > $2)
         ORDER BY id
         LIMIT $3",
        workspace.to_db(),
        page.after.map(InvitationId::to_db),
        page.size.get() as i64 + 1,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows.into_iter().map(Row::into_invitation).collect())
}

/// Locks the open invitation of a token hash or of an id, not accepted and not expired, and
/// returns it, or `None`.
pub(super) async fn lock_acceptable(
    tx: &mut Tx<'_>,
    token_hash: Option<&[u8]>,
    id: Option<InvitationId>,
) -> Result<Option<Invitation>, Error> {
    let row = sqlx::query_as!(
        Row,
        "SELECT id, workspace_id, email, role, invited_by, expires_at, sent_at FROM invitations
         WHERE (token_hash = $1 OR id = $2) AND accepted_at IS NULL AND expires_at > now()
         FOR UPDATE",
        token_hash,
        id.map(InvitationId::to_db),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(Row::into_invitation))
}

/// Locks an invitation of a workspace that nobody accepted, and returns it with the whole
/// seconds to wait for its next mail, or `None`.
pub(super) async fn lock_for_resend(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    id: InvitationId,
) -> Result<Option<(Invitation, Option<i64>)>, Error> {
    let row = sqlx::query!(
        r#"SELECT id, workspace_id, email, role, invited_by, expires_at, sent_at,
                  CASE WHEN sent_at > now() - make_interval(secs => $3)
                       THEN ceil(extract(epoch FROM
                                 sent_at + make_interval(secs => $3) - now()))::bigint
                  END AS "wait?"
           FROM invitations
           WHERE id = $1 AND workspace_id = $2 AND accepted_at IS NULL
           FOR UPDATE"#,
        id.to_db(),
        workspace.to_db(),
        COOLDOWN_SECS,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        let invitation = Row {
            id: row.id,
            workspace_id: row.workspace_id,
            email: row.email,
            role: row.role,
            invited_by: row.invited_by,
            expires_at: row.expires_at,
            sent_at: row.sent_at,
        };
        (invitation.into_invitation(), row.wait)
    }))
}

/// Gives an invitation a new token hash and a new expiry, sent now, and returns it.
pub(super) async fn renew(
    tx: &mut Tx<'_>,
    id: InvitationId,
    token_hash: &[u8],
) -> Result<Invitation, Error> {
    let row = sqlx::query_as!(
        Row,
        "UPDATE invitations
         SET token_hash = $2, expires_at = now() + make_interval(days => $3), sent_at = now()
         WHERE id = $1
         RETURNING id, workspace_id, email, role, invited_by, expires_at, sent_at",
        id.to_db(),
        token_hash,
        LIFETIME_DAYS,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.into_invitation())
}

/// Deletes an invitation of a workspace that nobody accepted. Returns `false` if there is none.
pub(super) async fn delete_open(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    id: InvitationId,
) -> Result<bool, Error> {
    let deleted = sqlx::query!(
        "DELETE FROM invitations WHERE id = $1 AND workspace_id = $2 AND accepted_at IS NULL",
        id.to_db(),
        workspace.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(deleted.rows_affected() == 1)
}

/// Marks an invitation that nobody accepted as accepted by a user. Returns `false` if there is
/// none.
pub(super) async fn mark_accepted(
    tx: &mut Tx<'_>,
    id: InvitationId,
    user: ActorId,
) -> Result<bool, Error> {
    let marked = sqlx::query!(
        "UPDATE invitations SET accepted_by = $2, accepted_at = now()
         WHERE id = $1 AND accepted_at IS NULL",
        id.to_db(),
        user.to_db(),
    )
    .execute(&mut **tx)
    .await?;
    Ok(marked.rows_affected() == 1)
}

/// Returns the name of a workspace, for the mail.
pub(super) async fn workspace_name(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
) -> Result<Option<String>, Error> {
    let name = sqlx::query_scalar!(
        "SELECT name FROM workspaces WHERE id = $1",
        workspace.to_db()
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(name)
}
