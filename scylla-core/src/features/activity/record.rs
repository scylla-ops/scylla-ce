//! What records a row of the feed, in the transaction of its change: a row of any kind, and the
//! rows that need a name that the change does not have.

use scylla_domain::activity::{Kind, NewActivity, Place, MAX_PARAMS};
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::apps::{self, App};
use scylla_domain::identity::users;
use scylla_domain::integrations::connections::VcsConnectionId;
use scylla_domain::projects::jobs::{self, Job};
use scylla_domain::projects::secrets::Secret;
use scylla_domain::projects::triggers::Trigger;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use super::store;
use crate::Error;

/// The kind of the object of a row about a trigger.
const TRIGGER: &str = "trigger";

/// The kind of the object of a row about a secret.
const SECRET: &str = "secret";

/// The kind of the object of a row about a Git connection.
const CONNECTION: &str = "vcs_connection";

/// The number of characters of a reason that a row keeps.
const REASON_CHARS: usize = 512;

/// Records a row in the transaction of its change. A rollback removes it. Refuses parameters
/// that are too large.
pub(crate) async fn record(tx: &mut Tx<'_>, new: NewActivity) -> Result<(), Error> {
    if new.params_size() > MAX_PARAMS {
        let reason = format!("the parameters of {} are too large", new.kind.as_str());
        return Err(Error::Internal(reason));
    }
    store::insert(tx, &new).await
}

/// Records the end of a job, with the name of its pipeline. A job that did not end records
/// nothing.
pub(crate) async fn job_ended(tx: &mut Tx<'_>, job: &Job) -> Result<(), Error> {
    if !job.status.is_final() {
        return Ok(());
    }
    let pipeline = store::pipeline_name(tx, job.pipeline_id).await?;
    let row = NewActivity::new(Kind::JobEnded, Place::Project(job.project_id))
        .actor(job.created_by)
        .subject(jobs::KIND.as_str(), job.id)
        .param("status", job.status.as_str())
        .param("pipeline", pipeline.unwrap_or_default())
        .param("origin", job.origin.as_str());
    record(tx, row).await
}

/// Records a change of a trigger by an actor, with its name, its kind, its schedule and the name
/// of its pipeline.
pub(crate) async fn trigger(
    tx: &mut Tx<'_>,
    kind: Kind,
    trigger: &Trigger,
    actor: ActorId,
) -> Result<(), Error> {
    let row = trigger_row(tx, kind, trigger).await?.actor(actor);
    record(tx, row).await
}

/// Records a fire of a trigger that made no job, with the start of the reason.
pub(crate) async fn fire_failed(
    tx: &mut Tx<'_>,
    trigger: &Trigger,
    reason: &str,
) -> Result<(), Error> {
    let reason: String = reason.chars().take(REASON_CHARS).collect();
    let row = trigger_row(tx, Kind::TriggerFireFailed, trigger)
        .await?
        .param("error", reason);
    record(tx, row).await
}

/// Records the times of the schedule of a trigger that the server missed, with their number and
/// the newest one in Unix seconds. No time records nothing.
pub(crate) async fn time_missed(
    tx: &mut Tx<'_>,
    trigger: &Trigger,
    times: &[OffsetDateTime],
) -> Result<(), Error> {
    let Some(last) = times.last() else {
        return Ok(());
    };
    let row = trigger_row(tx, Kind::TriggerTimeMissed, trigger)
        .await?
        .param("count", times.len().to_string())
        .param("last_due_at", last.unix_timestamp().to_string());
    record(tx, row).await
}

/// Makes the row of a trigger, with no actor.
async fn trigger_row(tx: &mut Tx<'_>, kind: Kind, trigger: &Trigger) -> Result<NewActivity, Error> {
    let pipeline = store::pipeline_name(tx, trigger.pipeline_id).await?;
    let mut row = NewActivity::new(kind, Place::Project(trigger.project_id))
        .subject(TRIGGER, trigger.id)
        .param("name", trigger.name.clone())
        .param("pipeline", pipeline.unwrap_or_default())
        .param("trigger_kind", trigger.kind.name());
    if let Some(schedule) = trigger.kind.schedule() {
        row = row.param("schedule", schedule);
    }
    Ok(row)
}

/// Records a change of a secret by an actor, with its name. The value never goes in the row.
pub(crate) async fn secret(
    tx: &mut Tx<'_>,
    kind: Kind,
    secret: &Secret,
    actor: ActorId,
) -> Result<(), Error> {
    let row = NewActivity::new(kind, Place::Project(secret.project_id))
        .actor(actor)
        .subject(SECRET, secret.id)
        .param("name", secret.name.as_str());
    record(tx, row).await
}

/// Records a role that a person or an App got or lost from an actor, with its name.
pub(crate) async fn role(
    tx: &mut Tx<'_>,
    kind: Kind,
    place: Place,
    (holder, role): (ActorId, &str),
    by: ActorId,
) -> Result<(), Error> {
    let (holder_kind, name) = store::actor(tx, holder)
        .await?
        .unwrap_or_else(|| (users::KIND.as_str().to_owned(), None));
    let row = NewActivity::new(kind, place)
        .actor(by)
        .subject(holder_kind, holder)
        .param("role", role)
        .param("name", name.unwrap_or_default());
    record(tx, row).await
}

/// Records a person that joined a workspace with an invitation, with the name of the person that
/// invited.
pub(crate) async fn joined(
    tx: &mut Tx<'_>,
    place: Place,
    (person, role): (ActorId, &str),
    inviter: ActorId,
) -> Result<(), Error> {
    let inviter = store::actor_name(tx, inviter).await?;
    let row = NewActivity::new(Kind::MemberJoined, place)
        .actor(person)
        .subject(users::KIND.as_str(), person)
        .param("role", role)
        .param("inviter", inviter.unwrap_or_default());
    record(tx, row).await
}

/// Makes the row of a change of a Git connection, with its provider and the login of its account.
/// The caller adds the actor and the other parameters, then records it.
pub(crate) fn connection(
    kind: Kind,
    (id, workspace): (VcsConnectionId, WorkspaceId),
    (provider, account): (&str, &str),
) -> NewActivity {
    NewActivity::new(kind, Place::Workspace(workspace))
        .subject(CONNECTION, id)
        .param("provider", provider)
        .param("account", account)
}

/// Records the stream of an agent that opened or ended, with its name, its slots and the name of
/// the person that added it.
pub(crate) async fn agent(tx: &mut Tx<'_>, kind: Kind, app: &App) -> Result<(), Error> {
    let mut row = NewActivity::new(kind, Place::Workspace(app.workspace_id))
        .actor(App::actor(app.id))
        .subject(apps::KIND.as_str(), app.id)
        .param("name", app.name.clone());
    if kind == Kind::AgentConnected {
        let added_by = store::actor_name(tx, app.created_by).await?;
        let slots = app.host.as_ref().map_or(1, |host| host.capacity.max(1));
        row = row
            .param("slots", slots.to_string())
            .param("added_by", added_by.unwrap_or_default());
    }
    record(tx, row).await
}
