//! The logic of the triggers that people call: one transaction for each call.

use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{
    ListTriggers, NewTrigger, Trigger, TriggerChange, TriggerId, TriggerKind, TriggerPage,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::Rejection;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::schedule;
use super::store::{deliveries, triggers};
use crate::infra::secret::{self, Sealer};
use crate::Error;

/// Creates a trigger. A webhook trigger gets a new secret, encrypted with the id of the
/// trigger, and the secret in clear comes back one time.
pub(crate) async fn create(
    pool: &PgPool,
    sealer: Option<&Sealer>,
    new: NewTrigger,
) -> Result<(Trigger, Option<String>), Error> {
    // Check the schedule, or make the secret of the webhook.
    let id = TriggerId::generate();
    let (sealed, secret, next) = match &new.kind {
        TriggerKind::Cron(text) => {
            schedule::check(text)?;
            (None, None, schedule::next(text, OffsetDateTime::now_utc()))
        }
        TriggerKind::Webhook => {
            let sealer = sealer.ok_or_else(|| {
                let message = "the server has no key for the secrets";
                Error::Rejected(Rejection::new("SECRET_KEY_MISSING", message))
            })?;
            let secret = hex::encode(secret::hash(&secret::token()));
            (
                Some(sealer.seal_with(secret.as_bytes(), &id.to_bytes())),
                Some(secret),
                None,
            )
        }
    };

    // Store the trigger.
    let mut tx = pool.begin().await?;
    let trigger = triggers::insert(&mut tx, id, &new, sealed, next)
        .await?
        .ok_or(Error::NotFound("pipeline"))?;
    tx.commit().await?;
    tracing::info!(event = "trigger.created", trigger = %trigger.id, pipeline = %trigger.pipeline_id);
    Ok((trigger, secret))
}

/// Returns a trigger, or `None` if no trigger has this id.
pub(crate) async fn get(pool: &PgPool, id: TriggerId) -> Result<Option<Trigger>, Error> {
    let mut tx = pool.begin().await?;
    let trigger = triggers::find(&mut tx, id).await?;
    tx.commit().await?;
    Ok(trigger)
}

/// Returns the project and the workspace of a trigger, or `None` if no trigger has this id.
pub(crate) async fn place(
    pool: &PgPool,
    id: TriggerId,
) -> Result<Option<(ProjectId, WorkspaceId)>, Error> {
    let mut tx = pool.begin().await?;
    let place = triggers::place(&mut tx, id).await?;
    tx.commit().await?;
    Ok(place)
}

/// Returns one page of the triggers of a pipeline, oldest first.
pub(crate) async fn list(pool: &PgPool, query: ListTriggers) -> Result<TriggerPage, Error> {
    let mut tx = pool.begin().await?;
    let items = triggers::list(&mut tx, query.pipeline_id, query.page).await?;
    tx.commit().await?;
    Ok(Page::from_overfetch(items, query.page.size, |trigger| {
        trigger.id
    }))
}

/// Changes the name, the schedule or the inputs of a trigger. A schedule of a webhook trigger
/// gives `INVALID_ARGUMENT`.
pub(crate) async fn update(
    pool: &PgPool,
    id: TriggerId,
    change: TriggerChange,
) -> Result<Trigger, Error> {
    let mut tx = pool.begin().await?;
    let mut trigger = triggers::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("trigger"))?;
    if let Some(name) = change.name {
        trigger.name = name.as_str().to_owned();
    }
    if let Some(text) = change.schedule {
        let TriggerKind::Cron(current) = &mut trigger.kind else {
            return Err(Error::invalid(
                "schedule",
                "only a cron trigger has a schedule",
            ));
        };
        schedule::check(&text)?;
        trigger.next_fire_at = schedule::next(&text, OffsetDateTime::now_utc());
        *current = text;
    }
    if let Some(inputs) = change.inputs {
        trigger.inputs = inputs.as_slice().to_vec();
    }
    triggers::save(&mut tx, &trigger).await?;
    tx.commit().await?;
    get(pool, id).await?.ok_or(Error::NotFound("trigger"))
}

/// Starts or stops a trigger. A schedule that starts again gets its next time after now.
pub(crate) async fn set_enabled(
    pool: &PgPool,
    id: TriggerId,
    enabled: bool,
) -> Result<Trigger, Error> {
    let mut tx = pool.begin().await?;
    let mut trigger = triggers::find(&mut tx, id)
        .await?
        .ok_or(Error::NotFound("trigger"))?;
    if trigger.enabled != enabled {
        trigger.enabled = enabled;
        if let (true, TriggerKind::Cron(text)) = (enabled, &trigger.kind) {
            trigger.next_fire_at = schedule::next(text, OffsetDateTime::now_utc());
        }
        triggers::save(&mut tx, &trigger).await?;
    }
    tx.commit().await?;
    get(pool, id).await?.ok_or(Error::NotFound("trigger"))
}

/// Deletes a trigger. Its jobs stay.
pub(crate) async fn delete(pool: &PgPool, id: TriggerId) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    if !triggers::delete(&mut tx, id).await? {
        return Err(Error::NotFound("trigger"));
    }
    tx.commit().await?;
    tracing::info!(event = "trigger.deleted", trigger = %id);
    Ok(())
}

/// Returns the project and the workspace of a pipeline, or `None` if no pipeline has this id.
pub(crate) async fn pipeline_place(
    pool: &PgPool,
    id: PipelineId,
) -> Result<Option<(ProjectId, WorkspaceId)>, Error> {
    crate::features::projects::pipelines::service::place(pool, id).await
}

/// Returns the secret of an enabled webhook trigger, opened with the id of the trigger, or
/// `None` for an unknown, stopped or cron trigger.
pub(crate) async fn webhook_secret(
    pool: &PgPool,
    sealer: Option<&Sealer>,
    id: TriggerId,
) -> Result<Option<Vec<u8>>, Error> {
    let mut tx = pool.begin().await?;
    let sealed = triggers::webhook_secret(&mut tx, id).await?;
    tx.commit().await?;
    let Some(sealed) = sealed else {
        return Ok(None);
    };
    let sealer = sealer
        .ok_or_else(|| Error::Internal("the server has no key for the secrets".to_owned()))?;
    let secret = sealer
        .open_with(&sealed, &id.to_bytes())
        .map_err(|reason| Error::Internal(format!("trigger {id}: {reason}")))?;
    Ok(Some(secret))
}

/// Records the key of a delivery. Returns `false` if the trigger already has this key.
pub(crate) async fn deliver(pool: &PgPool, id: TriggerId, key: &str) -> Result<bool, Error> {
    let mut tx = pool.begin().await?;
    let new = deliveries::insert(&mut tx, id, key).await?;
    tx.commit().await?;
    Ok(new)
}

/// Forgets the key of a delivery whose fire failed, so a retry of the sender fires.
pub(crate) async fn forget(pool: &PgPool, id: TriggerId, key: &str) -> Result<(), Error> {
    let mut tx = pool.begin().await?;
    deliveries::delete(&mut tx, id, key).await?;
    tx.commit().await?;
    Ok(())
}
