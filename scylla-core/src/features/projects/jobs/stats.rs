//! The job stats of the agents, for their screens: the ends of their jobs on each day of a
//! period, and their durations.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{AgentStats, Outcomes, StatsPeriod};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::store::stats;
use crate::Error;

/// Returns the stats of an agent of a workspace for a period. An agent with no job gets stats
/// with each count at 0.
pub(crate) async fn of_agent(
    pool: &PgPool,
    workspace: WorkspaceId,
    agent: AppId,
    period: StatsPeriod,
) -> Result<AgentStats, Error> {
    let found = collect(pool, workspace, Some(agent), period).await?.pop();
    Ok(found.unwrap_or_else(|| AgentStats {
        agent_id: agent,
        outcomes: Outcomes::default(),
        median_duration: None,
        p95_duration: None,
        days: period.days(&[]),
    }))
}

/// Returns the stats of the recent days of each agent of a workspace with a job in these days,
/// in the order of their ids.
pub(crate) async fn of_workspace(
    pool: &PgPool,
    workspace: WorkspaceId,
) -> Result<Vec<AgentStats>, Error> {
    let period = StatsPeriod::recent(OffsetDateTime::now_utc());
    collect(pool, workspace, None, period).await
}

/// Reads the totals and the days of the agents in one transaction, and puts them together.
async fn collect(
    pool: &PgPool,
    workspace: WorkspaceId,
    agent: Option<AppId>,
    period: StatsPeriod,
) -> Result<Vec<AgentStats>, Error> {
    let mut tx = pool.begin().await?;
    let totals = stats::totals(&mut tx, workspace, agent, period).await?;
    let days = stats::days(&mut tx, workspace, agent, period).await?;
    tx.commit().await?;
    Ok(totals
        .into_iter()
        .map(|totals| {
            let busy: Vec<_> = days
                .iter()
                .filter(|(id, _)| *id == totals.agent_id)
                .map(|(_, day)| *day)
                .collect();
            AgentStats {
                agent_id: totals.agent_id,
                outcomes: totals.outcomes,
                median_duration: totals.median,
                p95_duration: totals.p95,
                days: period.days(&busy),
            }
        })
        .collect())
}
