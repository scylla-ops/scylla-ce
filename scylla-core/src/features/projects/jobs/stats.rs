//! The job stats of the agents, for their screens: the ends of their jobs on each of the last
//! days, and their durations.

use scylla_domain::identity::apps::AppId;
use scylla_domain::projects::jobs::{AgentStats, Outcomes};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::store::stats;
use crate::Error;

/// Returns the stats of an agent of a workspace. An agent with no job gets stats with each
/// count at 0.
pub(crate) async fn of_agent(
    pool: &PgPool,
    workspace: WorkspaceId,
    agent: AppId,
) -> Result<AgentStats, Error> {
    let since = AgentStats::since(OffsetDateTime::now_utc());
    let found = collect(pool, workspace, Some(agent), since).await?.pop();
    Ok(found.unwrap_or_else(|| AgentStats {
        agent_id: agent,
        outcomes: Outcomes::default(),
        median_duration: None,
        p95_duration: None,
        days: AgentStats::days(since, &[]),
    }))
}

/// Returns the stats of each agent of a workspace with a job in the days of the stats, in the
/// order of their ids.
pub(crate) async fn of_workspace(
    pool: &PgPool,
    workspace: WorkspaceId,
) -> Result<Vec<AgentStats>, Error> {
    let since = AgentStats::since(OffsetDateTime::now_utc());
    collect(pool, workspace, None, since).await
}

/// Reads the totals and the days of the agents in one transaction, and puts them together.
async fn collect(
    pool: &PgPool,
    workspace: WorkspaceId,
    agent: Option<AppId>,
    since: OffsetDateTime,
) -> Result<Vec<AgentStats>, Error> {
    let mut tx = pool.begin().await?;
    let totals = stats::totals(&mut tx, workspace, agent, since).await?;
    let days = stats::days(&mut tx, workspace, agent, since).await?;
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
                days: AgentStats::days(since, &busy),
            }
        })
        .collect())
}
