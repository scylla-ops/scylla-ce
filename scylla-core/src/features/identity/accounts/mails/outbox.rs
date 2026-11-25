//! The task of the mail outbox: the only task that sends the mails. It takes each due mail, sends
//! it, and keeps the result: sent, due again after a transient error, or dropped.

use std::sync::Arc;
use std::time::Duration;

use scylla_extension::{MailError, MailReceipt, Mailer};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use super::store::{self, Claimed};
use crate::Error;

/// The time between two passes.
const TICK: Duration = Duration::from_secs(1);

/// The time between two deletes of the old rows.
const PURGE_TICK: Duration = Duration::from_secs(60 * 60);

/// The most mails of one pass.
const BATCH: usize = 50;

/// The time that a pass holds a mail while the mailer sends it, in seconds.
const LEASE_SECONDS: f64 = 300.0;

/// The wait after the first failed send.
const FIRST_WAIT: time::Duration = time::Duration::seconds(10);

/// The longest wait between two sends.
const LONGEST_WAIT: time::Duration = time::Duration::minutes(15);

/// The time after which a transient error drops the mail.
const MAX_AGE: time::Duration = time::Duration::hours(24);

/// The time that the outbox keeps a sent or dropped mail, with no content.
const KEEP: time::Duration = time::Duration::days(30);

/// Starts the task: each second it sends the due mails, and each hour it deletes the old rows.
pub(crate) fn spawn(pool: PgPool, mailer: Arc<dyn Mailer>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let start = tokio::time::Instant::now();
        let mut ticks = tokio::time::interval(TICK);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut purges = tokio::time::interval_at(start + PURGE_TICK, PURGE_TICK);
        loop {
            tokio::select! {
                _ = ticks.tick() => {
                    if let Err(error) = pass(&pool, mailer.as_ref()).await {
                        tracing::warn!(%error, "the mails wait");
                    }
                }
                _ = purges.tick() => {
                    if let Err(error) = purge(&pool).await {
                        tracing::warn!(%error, "the old mails stay");
                    }
                }
            }
        }
    })
}

/// Sends the due mails, one at a time, at most `BATCH`.
async fn pass(pool: &PgPool, mailer: &dyn Mailer) -> Result<(), Error> {
    for _ in 0..BATCH {
        let mut tx = pool.begin().await?;
        let claimed = store::claim(&mut tx, LEASE_SECONDS).await?;
        tx.commit().await?;
        let Some(claimed) = claimed else {
            return Ok(());
        };
        deliver(pool, mailer, &claimed).await?;
    }
    Ok(())
}

/// Sends one mail, keeps the result, and logs it with the kind, the masked recipient and the
/// number of attempts. A sent mail also logs its `Message-ID` and the answer of the mail server.
async fn deliver(pool: &PgPool, mailer: &dyn Mailer, claimed: &Claimed) -> Result<(), Error> {
    // Send the mail. A content that the outbox cannot read is a permanent error.
    let result = match claimed.mail() {
        Ok(mail) => {
            let address = mail.to.clone();
            let mask = |text: String| text.replace(&address, &claimed.recipient);
            match mailer.send_traced(mail).await {
                Ok(receipt) => Ok(MailReceipt {
                    reply: receipt.reply.map(mask),
                    ..receipt
                }),
                Err(error) => Err(MailError {
                    reason: mask(error.reason),
                    ..error
                }),
            }
        }
        Err(reason) => Err(MailError::permanent(reason)),
    };

    // Keep a sent mail, with no content.
    let (kind, to) = (claimed.kind.as_str(), claimed.recipient.as_str());
    let attempts = claimed.attempts + 1;
    let error = match result {
        Ok(MailReceipt { message_id, reply }) => {
            let mut tx = pool.begin().await?;
            store::sent(&mut tx, claimed.id).await?;
            tx.commit().await?;
            let (message_id, reply) = (message_id.as_deref(), reply.as_deref());
            tracing::info!(event = "mail.sent", kind, to, attempts, message_id, reply);
            return Ok(());
        }
        Err(error) => error,
    };

    // Drop the mail after a permanent error or after 24 hours, else make it due again later.
    tracing::warn!(
        event = "mail.failed",
        kind,
        to,
        attempts,
        error = error.reason
    );
    let now = OffsetDateTime::now_utc();
    let mut tx = pool.begin().await?;
    if error.permanent || now - claimed.created_at >= MAX_AGE {
        store::dropped(&mut tx, claimed.id).await?;
        tx.commit().await?;
        let reason = if error.permanent {
            "refused"
        } else {
            "too old"
        };
        tracing::warn!(event = "mail.dropped", kind, to, attempts, reason);
        return Ok(());
    }
    store::retry(&mut tx, claimed.id, now + wait(claimed.attempts)).await?;
    tx.commit().await?;
    Ok(())
}

/// Deletes the sent and dropped mails older than 30 days.
async fn purge(pool: &PgPool) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    store::purge(&mut tx, OffsetDateTime::now_utc() - KEEP).await?;
    tx.commit().await?;
    Ok(())
}

/// Returns the wait after a failed send, for a mail with `attempts` failed sends before it: 10
/// seconds, then twice as long after each failure, at most 15 minutes.
fn wait(attempts: i32) -> time::Duration {
    let doubled = 2_i32.saturating_pow(u32::try_from(attempts.clamp(0, 16)).unwrap_or(16));
    (FIRST_WAIT * doubled).min(LONGEST_WAIT)
}

#[cfg(test)]
mod tests {
    use super::wait;

    /// The wait starts at 10 seconds, doubles after each failure, and stops at 15 minutes.
    #[test]
    fn the_wait_doubles_up_to_15_minutes() {
        // given
        let attempts = [0, 1, 2, 6, 7, 40];

        // when
        let waits: Vec<i64> = attempts
            .into_iter()
            .map(|count| wait(count).whole_seconds())
            .collect();

        // then
        assert_eq!(waits, [10, 20, 40, 640, 900, 900]);
    }
}
