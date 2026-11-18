//! The summaries of the triggers of a workspace for the dashboard: each trigger with its next
//! fires and its recent results, and the order of a list of summaries.

use std::cmp::Ordering;

use time::OffsetDateTime;

use super::{Trigger, TriggerFire, TriggerId};
use crate::projects::jobs::JobStatus;
use crate::projects::pipelines::{PipelineId, TriggerType};
use crate::projects::ProjectId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Page, PageRequest};

/// The most upcoming fires of one trigger.
pub const MAX_UPCOMING_FIRES: usize = 48;

/// The longest window of the upcoming fires, in days.
pub const MAX_UPCOMING_DAYS: i64 = 7;

/// The order of a list of trigger summaries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TriggerOrder {
    /// The soonest next fire first, then the name. No next fire comes last.
    #[default]
    NextFire,
    /// The name, with no difference between upper and lower case.
    Name,
}

/// The input of a list of the triggers of a workspace.
#[derive(Clone, Debug)]
pub struct ListWorkspaceTriggers {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The projects that the caller can see, or `None` for each project of the workspace.
    pub visible: Option<Vec<ProjectId>>,
    /// Only the triggers of these projects. Empty gives each project.
    pub projects: Vec<ProjectId>,
    /// Only the triggers of these pipelines. Empty gives each pipeline.
    pub pipelines: Vec<PipelineId>,
    /// Only the triggers of these kinds. Empty gives each kind.
    pub kinds: Vec<TriggerType>,
    /// `true` gives only the enabled triggers.
    pub enabled_only: bool,
    /// The order of the triggers.
    pub order: TriggerOrder,
    /// The end of the window of the upcoming fires, or `None` for no upcoming fire.
    pub until: Option<OffsetDateTime>,
    /// The start of the period of the counts of the failed fires and of the missed times.
    pub since: OffsetDateTime,
    /// The page to return.
    pub page: PageRequest<TriggerId>,
}

/// A trigger with its next fires and its recent results.
#[derive(Clone, Debug)]
pub struct TriggerSummary {
    /// The trigger.
    pub trigger: Trigger,
    /// The display name of the pipeline.
    pub pipeline_name: String,
    /// The display name of the project.
    pub project_name: String,
    /// `false` when the pipeline is archived, or when its file left the default branch.
    pub active: bool,
    /// The next fires in the window, oldest first.
    pub upcoming: Vec<OffsetDateTime>,
    /// The state now of the newest job of the trigger.
    pub last_job_status: Option<JobStatus>,
    /// The newest fire or missed time of the trigger.
    pub last_fire: Option<TriggerFire>,
    /// The fires that ran but made no job, since the start of the period.
    pub failed_fires: u32,
    /// The cron times that the server missed, since the start of the period.
    pub missed_times: u32,
}

impl TriggerSummary {
    /// Returns the next fire of an enabled cron trigger of an active pipeline.
    pub fn next_fire(&self) -> Option<OffsetDateTime> {
        match self.active && self.trigger.enabled {
            true => self.trigger.next_fire_at,
            false => None,
        }
    }

    /// Compares the names of two summaries with no difference between upper and lower case.
    fn compare_names(&self, other: &Self) -> Ordering {
        let lower = |name: &str| {
            name.chars()
                .flat_map(char::to_lowercase)
                .collect::<Vec<_>>()
        };
        lower(&self.trigger.name).cmp(&lower(&other.trigger.name))
    }
}

impl TriggerOrder {
    /// Compares two summaries in this order. Two summaries with the same keys go in id order.
    pub fn compare(self, left: &TriggerSummary, right: &TriggerSummary) -> Ordering {
        let keys = match self {
            Self::NextFire => match (left.next_fire(), right.next_fire()) {
                (Some(left_at), Some(right_at)) => left_at.cmp(&right_at),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
            .then_with(|| left.compare_names(right)),
            Self::Name => left.compare_names(right),
        };
        keys.then_with(|| left.trigger.id.to_bytes().cmp(&right.trigger.id.to_bytes()))
    }

    /// Sorts the summaries of all the pages, and keeps the page after the trigger of the
    /// request. A trigger that is not in the list gives an empty page.
    pub fn page(
        self,
        mut all: Vec<TriggerSummary>,
        page: PageRequest<TriggerId>,
    ) -> Page<TriggerSummary, TriggerId> {
        all.sort_by(|left, right| self.compare(left, right));
        let start = match page.after {
            None => 0,
            Some(after) => all
                .iter()
                .position(|summary| summary.trigger.id == after)
                .map_or(all.len(), |index| index + 1),
        };
        let mut items = all.split_off(start);
        let size = page.size.get();
        let next_after = (items.len() > size).then(|| items[size - 1].trigger.id);
        items.truncate(size);
        Page { items, next_after }
    }
}
