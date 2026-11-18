//! The busy job slots of the agents of a workspace: a job holds one slot of its agent from its
//! start to its end. The mean and the largest number of busy slots of a period and of its buckets
//! of hours, days or weeks in the time zone of the workspace.

use time::{Duration, OffsetDateTime};

use super::place;
use crate::identity::apps::AppId;
use crate::projects::jobs::FiguresPeriod;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Span, TimeZone};

/// The busy slots of a period or of a bucket.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SlotFigures {
    /// The mean number of busy slots.
    pub busy_avg: f64,
    /// The largest number of slots that were busy at the same time.
    pub busy_max: u32,
}

/// The busy slots of one hour, day or week.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotBucket {
    /// The start in the time zone of the workspace.
    pub start: OffsetDateTime,
    /// The busy slots of the bucket.
    pub figures: SlotFigures,
}

/// The input of the slot usage of the agents of a workspace.
#[derive(Clone, Copy, Debug)]
pub struct SlotQuery {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// Only the slots of this agent, or `None` for each agent.
    pub agent: Option<AppId>,
    /// The period.
    pub period: FiguresPeriod,
    /// The size of the buckets, or `None` for no bucket.
    pub bucket: Option<Span>,
    /// The time of the request: a running job holds its slot up to it.
    pub now: OffsetDateTime,
}

impl SlotQuery {
    /// Returns the end of the time that the slots count: the end of the period, or now if it is
    /// earlier.
    pub fn end(&self) -> OffsetDateTime {
        self.period.to.min(self.now)
    }
}

/// The busy time of one part of the period, as the store counts it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Busy {
    /// The sum of the times that the jobs held a slot in the part.
    pub time: Duration,
    /// The largest number of slots that were busy at the same time in the part.
    pub max: u32,
}

/// The slot usage of the agents of a workspace.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotUsage {
    /// The busy slots of the period.
    pub totals: SlotFigures,
    /// Each bucket of the period, oldest first.
    pub buckets: Vec<SlotBucket>,
    /// The slots of the agents that are connected now.
    pub capacity: u32,
    /// The time zone of the buckets.
    pub time_zone: TimeZone,
}

impl SlotUsage {
    /// Makes the slot usage from the busy time of each part of `bounds` with a job: number 1 is
    /// the first part. The mean of a part is its busy time divided by its length up to the end of
    /// the query. A part with no length has a mean of 0.
    pub fn of_parts(
        query: &SlotQuery,
        starts: &[OffsetDateTime],
        bounds: &[OffsetDateTime],
        found: &[(usize, Busy)],
        time_zone: TimeZone,
    ) -> Self {
        // Find the busy time of each part.
        let end = query.end();
        let parts = place(bounds.len(), found);
        let length = |place: usize| {
            let next = bounds.get(place + 1).copied().unwrap_or(end).min(end);
            next - bounds[place]
        };

        // Make the buckets, then the totals of all the parts.
        let buckets = starts
            .iter()
            .zip(&parts)
            .enumerate()
            .map(|(place, (start, busy))| SlotBucket {
                start: *start,
                figures: figures(*busy, length(place)),
            })
            .collect();
        let whole = Busy {
            time: parts.iter().map(|busy| busy.time).sum(),
            max: parts.iter().map(|busy| busy.max).max().unwrap_or(0),
        };
        Self {
            totals: figures(whole, end - query.period.from),
            buckets,
            capacity: 0,
            time_zone,
        }
    }
}

/// Returns the figures of a busy time in a length of time.
fn figures(busy: Busy, length: Duration) -> SlotFigures {
    let busy_avg = match length.is_positive() {
        true => busy.time / length,
        false => 0.0,
    };
    SlotFigures {
        busy_avg,
        busy_max: busy.max,
    }
}
