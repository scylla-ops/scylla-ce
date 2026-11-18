//! The activity feed of a workspace: one row for each change of its jobs, triggers, secrets,
//! members, Git connections and agents. A row shows only to the readers of its area.

use std::collections::BTreeMap;

use time::OffsetDateTime;

use crate::authz::Permission;
use crate::identity::actors::{Actor, ActorId};
use crate::identity::apps::AppHolder;
use crate::integrations::connections::VcsConnectionHolder;
use crate::integrations::named;
use crate::projects::pipelines::{PipelineAccess, PipelineHolder};
use crate::projects::triggers::{TriggerAccess, TriggerHolder};
use crate::projects::{Project, ProjectHolder, ProjectId};
use crate::tenancy::grants::WorkspaceMembers;
use crate::tenancy::workspaces::{Workspace, WorkspaceId};
use crate::{Id, Page, PageRequest};

/// The id of a row.
pub type ActivityId = Id<Activity>;

/// The id of the object of a row.
pub type SubjectId = Id<Subject>;

/// The time that a row stays.
pub const RETENTION: time::Duration = time::Duration::days(30);

/// The number at which the count of the rows since a time stops.
pub const SINCE_CAP: i32 = 100;

/// The largest size of the parameters of a row, in bytes.
pub const MAX_PARAMS: usize = 2048;

named! {
    /// The area of a row: a reader needs the read permission of the area to see the row.
    pub enum Area {
        /// The jobs of a project.
        Jobs = "jobs",
        /// The triggers of a project.
        Triggers = "triggers",
        /// The secrets of a project.
        Secrets = "secrets",
        /// The members of the workspace.
        Members = "members",
        /// The Git connections of the workspace.
        Integrations = "integrations",
        /// The agents of the workspace.
        Agents = "agents",
    }
}

impl Area {
    /// Returns the read permission of the area on the workspace: with it, a reader sees each row
    /// of the area in the workspace.
    pub fn holder(self) -> Permission<Workspace> {
        match self {
            Self::Jobs => PipelineHolder::LIST_WORKSPACE_JOBS,
            Self::Triggers => TriggerHolder::LIST_PIPELINE_TRIGGERS,
            Self::Secrets => ProjectHolder::LIST_SECRETS,
            Self::Members => WorkspaceMembers::LIST_MEMBERS,
            Self::Integrations => VcsConnectionHolder::LIST_VCS_CONNECTIONS,
            Self::Agents => AppHolder::LIST_AGENT_STATS,
        }
    }

    /// Returns the read permission of the area on a project, or `None` for an area of the
    /// workspace.
    pub fn access(self) -> Option<Permission<Project>> {
        match self {
            Self::Jobs => Some(PipelineAccess::LIST_WORKSPACE_JOBS),
            Self::Triggers => Some(TriggerAccess::LIST_PIPELINE_TRIGGERS),
            Self::Secrets => Some(Project::LIST_SECRETS),
            Self::Members | Self::Integrations | Self::Agents => None,
        }
    }
}

named! {
    /// The kind of a row, as `<subject>.<action>`.
    pub enum Kind {
        /// A job ended.
        JobEnded = "job.ended",
        /// A person made a trigger.
        TriggerCreated = "trigger.created",
        /// A person changed a trigger.
        TriggerUpdated = "trigger.updated",
        /// A person deleted a trigger.
        TriggerDeleted = "trigger.deleted",
        /// A person started a trigger.
        TriggerEnabled = "trigger.enabled",
        /// A person stopped a trigger.
        TriggerDisabled = "trigger.disabled",
        /// A fire of a trigger made no job.
        TriggerFireFailed = "trigger.fire_failed",
        /// The server missed times of the schedule of a trigger.
        TriggerTimeMissed = "trigger.time_missed",
        /// A person made a secret.
        SecretCreated = "secret.created",
        /// A person gave a secret a new value.
        SecretUpdated = "secret.updated",
        /// A person deleted a secret.
        SecretDeleted = "secret.deleted",
        /// A person accepted an invitation to the workspace.
        MemberJoined = "member.joined",
        /// A person got a role in the workspace or on one of its projects.
        MemberRoleGranted = "member.role_granted",
        /// A person lost a role in the workspace or on one of its projects.
        MemberRoleRevoked = "member.role_revoked",
        /// A person connected the workspace to a Git provider.
        ConnectionConnected = "connection.connected",
        /// A person deleted a Git connection.
        ConnectionDisconnected = "connection.disconnected",
        /// A Git connection stopped and needs a person.
        ConnectionBroken = "connection.broken",
        /// The repositories that a Git connection sees changed.
        ConnectionRepositoriesChanged = "connection.repositories_changed",
        /// An agent opened its stream.
        AgentConnected = "agent.connected",
        /// The stream of an agent ended.
        AgentDisconnected = "agent.disconnected",
    }
}

