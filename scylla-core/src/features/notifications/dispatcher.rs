//! The task of the notifications: it turns each fact into the rows of its recipients, and deletes
//! the old rows.

use std::sync::Arc;
use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_extension::{
    AudienceContext, Collapse, EventRow, Gate, NotificationKind, NotificationKinds, Recipient, Tx,
    REASON_SUBJECT,
};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::task::JoinHandle;

use super::purge;
use super::store::{events, rows};
use crate::Error;

/// The time between two reads of the facts.
const TICK: Duration = Duration::from_secs(5);

/// The time between two deletes of the old rows.
const PURGE_TICK: Duration = Duration::from_secs(60 * 60);

/// The most facts in one pass.
const BATCH: i64 = 100;

/// The attempts after which the task drops a fact.
const MAX_ATTEMPTS: i32 = 10;

/// Starts the task: each 5 seconds it reads the facts, and each hour it deletes the old rows.
pub(crate) fn spawn(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    kinds: Arc<NotificationKinds>,
    retention: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let start = tokio::time::Instant::now();
        let mut ticks = tokio::time::interval_at(start + TICK, TICK);
        let mut purges = tokio::time::interval_at(start + PURGE_TICK, PURGE_TICK);
        loop {
            tokio::select! {
                _ = ticks.tick() => {
                    if let Err(error) = dispatch_pending(&pool, gate.as_ref(), &kinds).await {
                        tracing::warn!(%error, "the notifications wait");
                    }
                }
                _ = purges.tick() => {
                    if let Err(error) = purge::run(&pool, retention).await {
                        tracing::warn!(%error, "the old notifications stay");
                    }
                }
            }
        }
    })
}

/// Runs one pass, and returns the number of facts that it read. A fact that fails stays for a
/// later pass, with a wait that grows.
pub async fn dispatch_pending(
    pool: &PgPool,
    gate: &dyn Gate,
    kinds: &NotificationKinds,
) -> Result<usize, Error> {
    let mut tx = pool.begin().await?;
    let pending = events::pending(&mut tx, BATCH).await?;
    tx.commit().await?;
    let count = pending.len();
    for (row, attempts) in pending {
        let result = dispatch_one(pool, gate, kinds, &row).await;
        if let Err(error) = result {
            tracing::warn!(%error, kind = row.event.kind, "a notification waits");
            retry_or_drop(pool, &row, attempts).await?;
        }
    }
    Ok(count)
}

/// Adds one attempt to a fact with a longer wait, or drops the fact after 10 attempts.
async fn retry_or_drop(pool: &PgPool, row: &EventRow, attempts: i32) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if attempts + 1 >= MAX_ATTEMPTS {
        events::claim(&mut tx, row.id).await?;
        tx.commit().await?;
        drop_event(row, "too many attempts");
        return Ok(());
    }
    let wait = time::Duration::seconds(5 * (1 << attempts.clamp(0, 10)));
    events::retry_at(&mut tx, row.id, OffsetDateTime::now_utc() + wait).await?;
    tx.commit().await?;
    Ok(())
}

/// Logs the event `notification.dropped`.
fn drop_event(row: &EventRow, reason: &'static str) {
    let (id, kind) = (row.id, row.event.kind.as_str());
    tracing::warn!(event = "notification.dropped", %id, kind, reason);
}

/// Finds the recipients of a fact with no open transaction, then writes their rows and marks the
/// fact in one transaction, so two passes cannot both write.
async fn dispatch_one(
    pool: &PgPool,
    gate: &dyn Gate,
    kinds: &NotificationKinds,
    row: &EventRow,
) -> Result<(), Error> {
    // Drop a fact with a kind that this edition does not have.
    let Some(kind) = kinds.get(&row.event.kind) else {
        let mut tx = pool.begin().await?;
        events::claim(&mut tx, row.id).await?;
        tx.commit().await?;
        drop_event(row, "unknown kind");
        return Ok(());
    };

    // Find the recipients.
    let ctx = AudienceContext::new(pool, gate);
    let recipients = kind
        .audience
        .recipients(&ctx, row)
        .await
        .map_err(|error| Error::Internal(error.0))?;
    let recipients = without_actor(kind, row.event.actor, recipients);

    // Mark the fact, and write the row of each active user.
    let mut tx = pool.begin().await?;
    if !events::claim(&mut tx, row.id).await? {
        return Ok(());
    }
    let actors: Vec<ActorId> = recipients.iter().map(|person| person.user).collect();
    let active = rows::active_users(&mut tx, &actors).await?;
    for recipient in recipients {
        if active.contains(&recipient.user) {
            write_row(&mut tx, kind, row, recipient).await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Removes the actor of a fact from its recipients, except a mandatory fact about the actor.
fn without_actor(
    kind: &NotificationKind,
    actor: Option<ActorId>,
    recipients: Vec<Recipient>,
) -> Vec<Recipient> {
    recipients
        .into_iter()
        .filter(|person| {
            Some(person.user) != actor || (kind.mandatory && person.reason == REASON_SUBJECT)
        })
        .collect()
}

/// Writes the row of one recipient, or changes the open row of its group as the kind says.
async fn write_row(
    tx: &mut Tx<'_>,
    kind: &NotificationKind,
    row: &EventRow,
    recipient: Recipient,
) -> Result<(), Error> {
    // Close a row that is too old to merge.
    let event = &row.event;
    let fallback = row.id.to_string();
    let group = event.group.as_deref().unwrap_or(&fallback);
    let mut merge_into = None;
    if let Collapse::Merge { window, merge } = kind.collapse {
        if let Some(mut open) = rows::lock_open(tx, recipient.user, group).await? {
            if OffsetDateTime::now_utc() - open.last_at < window {
                merge(&mut open.params, &event.params);
                merge_into = Some(open);
            } else {
                rows::close_row(tx, open.id).await?;
            }
        }
    }
    if let Some(open) = merge_into {
        return rows::merged(tx, open.id, &open.params, event).await;
    }

    // Insert the row, or follow the rule of the kind for the open row of the group.
    if rows::insert_new(tx, recipient, event, group, kind.needs_action).await? {
        return Ok(());
    }
    match kind.collapse {
        Collapse::Count => rows::count(tx, recipient.user, event, group).await,
        Collapse::Replace => rows::replace(tx, recipient.user, event, group).await,
        Collapse::Keep | Collapse::Merge { .. } => Ok(()),
    }
}
