//! The history of the fires of the triggers: each fire, each cron time that the server missed,
//! their lists, and their counts for the dashboard.

use time::{Duration, OffsetDateTime};

use super::TriggerId;
use crate::projects::jobs::{FiguresPeriod, JobId, JobStatus};
use crate::projects::pipelines::{PipelineId, TriggerType};
use crate::projects::ProjectId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Id, PageRequest, Span, TimeZone};

/// The days that the history keeps a fire.
pub const FIRE_RETENTION_DAYS: i64 = 30;

/// The id of a fire.
pub type FireId = Id<TriggerFire>;

/// What started a fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireSource {
    /// The cron schedule of the trigger.
    Schedule,
    /// A signed request to the webhook route of the trigger.
    Webhook,
    /// A call of `FireTriggerNow`.
    Manual,
}

impl FireSource {
    /// Returns the name of the source, as the database keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Schedule => "schedule",
            Self::Webhook => "webhook",
            Self::Manual => "manual",
        }
    }

    /// Reads the name of a source.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Schedule, Self::Webhook, Self::Manual]
            .into_iter()
            .find(|source| source.as_str() == name)
    }
}

/// The result of a fire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireStatus {
    /// The fire made a job.
    Ok,
    /// The fire ran but made no job: "fire failed".
    Error,
    /// The server was down at the cron time, so the time did not fire: "time missed".
    Missed,
    /// The delivery of a webhook did not match its conditions, so the fire made no job.
    Skipped,
}

impl FireStatus {
    /// Returns the name of the result, as the database keeps it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Missed => "missed",
            Self::Skipped => "skipped",
        }
    }

    /// Reads the name of a result.
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Ok, Self::Error, Self::Missed, Self::Skipped]
            .into_iter()
            .find(|status| status.as_str() == name)
    }
}

/// One fire of a trigger, or one cron time that the server missed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerFire {
    /// The id.
    pub id: FireId,
    /// The trigger.
    pub trigger_id: TriggerId,
    /// The display name of the trigger.
    pub trigger_name: String,
    /// The pipeline of the trigger.
    pub pipeline_id: PipelineId,
    /// The project of the pipeline.
    pub project_id: ProjectId,
    /// What started the fire.
    pub source: FireSource,
    /// The time when the fire was due: the cron time of a schedule, else the time of the request.
    pub due_at: OffsetDateTime,
    /// The start of the fire, or `None` for a missed time.
    pub fired_at: Option<OffsetDateTime>,
    /// The result.
    pub status: FireStatus,
    /// The reason of a fire that failed, or of a delivery that the conditions skipped.
    pub error: Option<String>,
    /// The job of the fire, while it exists.
    pub job_id: Option<JobId>,
    /// The state of the job now.
    pub job_status: Option<JobStatus>,
}

/// The fires to read in a workspace: the projects that the caller can see, and the filters.
#[derive(Clone, Debug)]
pub struct WorkspaceFires {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The projects that the caller can see, or `None` for each project of the workspace.
    pub visible: Option<Vec<ProjectId>>,
    /// Only the fires of these projects. Empty gives each project.
    pub projects: Vec<ProjectId>,
    /// Only the fires of these pipelines. Empty gives each pipeline.
    pub pipelines: Vec<PipelineId>,
    /// Only the fires of these triggers. Empty gives each trigger.
    pub triggers: Vec<TriggerId>,
    /// Only the fires from these sources. Empty gives each source.
    pub sources: Vec<FireSource>,
}

/// The input of a list of fires, newest due time first.
#[derive(Clone, Debug)]
pub struct ListFires {
    /// The fires to read.
    pub fires: WorkspaceFires,
    /// Only the fires of the triggers of these kinds. Empty gives each kind.
    pub kinds: Vec<TriggerType>,
    /// Only the fires with these results. Empty gives each result.
    pub statuses: Vec<FireStatus>,
    /// Only the fires due at this time or later.
    pub from: Option<OffsetDateTime>,
    /// Only the fires due before this time.
    pub to: Option<OffsetDateTime>,
    /// The page to return.
    pub page: PageRequest<FireId>,
}

