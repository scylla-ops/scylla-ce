//! A fact for the notifications, the people that get a row, and the notifier that records it.

use std::collections::BTreeMap;

use async_trait::async_trait;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::projects::ProjectId;
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_domain::Id;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{Gate, Tx};

/// The parameters of a fact: short texts, never a sentence, a secret or a link with a token.
pub type Params = BTreeMap<String, String>;

/// The reason of a recipient that the fact is about. Such a row stays after its access ends.
pub const REASON_SUBJECT: &str = "subject";

/// An error of the notifications.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct NotificationError(pub String);

/// A fact to record, in the transaction of the write that makes it true.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Event {
    /// The key of the kind.
    pub kind: String,
    /// The actor that made the fact.
    pub actor: Option<ActorId>,
    /// The workspace of the fact.
    pub workspace: Option<WorkspaceId>,
    /// The project of the fact.
    pub project: Option<ProjectId>,
    /// The kind and the id of the object of the fact.
    pub subject: Option<(String, Uuid)>,
    /// The parameters.
    pub params: Params,
    /// A second fact with this key is not recorded.
    pub dedupe: Option<String>,
    /// The rows of one recipient with this key are one row. `None` makes a row for each fact.
    pub group: Option<String>,
}

impl Event {
    /// Makes a fact of a kind.
    pub fn new(kind: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            actor: None,
            workspace: None,
            project: None,
            subject: None,
            params: Params::new(),
            dedupe: None,
            group: None,
        }
    }

    /// Sets the actor.
    pub fn actor(mut self, actor: ActorId) -> Self {
        self.actor = Some(actor);
        self
    }

    /// Sets the workspace.
    pub fn workspace(mut self, workspace: WorkspaceId) -> Self {
        self.workspace = Some(workspace);
        self
    }

    /// Sets the project.
    pub fn project(mut self, project: ProjectId) -> Self {
        self.project = Some(project);
        self
    }

    /// Sets the object of the fact.
    pub fn subject<T: ?Sized>(mut self, kind: impl Into<String>, id: Id<T>) -> Self {
        self.subject = Some((kind.into(), id.to_db()));
        self
    }

    /// Adds one parameter.
    pub fn param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    /// Sets the key that stops a second record of the fact.
    pub fn dedupe(mut self, key: impl Into<String>) -> Self {
        self.dedupe = Some(key.into());
        self
    }

    /// Sets the key of the group of rows.
    pub fn group(mut self, key: impl Into<String>) -> Self {
        self.group = Some(key.into());
        self
    }
}

/// A recorded fact, as an audience reads it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct EventRow {
    /// The id of the record.
    pub id: Uuid,
    /// The fact.
    pub event: Event,
}

impl EventRow {
    /// Makes a record of a fact.
    pub fn new(id: Uuid, event: Event) -> Self {
        Self { id, event }
    }

    /// Returns the object of the fact as the id of a type, if the fact has an object of this kind.
    pub fn subject_id<T: ?Sized>(&self, kind: &str) -> Option<Id<T>> {
        let (subject_kind, id) = self.event.subject.as_ref()?;
        (subject_kind == kind).then(|| Id::from_db(*id))
    }
}

/// A person that gets a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recipient {
    /// The person. An actor that is not an active user gets no row.
    pub user: ActorId,
    /// Why the person gets the row: a closed key for the web app.
    pub reason: &'static str,
}

impl Recipient {
    /// Makes a recipient for a reason.
    pub fn new(user: ActorId, reason: &'static str) -> Self {
        Self { user, reason }
    }

    /// Makes the recipient that the fact is about.
    pub fn about(user: ActorId) -> Self {
        Self::new(user, REASON_SUBJECT)
    }
}

/// What an audience reads.
#[non_exhaustive]
pub struct AudienceContext<'a> {
    /// The database pool.
    pub pool: &'a PgPool,
    /// The access gate.
    pub gate: &'a dyn Gate,
}

impl<'a> AudienceContext<'a> {
    /// Makes the context.
    pub fn new(pool: &'a PgPool, gate: &'a dyn Gate) -> Self {
        Self { pool, gate }
    }
}

/// Finds the people that get a row. It runs with no open transaction, after the commit of the fact.
#[async_trait]
pub trait Audience: Send + Sync + 'static {
    /// Returns the recipients of a fact. An error delays the fact and tries again.
    async fn recipients(
        &self,
        ctx: &AudienceContext<'_>,
        event: &EventRow,
    ) -> Result<Vec<Recipient>, NotificationError>;
}

/// Records a fact. The core gives it to the services of the plugins.
#[async_trait]
pub trait Notifier: Send + Sync + 'static {
    /// Records a fact in the transaction of a write, and returns `false` if its dedupe key has a
    /// record. A rollback removes the fact.
    async fn emit(&self, tx: &mut Tx<'_>, event: Event) -> Result<bool, NotificationError>;

    /// Records a fact in its own transaction: a refusal rolls back the transaction of its hook, so
    /// a fact about a refusal goes here. Use it with a dedupe key.
    async fn emit_detached(&self, event: Event) -> Result<bool, NotificationError>;
}
