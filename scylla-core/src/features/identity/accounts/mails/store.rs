//! The SQL of the mail outbox: a new mail, the due mail that one pass takes, the result of its
//! send, and the delete of the old rows. A sent or dropped mail keeps no content.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use scylla_extension::{Mail, MailImage, Tx};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::infra::logs::mask_email;
use crate::Error;

/// The content of a mail in the outbox: what the mailer needs.
#[derive(Serialize, Deserialize)]
struct Content {
    /// The address of the recipient.
    to: String,
    /// The subject.
    subject: String,
    /// The text body.
    body: String,
    /// The HTML body.
    html: String,
    /// The images of the HTML body.
    images: Vec<Image>,
}

/// An image of the HTML body, with its bytes in base64.
#[derive(Serialize, Deserialize)]
struct Image {
    /// The content id.
    id: String,
    /// The media type.
    content_type: String,
    /// The bytes, in base64.
    bytes: String,
}

/// A due mail that a pass took. Its time is the end of the lease, so no other pass takes it.
pub(super) struct Claimed {
    /// The id of the row.
    pub(super) id: Uuid,
    /// The kind of the mail, for example `verify-email`.
    pub(super) kind: String,
    /// The masked recipient.
    pub(super) recipient: String,
    /// The content, as the outbox keeps it.
    content: Option<serde_json::Value>,
    /// The failed sends before this one.
    pub(super) attempts: i32,
    /// The time when the change put the mail in the outbox.
    pub(super) created_at: OffsetDateTime,
}

impl Claimed {
    /// Returns the mail to send, or why the outbox cannot give it.
    pub(super) fn mail(&self) -> Result<Mail, String> {
        let content = self.content.clone().ok_or("the mail has no content")?;
        let content: Content = serde_json::from_value(content)
            .map_err(|error| format!("the content is not valid: {error}"))?;
        let images = content
            .images
            .into_iter()
            .map(|image| {
                let bytes = STANDARD
                    .decode(&image.bytes)
                    .map_err(|error| error.to_string())?;
                Ok(MailImage {
                    id: image.id,
                    content_type: image.content_type,
                    bytes,
                })
            })
            .collect::<Result<_, String>>()?;
        Ok(Mail {
            to: content.to,
            subject: content.subject,
            body: content.body,
            html: content.html,
            images,
        })
    }
}

/// Puts a mail in the outbox, due at once.
pub(super) async fn insert(tx: &mut Tx<'_>, kind: &str, mail: Mail) -> Result<(), Error> {
    let recipient = mask_email(&mail.to);
    let content = Content {
        to: mail.to,
        subject: mail.subject,
        body: mail.body,
        html: mail.html,
        images: mail
            .images
            .into_iter()
            .map(|image| Image {
                id: image.id,
                content_type: image.content_type,
                bytes: STANDARD.encode(image.bytes),
            })
            .collect(),
    };
    let content = serde_json::to_value(content)
        .map_err(|error| Error::Internal(format!("the mail is not valid: {error}")))?;
    sqlx::query!(
        "INSERT INTO mail_outbox (id, kind, recipient, content, next_attempt_at)
         VALUES ($1, $2, $3, $4, now())",
        Uuid::now_v7(),
        kind,
        recipient,
        content,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Takes the oldest due mail, and moves its time to the end of the lease.
pub(super) async fn claim(tx: &mut Tx<'_>, lease_seconds: f64) -> Result<Option<Claimed>, Error> {
    let claimed = sqlx::query_as!(
        Claimed,
        "UPDATE mail_outbox m
         SET next_attempt_at = now() + make_interval(secs => $1), updated_at = now()
         FROM (SELECT id FROM mail_outbox
               WHERE state = 'pending' AND next_attempt_at <= now()
               ORDER BY next_attempt_at
               LIMIT 1
               FOR UPDATE SKIP LOCKED) due
         WHERE m.id = due.id
         RETURNING m.id, m.kind, m.recipient, m.content, m.attempts, m.created_at",
        lease_seconds,
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(claimed)
}

/// Marks a mail as sent, and clears its content.
pub(super) async fn sent(tx: &mut Tx<'_>, id: Uuid) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE mail_outbox
         SET state = 'sent', content = NULL, next_attempt_at = NULL, attempts = attempts + 1,
             updated_at = now()
         WHERE id = $1",
        id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Keeps a failed send, and makes the mail due again at a later time.
pub(super) async fn retry(tx: &mut Tx<'_>, id: Uuid, at: OffsetDateTime) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE mail_outbox
         SET next_attempt_at = $2, attempts = attempts + 1, updated_at = now()
         WHERE id = $1",
        id,
        at,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Marks a mail as dropped, and clears its content.
pub(super) async fn dropped(tx: &mut Tx<'_>, id: Uuid) -> Result<(), Error> {
    sqlx::query!(
        "UPDATE mail_outbox
         SET state = 'dropped', content = NULL, next_attempt_at = NULL, attempts = attempts + 1,
             updated_at = now()
         WHERE id = $1",
        id,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Deletes the sent and dropped mails that did not change after a time, and returns their
/// number.
pub(super) async fn purge(tx: &mut Tx<'_>, before: OffsetDateTime) -> Result<u64, Error> {
    let deleted = sqlx::query!(
        "DELETE FROM mail_outbox WHERE state <> 'pending' AND updated_at < $1",
        before,
    )
    .execute(&mut **tx)
    .await?;
    Ok(deleted.rows_affected())
}