/// The counts of a set of fires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FireFigures {
    /// Each fire and each missed time.
    pub total: u32,
    /// The fires that made a job.
    pub ok: u32,
    /// The fires that ran but made no job.
    pub failed: u32,
    /// The cron times that the server missed.
    pub missed: u32,
    /// The deliveries of a webhook that did not match its conditions.
    pub skipped: u32,
    /// The median delay of the fires of a schedule, from the cron time to the start.
    pub p50_delay: Option<Duration>,
    /// The 95th percentile of the same delays.
    pub p95_delay: Option<Duration>,
}

impl FireFigures {
    /// Returns the failed fires and the missed times.
    pub fn problems(&self) -> u32 {
        self.failed.saturating_add(self.missed)
    }
}

/// What makes the groups of the fire counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireGroupBy {
    /// One group for each trigger.
    Trigger,
    /// One group for each pipeline.
    Pipeline,
    /// One group for each project.
    Project,
}

/// The key of a group of fires.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FireGroupKey {
    /// A trigger.
    Trigger(TriggerId),
    /// A pipeline.
    Pipeline(PipelineId),
    /// A project.
    Project(ProjectId),
}

impl FireGroupKey {
    /// Returns the bits of the id of the key, to order two keys of one kind.
    fn rank(&self) -> [u8; 16] {
        match self {
            Self::Trigger(id) => id.to_bytes(),
            Self::Pipeline(id) => id.to_bytes(),
            Self::Project(id) => id.to_bytes(),
        }
    }
}

/// The fires of one trigger, pipeline or project.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FireGroup {
    /// The key.
    pub key: FireGroupKey,
    /// The fires of the period.
    pub figures: FireFigures,
}

/// The input of the fire counts of a workspace.
#[derive(Clone, Debug)]
pub struct FireStatsQuery {
    /// The fires to count.
    pub fires: WorkspaceFires,
    /// The period of the due times.
    pub period: FiguresPeriod,
    /// The size of the buckets, or `None` for no bucket.
    pub bucket: Option<Span>,
    /// What makes the groups, or `None` for no group.
    pub group_by: Option<FireGroupBy>,
    /// The most groups.
    pub limit: u32,
}

/// The fire counts of a workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct FireStats {
    /// The fires of the period.
    pub totals: FireFigures,
    /// The start of each bucket of the period with its fires, oldest first.
    pub buckets: Vec<(OffsetDateTime, FireFigures)>,
    /// The first groups: the most problems first, then the most fires.
    pub groups: Vec<FireGroup>,
    /// The time zone of the buckets.
    pub time_zone: TimeZone,
}

impl FireStats {
    /// Returns a bucket for each start, with the counts of the bucket numbers of `found`: number
    /// 1 is the first start. A start with no fire gets counts at 0.
    pub fn buckets(
        starts: &[OffsetDateTime],
        found: &[(usize, FireFigures)],
    ) -> Vec<(OffsetDateTime, FireFigures)> {
        starts
            .iter()
            .enumerate()
            .map(|(place, start)| {
                let figures = found
                    .iter()
                    .find(|(number, _)| *number == place + 1)
                    .map(|(_, figures)| *figures)
                    .unwrap_or_default();
                (*start, figures)
            })
            .collect()
    }

    /// Returns the first groups, at most `limit`: the most problems first, then the most fires,
    /// then the key.
    pub fn top(mut groups: Vec<FireGroup>, limit: u32) -> Vec<FireGroup> {
        groups.sort_by(|a, b| {
            b.figures
                .problems()
                .cmp(&a.figures.problems())
                .then_with(|| b.figures.total.cmp(&a.figures.total))
                .then_with(|| a.key.rank().cmp(&b.key.rank()))
        });
        groups.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        groups
    }
}
