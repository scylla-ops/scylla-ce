//! The open items that need a person on the dashboard ("Needs you"): their kinds, the projects
//! where a caller reads each kind, the query of a list and the key of its pages.

mod items;
mod list;
#[cfg(test)]
mod tests;

pub use items::{
    AttentionItem, AttentionItemId, ItemAgent, ItemFire, ItemJob, ItemNode, Place, Subject,
};
pub use list::{AttentionList, KindCount};
use time::{Duration, OffsetDateTime};

use crate::projects::triggers::FIRE_RETENTION_DAYS;
use crate::projects::ProjectId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Id, PageRequest};

/// The days in which a job or a fire stays an open item: the days of the history of the fires.
pub const OPEN_DAYS: i64 = FIRE_RETENTION_DAYS;

/// The time after which an agent that is not connected is an open item.
pub const OFFLINE_AFTER: Duration = Duration::minutes(5);

/// The kind of an open item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttentionKind {
    /// A job that failed.
    JobFailed,
    /// A job whose agent went away.
    JobOrphaned,
    /// A fire of a trigger that ran and made no job.
    FireFailed,
    /// A cron time that the server missed.
    FireMissed,
    /// An enabled agent that is not connected.
    AgentOffline,
    /// A job from a fork that waits for an approval.
    ApprovalWaiting,
}

impl AttentionKind {
    /// Each kind, in the order of the counts.
    pub const ALL: [Self; 6] = [
        Self::JobFailed,
        Self::JobOrphaned,
        Self::FireFailed,
        Self::FireMissed,
        Self::AgentOffline,
        Self::ApprovalWaiting,
    ];
}

/// The projects of a workspace in which a caller reads the items of one kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Reach {
    /// No item of the kind.
    #[default]
    Nothing,
    /// The items of each project of the workspace.
    Workspace,
    /// The items of these projects only.
    Projects(Vec<ProjectId>),
}

impl Reach {
    /// Makes the reach of a list of visible projects: `None` for each project, and no project
    /// for nothing.
    pub fn of(visible: Option<Vec<ProjectId>>) -> Self {
        match visible {
            None => Self::Workspace,
            Some(projects) if projects.is_empty() => Self::Nothing,
            Some(projects) => Self::Projects(projects),
        }
    }

    /// Returns `true` if the caller reads at least one item of the kind.
    pub fn reads(&self) -> bool {
        *self != Self::Nothing
    }

    /// Returns `true` if the caller reads the items of each project.
    pub fn is_workspace(&self) -> bool {
        *self == Self::Workspace
    }

    /// Returns the projects of a reach to some projects. Empty for another reach.
    pub fn projects(&self) -> &[ProjectId] {
        match self {
            Self::Projects(projects) => projects,
            Self::Nothing | Self::Workspace => &[],
        }
    }
}

/// What a caller reads in a workspace, for each source of items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttentionReach {
    /// The jobs that failed or that lost their agent.
    pub jobs: Reach,
    /// The fires of the triggers.
    pub fires: Reach,
    /// The jobs that wait for an approval.
    pub approvals: Reach,
    /// The agents: `Workspace` or `Nothing`, as an agent is in no project.
    pub agents: Reach,
}

/// The input of a list of the open items of a workspace.
#[derive(Clone, Debug)]
pub struct AttentionQuery {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// Only the items of these projects. Empty: each project, and the agents.
    pub projects: Vec<ProjectId>,
    /// Only the items of these kinds. Empty: each kind.
    pub kinds: Vec<AttentionKind>,
    /// The last visit of the caller: the items after it are new.
    pub since: Option<OffsetDateTime>,
    /// The page to return.
    pub page: PageRequest<AttentionCursor>,
    /// What the caller reads. The handler sets it.
    pub reach: AttentionReach,
}

impl AttentionQuery {
    /// Returns the reach of the caller for one kind.
    pub fn reach_of(&self, kind: AttentionKind) -> &Reach {
        match kind {
            AttentionKind::JobFailed | AttentionKind::JobOrphaned => &self.reach.jobs,
            AttentionKind::FireFailed | AttentionKind::FireMissed => &self.reach.fires,
            AttentionKind::AgentOffline => &self.reach.agents,
            AttentionKind::ApprovalWaiting => &self.reach.approvals,
        }
    }

    /// Returns `true` if the query asks for a kind and the caller reads it. A filter of projects
    /// asks for no agent.
    pub fn asks(&self, kind: AttentionKind) -> bool {
        let wanted = self.kinds.is_empty() || self.kinds.contains(&kind);
        let place = kind != AttentionKind::AgentOffline || self.projects.is_empty();
        wanted && place && self.reach_of(kind).reads()
    }

    /// Returns the kinds that the query asks for and that the caller reads, in the order of the
    /// counts.
    pub fn open_kinds(&self) -> Vec<AttentionKind> {
        AttentionKind::ALL
            .into_iter()
            .filter(|kind| self.asks(*kind))
            .collect()
    }
}

/// The key of a page of open items: the time and the id of the last item of the previous page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttentionCursor {
    /// The time of the item.
    pub at: OffsetDateTime,
    /// The id of the item.
    pub id: AttentionItemId,
}

impl AttentionCursor {
    /// Reads the text of a key, or `None` if the text is not a key.
    pub fn parse(text: &str) -> Option<Self> {
        let (micros, id) = text.split_once('.')?;
        let nanos = micros.parse::<i128>().ok()?.checked_mul(1_000)?;
        Some(Self {
            at: OffsetDateTime::from_unix_timestamp_nanos(nanos).ok()?,
            id: Id::parse(id).ok()?,
        })
    }

    /// Returns the text of the key.
    pub fn text(&self) -> String {
        let micros = self.at.unix_timestamp_nanos() / 1_000;
        format!("{micros}.{}", self.id)
    }
}
