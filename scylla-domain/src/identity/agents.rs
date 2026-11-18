//! The agents: the Apps that run jobs, with their stream, the busy slots of their jobs, and the
//! samples of the use of their machines.

mod samples;
mod slots;

#[cfg(test)]
mod tests;

use time::OffsetDateTime;

use crate::identity::actors::Actor;
use crate::identity::apps::App;
use crate::projects::jobs::FiguresPeriod;
use crate::tenancy::workspaces::Workspace;

pub use samples::{
    AgentMetrics, HostSample, MetricsBucket, MetricsFigures, MetricsQuery, Sample, SampleSums,
    SAMPLE_GAP, SAMPLE_INTERVAL, SAMPLE_RETENTION,
};
pub use slots::{Busy, SlotBucket, SlotFigures, SlotQuery, SlotUsage};

crate::authz! {
    /// The routes of an agent on itself.
    extend Actor as AgentCaller {
        /// The route `OpenStream`: an agent opens its own stream.
        permission open_stream = nil;
    }

    /// The routes of the agents on their workspace.
    extend Workspace as AgentHolder {
        /// The route `GetSlotUsage`: the busy slots of the agents of this workspace.
        permission get_slot_usage = read;
        /// The route `GetAgentMetrics`, on each App of this workspace.
        permission get_agent_metrics = read;
    }

    /// The routes of the agents on one App.
    extend App as AgentAccess {
        /// The route `GetAgentMetrics`.
        permission get_agent_metrics = creator;
    }
}

/// Returns the bounds of the parts of a period: its start, then the start of each bucket after
/// the first. With no bucket, the period is one part.
pub fn bounds(period: FiguresPeriod, starts: &[OffsetDateTime]) -> Vec<OffsetDateTime> {
    let mut bounds = vec![period.from];
    bounds.extend(starts.iter().skip(1).copied());
    bounds
}

/// Returns the value of each part of `count` parts from the part numbers of `found`: number 1 is
/// the first part. A part that is not in `found` gets the default value.
fn place<T: Copy + Default>(count: usize, found: &[(usize, T)]) -> Vec<T> {
    (1..=count)
        .map(|number| {
            found
                .iter()
                .find(|(place, _)| *place == number)
                .map(|(_, value)| *value)
                .unwrap_or_default()
        })
        .collect()
}
