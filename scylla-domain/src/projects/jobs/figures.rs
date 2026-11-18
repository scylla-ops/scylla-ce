//! The figures of the jobs of a workspace for the dashboard: the totals of a period, its buckets
//! of hours, days or weeks in the time zone of the workspace, its groups, and the totals of the
//! previous period.

#[cfg(test)]
mod tests;

use std::cmp::Ordering;
use std::collections::HashMap;

use time::{Duration, OffsetDateTime};

use super::{JobOrigin, Outcomes, WorkspaceJobs};
use crate::identity::apps::AppId;
use crate::projects::pipelines::PipelineId;
use crate::projects::ProjectId;
use crate::{DomainError, Span, TimeZone};

/// The longest period of the figures.
pub const MAX_FIGURES_DAYS: i64 = 400;

/// The largest number of buckets of the figures.
pub const MAX_BUCKETS: usize = 1000;

/// The number of groups when the client gives 0.
pub const DEFAULT_GROUPS: u32 = 20;

/// The largest number of groups.
pub const MAX_GROUPS: u32 = 100;

/// The figures of a set of jobs.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Figures {
    /// Each job of the set, also the jobs that did not end.
    pub created: u32,
    /// The jobs that ended, by their end.
    pub outcomes: Outcomes,
    /// The median time from the start to the end of the jobs that started and ended.
    pub p50_duration: Option<Duration>,
    /// The 95th percentile of the same times.
    pub p95_duration: Option<Duration>,
    /// The median wait for an agent of the jobs that started: from the creation, or from the
    /// approval of a job from a fork, to the start.
    pub p50_queue: Option<Duration>,
    /// The 95th percentile of the same waits.
    pub p95_queue: Option<Duration>,
    /// The sum of the times from the start to the end of the jobs that started and ended.
    pub run_time: Duration,
}

/// The period of the figures: the jobs created at `from` or later, and before `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FiguresPeriod {
    /// The start.
    pub from: OffsetDateTime,
    /// The end.
    pub to: OffsetDateTime,
}

impl FiguresPeriod {
    /// Returns the period of a request: `to` is `now` when it is not set. Refuses an end that is
    /// not after the start, and a period of more than `MAX_FIGURES_DAYS` days.
    pub fn new(
        from: OffsetDateTime,
        to: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> Result<Self, DomainError> {
        let to = to.unwrap_or(now);
        if to <= from {
            return Err(DomainError::NotAfterStart);
        }
        if to - from > Duration::days(MAX_FIGURES_DAYS) {
            return Err(DomainError::PeriodTooLong {
                days: MAX_FIGURES_DAYS,
            });
        }
        Ok(Self { from, to })
    }

    /// Returns the period of the same length that ends at the start of this period.
    pub fn previous(&self) -> Self {
        Self {
            from: self.from - (self.to - self.from),
            to: self.from,
        }
    }

    /// Returns the start of each hour, day or week of the period in a zone, oldest first. The
    /// first start can be before `from`. Refuses more than `MAX_BUCKETS` buckets.
    pub fn starts(&self, zone: TimeZone, span: Span) -> Result<Vec<OffsetDateTime>, DomainError> {
        let mut starts = vec![zone.start_of(self.from, span)];
        loop {
            let next = starts
                .last()
                .map_or(self.to, |last| zone.next_start(*last, span));
            if next >= self.to {
                return Ok(starts);
            }
            if starts.len() == MAX_BUCKETS {
                return Err(DomainError::TooMany { max: MAX_BUCKETS });
            }
            starts.push(next);
        }
    }
}

/// What makes the groups of the figures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupBy {
    /// One group for each project.
    Project,
    /// One group for each pipeline.
    Pipeline,
    /// One group for each agent. A job with no agent is in no group.
    Agent,
    /// One group for each origin.
    Origin,
}

/// The order of the groups: the largest value first, then the key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GroupOrder {
    /// The most jobs.
    #[default]
    Count,
    /// The most jobs that failed or were orphaned.
    Failed,
    /// The longest median duration. No median comes last.
    P50Duration,
    /// The longest run time.
    RunTime,
}

/// The key of a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GroupKey {
    /// A project.
    Project(ProjectId),
    /// A pipeline.
    Pipeline(PipelineId),
    /// An agent.
    Agent(AppId),
    /// An origin.
    Origin(JobOrigin),
}

