//! The figures of the jobs of a workspace for the dashboard: one transaction reads the time zone
//! of the workspace, then counts the jobs of the period, of its buckets, of its groups and of the
//! previous period.

use scylla_domain::projects::jobs::{self, FiguresQuery, JobFigures};
use sqlx::PgPool;

use super::store::figures::{self, Slice};
use crate::Error;

/// Returns the figures of the jobs of a query. Refuses more buckets than the most in the time
/// zone of the workspace.
pub(super) async fn get(pool: &PgPool, query: FiguresQuery) -> Result<JobFigures, Error> {
    // Read the time zone of the workspace, and find the buckets of the period in it.
    let mut tx = pool.begin().await?;
    let zone = figures::time_zone(&mut tx, query.jobs.workspace).await?;
    let starts = match query.bucket {
        Some(span) => query
            .period
            .starts(zone, span)
            .map_err(|error| Error::invalid("bucket", error.to_string()))?,
        None => Vec::new(),
    };

    // Count the jobs of the period: all of them, then by bucket and by group.
    let slice = Slice {
        jobs: &query.jobs,
        origins: &query.origins,
        period: query.period,
    };
    let totals = figures::totals(&mut tx, slice).await?;
    let found = match starts.is_empty() {
        true => Vec::new(),
        false => figures::buckets(&mut tx, slice, &starts).await?,
    };
    let groups = match query.group_by {
        Some(by) => figures::groups(&mut tx, slice, by).await?,
        None => Vec::new(),
    };

    // Count the jobs of the previous period, all of them and by group.
    let (mut previous, mut before) = (None, None);
    if query.compare {
        let slice = Slice {
            period: query.period.previous(),
            ..slice
        };
        previous = Some(figures::totals(&mut tx, slice).await?);
        if let Some(by) = query.group_by {
            before = Some(figures::groups(&mut tx, slice, by).await?);
        }
    }
    tx.commit().await?;
    Ok(JobFigures {
        totals,
        buckets: jobs::buckets(&starts, &found),
        groups: jobs::top(groups, before, query.order, query.limit),
        previous,
        time_zone: zone,
    })
}
