//! The summaries of the pipelines for the dashboard: the recent jobs, the health and the triggers
//! of each pipeline, and the order of a list of summaries.

#[cfg(test)]
mod tests;

use std::cmp::Ordering;

use time::{Duration, OffsetDateTime};

use super::PipelineId;
use crate::projects::jobs::{JobId, JobStatus};
use crate::projects::triggers::TriggerId;
use crate::projects::ProjectId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::PageRequest;

/// The number of recent jobs of a summary when the client gives 0.
pub const DEFAULT_RUNS: u32 = 20;

/// The largest number of recent jobs of a summary.
pub const MAX_RUNS: u32 = 50;

/// The largest number of ids in a filter of a list of summaries.
pub const MAX_FILTER_IDS: usize = 100;

/// The order of a list of summaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SummaryOrder {
    /// The red pipelines, then the pipelines with no ended job, then the green pipelines. In
    /// each group, the lowest success rate first, then the name.
    #[default]
    FailingFirst,
    /// The longest median duration first, then the name. No median comes last.
    Slowest,
    /// The name, with no difference between upper and lower case.
    Name,
}

/// The kind of a trigger, with no rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TriggerType {
    /// A schedule.
    Cron,
    /// A webhook.
    Webhook,
    /// A push to a branch of the linked repository.
    Push,
    /// A pull request of the linked repository.
    PullRequest,
    /// A push of a tag of the linked repository.
    Tag,
}

impl TriggerType {
    /// Reads the name of a kind, as the database keeps it.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "cron" => Some(Self::Cron),
            "webhook" => Some(Self::Webhook),
            "push" => Some(Self::Push),
            "pull_request" => Some(Self::PullRequest),
            "tag" => Some(Self::Tag),
            _ => None,
        }
    }
}

/// The input of a list of summaries.
#[derive(Clone, Debug)]
pub struct ListSummaries {
    /// The workspace.
    pub workspace_id: WorkspaceId,
    /// Only the pipelines of these projects. Empty: each project.
    pub project_ids: Vec<ProjectId>,
    /// Only these pipelines. Empty: each pipeline.
    pub pipeline_ids: Vec<PipelineId>,
    /// The number of recent jobs of each pipeline.
    pub runs: u32,
    /// The order of the pipelines.
    pub order: SummaryOrder,
    /// The page to return.
    pub page: PageRequest<PipelineId>,
}

/// One recent job of a pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentJob {
    /// The job.
    pub id: JobId,
    /// The state of the job.
    pub status: JobStatus,
    /// The branch of the commit of the job, if it has one.
    pub branch: Option<String>,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The start on the agent.
    pub started_at: Option<OffsetDateTime>,
    /// The end.
    pub finished_at: Option<OffsetDateTime>,
}

/// The next fire of a cron trigger of a pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextFire {
    /// The trigger.
    pub trigger_id: TriggerId,
    /// The schedule.
    pub schedule: String,
    /// The time of the fire.
    pub at: OffsetDateTime,
}

/// A pipeline with its recent jobs, its health and its triggers.
#[derive(Clone, Debug)]
pub struct PipelineSummary {
    /// The pipeline.
    pub pipeline_id: PipelineId,
    /// The display name of the pipeline.
    pub name: String,
    /// The project of the pipeline.
    pub project_id: ProjectId,
    /// The display name of the project.
    pub project_name: String,
    /// The recent jobs, newest first.
    pub recent: Vec<RecentJob>,
    /// The recent jobs that completed.
    pub completed: u32,
    /// The recent jobs that failed or were orphaned.
    pub failed: u32,
    /// The median duration of the recent jobs that completed.
    pub median: Option<Duration>,
    /// The state of the last ended job of the default branch, or of the last ended job with no
    /// linked repository.
    pub last_ended: Option<JobStatus>,
    /// `true` when a job of the pipeline runs or waits.
    pub active: bool,
    /// The kinds of the triggers that can start a job, in the order of their kinds.
    pub trigger_kinds: Vec<TriggerType>,
    /// The next fire of the enabled cron triggers.
    pub next_fire: Option<NextFire>,
}

impl PipelineSummary {
    /// Returns `true` when the last ended job completed.
    pub fn is_green(&self) -> bool {
        self.last_ended == Some(JobStatus::Completed)
    }

    /// Returns the group of the order with the failing pipelines first: red, no ended job,
    /// green.
    fn health_group(&self) -> u8 {
        match self.last_ended {
            Some(JobStatus::Completed) => 2,
            Some(_) => 0,
            None => 1,
        }
    }

    /// Compares the success rates of two summaries. A summary with no counted job has the rate
    /// of a summary with no failure.
    fn compare_rates(&self, other: &Self) -> Ordering {
        let rate = |summary: &Self| {
            let counted = u64::from(summary.completed) + u64::from(summary.failed);
            match counted {
                0 => (1, 1),
                counted => (u64::from(summary.completed), counted),
            }
        };
        let ((left, left_counted), (right, right_counted)) = (rate(self), rate(other));
        (left * right_counted).cmp(&(right * left_counted))
    }

    /// Compares the names of two summaries with no difference between upper and lower case.
    fn compare_names(&self, other: &Self) -> Ordering {
        let lower = |name: &str| {
            name.chars()
                .flat_map(char::to_lowercase)
                .collect::<Vec<_>>()
        };
        lower(&self.name).cmp(&lower(&other.name))
    }
}

impl SummaryOrder {
    /// Compares two summaries in this order. Two summaries with the same keys go in id order.
    pub fn compare(self, left: &PipelineSummary, right: &PipelineSummary) -> Ordering {
        let keys = match self {
            Self::FailingFirst => left
                .health_group()
                .cmp(&right.health_group())
                .then_with(|| left.compare_rates(right))
                .then_with(|| left.compare_names(right)),
            Self::Slowest => match (left.median, right.median) {
                (Some(left_median), Some(right_median)) => right_median.cmp(&left_median),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
            .then_with(|| left.compare_names(right)),
            Self::Name => left.compare_names(right),
        };
        keys.then_with(|| {
            left.pipeline_id
                .to_bytes()
                .cmp(&right.pipeline_id.to_bytes())
        })
    }
}

/// One page of summaries, with the counts of all the pages.
#[derive(Clone, Debug)]
pub struct SummaryPage {
    /// The summaries of the page, in the order of the request.
    pub items: Vec<PipelineSummary>,
    /// The number of pipelines on all the pages.
    pub total: u32,
    /// The number of green pipelines on all the pages.
    pub green: u32,
    /// The last pipeline of the page, if a next page exists.
    pub next_after: Option<PipelineId>,
}

impl SummaryPage {
    /// Sorts the summaries of all the pages, and keeps the page after the pipeline of the
    /// request. A pipeline that is not in the list gives an empty page.
    pub fn of(
        mut all: Vec<PipelineSummary>,
        order: SummaryOrder,
        page: PageRequest<PipelineId>,
    ) -> Self {
        // Count all the pipelines, then sort them.
        let count = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
        let total = count(all.len());
        let green = count(all.iter().filter(|summary| summary.is_green()).count());
        all.sort_by(|left, right| order.compare(left, right));

        // Keep the summaries after the pipeline of the request, then one page of them.
        let start = match page.after {
            None => 0,
            Some(after) => all
                .iter()
                .position(|summary| summary.pipeline_id == after)
                .map_or(all.len(), |index| index + 1),
        };
        let mut items = all.split_off(start);
        let size = page.size.get();
        let next_after = (items.len() > size).then(|| items[size - 1].pipeline_id);
        items.truncate(size);
        Self {
            items,
            total,
            green,
            next_after,
        }
    }
}