impl GroupKey {
    /// Returns a value that orders the keys of one kind: the bits of an id, or the place of an
    /// origin in `JobOrigin::ALL`.
    fn rank(&self) -> [u8; 16] {
        match self {
            Self::Project(id) => id.to_bytes(),
            Self::Pipeline(id) => id.to_bytes(),
            Self::Agent(id) => id.to_bytes(),
            Self::Origin(origin) => {
                let place = JobOrigin::ALL.iter().position(|one| one == origin);
                (place.unwrap_or(usize::MAX) as u128).to_be_bytes()
            }
        }
    }
}

/// The input of the figures of the jobs of a workspace.
#[derive(Clone, Debug)]
pub struct FiguresQuery {
    /// The jobs: the workspace, the projects that the caller can see, and the filters.
    pub jobs: WorkspaceJobs,
    /// Only the jobs with one of these origins. Empty gives each origin.
    pub origins: Vec<JobOrigin>,
    /// The period.
    pub period: FiguresPeriod,
    /// The size of the buckets, or `None` for no bucket.
    pub bucket: Option<Span>,
    /// What makes the groups, or `None` for no group.
    pub group_by: Option<GroupBy>,
    /// The order of the groups.
    pub order: GroupOrder,
    /// The most groups.
    pub limit: u32,
    /// `true` to also count the jobs of the previous period.
    pub compare: bool,
}

/// The jobs of one hour, day or week.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bucket {
    /// The start in the time zone of the workspace.
    pub start: OffsetDateTime,
    /// The jobs created in the bucket.
    pub figures: Figures,
}

/// The jobs of one project, pipeline, agent or origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    /// The key.
    pub key: GroupKey,
    /// The jobs of the period.
    pub figures: Figures,
    /// The jobs of the previous period, when the query compares.
    pub previous: Option<Figures>,
}

/// The figures of the jobs of a workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct JobFigures {
    /// The jobs of the period.
    pub totals: Figures,
    /// Each bucket of the period, oldest first.
    pub buckets: Vec<Bucket>,
    /// The first groups in the order of the query.
    pub groups: Vec<Group>,
    /// The jobs of the previous period, when the query compares.
    pub previous: Option<Figures>,
    /// The time zone of the buckets.
    pub time_zone: TimeZone,
}

/// Returns a bucket for each start, with the figures of the bucket numbers of `found`: number 1
/// is the first start. A start with no figures gets figures at 0.
pub fn buckets(starts: &[OffsetDateTime], found: &[(usize, Figures)]) -> Vec<Bucket> {
    starts
        .iter()
        .enumerate()
        .map(|(place, start)| Bucket {
            start: *start,
            figures: found
                .iter()
                .find(|(number, _)| *number == place + 1)
                .map(|(_, figures)| *figures)
                .unwrap_or_default(),
        })
        .collect()
}

/// Returns the first groups in an order, at most `limit`, each with its figures of the previous
/// period when `previous` is set. A group with no job in the previous period gets figures at 0.
pub fn top(
    mut groups: Vec<(GroupKey, Figures)>,
    previous: Option<Vec<(GroupKey, Figures)>>,
    order: GroupOrder,
    limit: u32,
) -> Vec<Group> {
    // Order the groups, and keep the first ones.
    groups.sort_by(|a, b| compare(order, &a.1, &b.1).then_with(|| a.0.rank().cmp(&b.0.rank())));
    groups.truncate(usize::try_from(limit).unwrap_or(usize::MAX));

    // Add the figures of the previous period to each group.
    let before: Option<HashMap<GroupKey, Figures>> =
        previous.map(|groups| groups.into_iter().collect());
    groups
        .into_iter()
        .map(|(key, figures)| Group {
            key,
            figures,
            previous: before
                .as_ref()
                .map(|before| before.get(&key).copied().unwrap_or_default()),
        })
        .collect()
}

/// Compares two figures in an order: the larger value comes first.
fn compare(order: GroupOrder, a: &Figures, b: &Figures) -> Ordering {
    let failed = |figures: &Figures| {
        figures
            .outcomes
            .failed
            .saturating_add(figures.outcomes.orphaned)
    };
    match order {
        GroupOrder::Count => b.created.cmp(&a.created),
        GroupOrder::Failed => failed(b).cmp(&failed(a)),
        GroupOrder::P50Duration => match (a.p50_duration, b.p50_duration) {
            (Some(a), Some(b)) => b.cmp(&a),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        },
        GroupOrder::RunTime => b.run_time.cmp(&a.run_time),
    }
}