impl Kind {
    /// Returns the area of the kind.
    pub fn area(self) -> Area {
        match self {
            Self::JobEnded => Area::Jobs,
            Self::TriggerCreated
            | Self::TriggerUpdated
            | Self::TriggerDeleted
            | Self::TriggerEnabled
            | Self::TriggerDisabled
            | Self::TriggerFireFailed
            | Self::TriggerTimeMissed => Area::Triggers,
            Self::SecretCreated | Self::SecretUpdated | Self::SecretDeleted => Area::Secrets,
            Self::MemberJoined | Self::MemberRoleGranted | Self::MemberRoleRevoked => Area::Members,
            Self::ConnectionConnected
            | Self::ConnectionDisconnected
            | Self::ConnectionBroken
            | Self::ConnectionRepositoriesChanged => Area::Integrations,
            Self::AgentConnected | Self::AgentDisconnected => Area::Agents,
        }
    }
}

/// The object of a row: its kind and its id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    /// The kind of the object, for example `job`.
    pub kind: String,
    /// The id of the object.
    pub id: SubjectId,
}

/// Where a change is: a workspace, or a project of a workspace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The workspace itself.
    Workspace(WorkspaceId),
    /// A project, in its workspace.
    Project(ProjectId),
}

/// A row to record, in the transaction of its change.
#[derive(Clone, Debug)]
pub struct NewActivity {
    /// The kind.
    pub kind: Kind,
    /// Where the change is.
    pub place: Place,
    /// The actor of the change.
    pub actor: Option<ActorId>,
    /// The object of the change.
    pub subject: Option<Subject>,
    /// The parameters of the kind: short texts, never a secret.
    pub params: BTreeMap<String, String>,
}

impl NewActivity {
    /// Makes a row of a kind with no actor, no object and no parameter.
    pub fn new(kind: Kind, place: Place) -> Self {
        Self {
            kind,
            place,
            actor: None,
            subject: None,
            params: BTreeMap::new(),
        }
    }

    /// Sets the actor of the change.
    pub fn actor(mut self, actor: ActorId) -> Self {
        self.actor = Some(actor);
        self
    }

    /// Sets the object of the change.
    pub fn subject<T: ?Sized>(mut self, kind: impl Into<String>, id: Id<T>) -> Self {
        self.subject = Some(Subject {
            kind: kind.into(),
            id: id.retype(),
        });
        self
    }

    /// Adds a parameter.
    pub fn param(mut self, key: &str, value: impl Into<String>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }

    /// Returns the size of the parameters, in bytes.
    pub fn params_size(&self) -> usize {
        self.params
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum()
    }
}

/// One row of the feed, with what the read joins.
#[derive(Clone, Debug)]
pub struct Activity {
    /// The id.
    pub id: ActivityId,
    /// The key of the kind. A kind that this version does not know stays a text.
    pub kind: String,
    /// The name of the area.
    pub area: String,
    /// The actor of the change.
    pub actor_id: Option<ActorId>,
    /// The display name of the actor now.
    pub actor_name: Option<String>,
    /// The project of the change.
    pub project_id: Option<ProjectId>,
    /// The name of the project now.
    pub project_name: Option<String>,
    /// The object of the change.
    pub subject: Option<Subject>,
    /// The parameters of the kind.
    pub params: BTreeMap<String, String>,
    /// The time of the change.
    pub at: OffsetDateTime,
}

/// What a reader sees of a workspace: the areas of the whole workspace, and the areas of single
/// projects.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Visible {
    /// The areas in which the reader sees each row of the workspace.
    pub areas: Vec<Area>,
    /// The projects and their areas in which the reader sees the rows.
    pub projects: Vec<(ProjectId, Area)>,
}

/// The input of a list of the feed.
#[derive(Clone, Debug)]
pub struct ListActivity {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// What the reader sees.
    pub visible: Visible,
    /// The page.
    pub page: PageRequest<ActivityId>,
    /// Counts the rows at this time or later, if set.
    pub since: Option<OffsetDateTime>,
}

/// One page of the feed, newest first, and the count of the rows since a time.
#[derive(Clone, Debug)]
pub struct ActivityPage {
    /// The rows and the key of the next page.
    pub page: Page<Activity, ActivityId>,
    /// The number of rows since the time of the request, up to `SINCE_CAP`.
    pub since_count: i32,
}

crate::authz! {
    /// The route of the activity feed on the caller itself: the list shows only the rows that the
    /// caller can read.
    extend Actor as ActivityCaller {
        /// The route `ListActivity`: each caller lists the rows that it can read.
        permission list_activity = nil;
    }
}

#[cfg(test)]
mod tests;
