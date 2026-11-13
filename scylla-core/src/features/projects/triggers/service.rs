//! The logic of the triggers that people call: one transaction for each call.

use scylla_domain::projects::git_rules::GitRules;
use scylla_domain::projects::pipelines::PipelineId;
use scylla_domain::projects::triggers::{
    ListTriggers, NewTrigger, Trigger, TriggerChange, TriggerId, TriggerKind, TriggerPage,
};
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Page;
use scylla_extension::{Rejection, Tx};
use sqlx::PgPool;
use time::OffsetDateTime;

use super::schedule;
use super::store::{deliveries, files, triggers};
use crate::infra::secret::{self, Sealer};
use crate::Error;

/// Creates a trigger. A webhook trigger gets a new secret, encrypted with the id of the
/// trigger, and the secret in clear comes back one time. A Git trigger needs a linked repository
/// and an editor pipeline.
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
        TriggerKind::Push(_) | TriggerKind::PullRequest(_) | TriggerKind::Tag(_) => {
            (None, None, None)
        }
    };

    // Refuse a Git trigger with no linked repository or on a file pipeline, then store it.
    let mut tx = pool.begin().await?;
    if new.kind.git_rules().is_some() {
        check_git_place(&mut tx, new.pipeline_id).await?;
    }
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

/// Returns one page of the triggers of a pipeline, oldest first. The first page of a file
/// pipeline starts with the read only triggers of the `on` table of its file.
pub(crate) async fn list(pool: &PgPool, query: ListTriggers) -> Result<TriggerPage, Error> {
    // Read the page, and the triggers of the file for the first page.
    let mut tx = pool.begin().await?;
    let items = triggers::list(&mut tx, query.pipeline_id, query.page).await?;
    let from_file = match query.page.after {
        None => files::list(&mut tx, query.pipeline_id).await?,
        Some(_) => Vec::new(),
    };
    tx.commit().await?;

    // Put the triggers of the file first.
    let mut page = Page::from_overfetch(items, query.page.size, |trigger| trigger.id);
    page.items.splice(0..0, from_file);
    Ok(page)
}

/// Changes the name, the schedule, the rule or the inputs of a trigger. A schedule of another
/// kind than cron, or a rule of another kind than the kind of the trigger, gives
/// `INVALID_ARGUMENT`.
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
    if let Some(rules) = change.rules {
        if rules.name() != trigger.kind.name() {
            return Err(Error::invalid(
                "rules",
                "do not match the kind of the trigger",
            ));
        }
        trigger.kind = rules;
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

/// Returns the rules of the enabled Git triggers of a pipeline, for the `on` table of its file:
/// the oldest trigger of each kind.
pub(crate) async fn git_rules(pool: &PgPool, pipeline: PipelineId) -> Result<GitRules, Error> {
    let mut tx = pool.begin().await?;
    let kinds = triggers::git_kinds(&mut tx, pipeline).await?;
    tx.commit().await?;
    let mut rules = GitRules::default();
    for kind in kinds {
        match kind {
            TriggerKind::Push(rule) => rules.push = rules.push.or(Some(rule)),
            TriggerKind::PullRequest(rule) => {
                rules.pull_request = rules.pull_request.or(Some(rule));
            }
            TriggerKind::Tag(rule) => rules.tag = rules.tag.or(Some(rule)),
            TriggerKind::Cron(_) | TriggerKind::Webhook => {}
        }
    }
    Ok(rules)
}

/// Refuses a Git trigger of a project with no linked repository with `NO_REPOSITORY_LINK`, and
/// one of a file pipeline with `PIPELINE_READ_ONLY`: its `on` table gives its Git triggers.
async fn check_git_place(tx: &mut Tx<'_>, pipeline: PipelineId) -> Result<(), Error> {
    match triggers::git_place(tx, pipeline).await? {
        None => Err(Error::NotFound("pipeline")),
        Some((true, _)) => {
            let message = "the Git triggers of a file pipeline come from the on table of its file";
            Err(Error::Rejected(Rejection::new(
                "PIPELINE_READ_ONLY",
                message,
            )))
        }
        Some((false, false)) => {
            let message = "the project has no linked repository";
            Err(Error::Rejected(Rejection::new(
                "NO_REPOSITORY_LINK",
                message,
            )))
        }
        Some((false, true)) => Ok(()),
    }
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
