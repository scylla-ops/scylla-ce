//! The counts of the webhook deliveries of a workspace for the dashboard: the deliveries of its
//! GitHub and GitLab connections and the fires of its webhook triggers in a period, for each
//! connection, each trigger and each hour, day or week in the time zone of the workspace.

#[cfg(test)]
mod tests;

use time::OffsetDateTime;

use super::connections::VcsConnectionId;
use crate::projects::jobs::FiguresPeriod;
use crate::projects::triggers::TriggerId;
use crate::projects::ProjectId;
use crate::tenancy::workspaces::WorkspaceId;
use crate::{Span, TimeZone};

/// The counts of a set of deliveries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DeliveryFigures {
    /// Each delivery that the server kept, also the failed ones.
    pub delivered: u32,
    /// The deliveries whose work failed.
    pub failed: u32,
}

impl DeliveryFigures {
    /// Adds the counts of another set.
    fn add(&mut self, other: Self) {
        self.delivered = self.delivered.saturating_add(other.delivered);
        self.failed = self.failed.saturating_add(other.failed);
    }
}

/// The deliveries of one connection or one webhook trigger in one bucket, as the SQL counts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BucketCount<K> {
    /// The connection or the trigger.
    pub key: K,
    /// The number of the bucket, from 1. With no bucket, number 1 is the whole period.
    pub bucket: usize,
    /// The counts.
    pub figures: DeliveryFigures,
    /// The arrival of the newest delivery of the bucket.
    pub last_received_at: OffsetDateTime,
}

/// The newest delivery of a connection in the period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LastDelivery {
    /// The connection.
    pub connection: VcsConnectionId,
    /// The arrival.
    pub received_at: OffsetDateTime,
    /// The event, such as `push`.
    pub event: String,
    /// The full name of the repository of a Git event.
    pub repository: Option<String>,
    /// The Git ref of a Git event, such as `refs/heads/main`.
    pub git_ref: Option<String>,
}

/// The deliveries of one connection in the period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionDeliveries {
    /// The connection.
    pub connection: VcsConnectionId,
    /// The counts.
    pub figures: DeliveryFigures,
    /// The newest delivery.
    pub last: Option<LastDelivery>,
}

/// The deliveries of one webhook trigger in the period.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerDeliveries {
    /// The trigger.
    pub trigger: TriggerId,
    /// The counts.
    pub figures: DeliveryFigures,
    /// The arrival of the newest delivery.
    pub last_received_at: OffsetDateTime,
}

/// The input of the delivery counts of a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryStatsQuery {
    /// The workspace.
    pub workspace: WorkspaceId,
    /// The projects whose webhook triggers the caller can see, or `None` for each project.
    pub visible: Option<Vec<ProjectId>>,
    /// The period of the arrivals.
    pub period: FiguresPeriod,
    /// The size of the buckets, or `None` for no bucket.
    pub bucket: Option<Span>,
}

/// The delivery counts of a workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryStats {
    /// The deliveries of the period.
    pub totals: DeliveryFigures,
    /// Each connection with a delivery, in the order of the connections.
    pub connections: Vec<ConnectionDeliveries>,
    /// Each webhook trigger with a delivery: the most failed first, then the most deliveries.
    pub triggers: Vec<TriggerDeliveries>,
    /// The start of each bucket of the period with its deliveries, oldest first.
    pub buckets: Vec<(OffsetDateTime, DeliveryFigures)>,
    /// The time zone of the buckets.
    pub time_zone: TimeZone,
}

impl DeliveryStats {
    /// Makes the counts from the counts of each bucket: the starts of the buckets (none for no
    /// bucket), the connections in their order, their newest deliveries, and the triggers.
    pub fn new(
        starts: &[OffsetDateTime],
        connections: &[BucketCount<VcsConnectionId>],
        lasts: Vec<LastDelivery>,
        triggers: &[BucketCount<TriggerId>],
        time_zone: TimeZone,
    ) -> Self {
        // Add the counts of all the sources, and of each bucket.
        let mut totals = DeliveryFigures::default();
        let mut buckets: Vec<_> = starts
            .iter()
            .map(|start| (*start, DeliveryFigures::default()))
            .collect();
        let all = connections
            .iter()
            .map(|count| (count.bucket, count.figures));
        for (bucket, figures) in all.chain(triggers.iter().map(|c| (c.bucket, c.figures))) {
            totals.add(figures);
            if let Some((_, sum)) = bucket.checked_sub(1).and_then(|at| buckets.get_mut(at)) {
                sum.add(figures);
            }
        }

        // Add the counts of each connection, with its newest delivery.
        let connections = merge(connections)
            .into_iter()
            .map(|(connection, figures, _)| ConnectionDeliveries {
                connection,
                figures,
                last: lasts
                    .iter()
                    .find(|last| last.connection == connection)
                    .cloned(),
            })
            .collect();

        // Add the counts of each trigger, and put the most failed first.
        let mut triggers: Vec<_> = merge(triggers)
            .into_iter()
            .map(|(trigger, figures, last_received_at)| TriggerDeliveries {
                trigger,
                figures,
                last_received_at,
            })
            .collect();
        triggers.sort_by(|a, b| {
            (b.figures.failed, b.figures.delivered)
                .cmp(&(a.figures.failed, a.figures.delivered))
                .then_with(|| a.trigger.to_bytes().cmp(&b.trigger.to_bytes()))
        });
        Self {
            totals,
            connections,
            triggers,
            buckets,
            time_zone,
        }
    }
}

/// Adds the counts of the buckets of each key, in the order of the first count of each key, with
/// the arrival of its newest delivery.
fn merge<K: Copy + Eq>(counts: &[BucketCount<K>]) -> Vec<(K, DeliveryFigures, OffsetDateTime)> {
    let mut merged: Vec<(K, DeliveryFigures, OffsetDateTime)> = Vec::new();
    for count in counts {
        match merged.iter_mut().find(|(key, ..)| *key == count.key) {
            Some((_, figures, last)) => {
                figures.add(count.figures);
                *last = (*last).max(count.last_received_at);
            }
            None => merged.push((count.key, count.figures, count.last_received_at)),
        }
    }
    merged
}
