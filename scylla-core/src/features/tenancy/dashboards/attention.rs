//! The open items of a workspace that need a person ("Needs you"): one transaction reads the
//! items of each kind that the query asks for, then the page keeps the newest ones.

use scylla_domain::identity::apps::AppId;
use scylla_domain::tenancy::dashboards::attention::{
    AttentionKind, AttentionList, AttentionQuery, ItemNode, Reach, Subject, OFFLINE_AFTER,
    OPEN_DAYS,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::Tx;
use sqlx::PgPool;
use time::{Duration, OffsetDateTime};

use super::store::{agents, fires, jobs};
use crate::Error;

/// Returns one page of the open items of a query at the time `now`, with their counts. The
/// agents of `connected` have an open stream.
pub(super) async fn list(
    pool: &PgPool,
    mut query: AttentionQuery,
    connected: &[AppId],
    now: OffsetDateTime,
) -> Result<AttentionList, Error> {
    // Keep the projects of the workspace in what the caller reads.
    let mut tx = pool.begin().await?;
    let workspace = query.workspace;
    for reach in [
        &mut query.reach.jobs,
        &mut query.reach.fires,
        &mut query.reach.approvals,
    ] {
        *reach = in_workspace(&mut tx, workspace, reach).await?;
    }

    // Read the open items of each kind that the query asks for, and the last time one went away.
    let from = now - Duration::days(OPEN_DAYS);
    let (mut items, mut cleared) = (Vec::new(), Vec::new());
    let failures = statuses(
        &query,
        [AttentionKind::JobFailed, AttentionKind::JobOrphaned],
    );
    if !failures.is_empty() {
        items.extend(jobs::failures(&mut tx, &query, &failures, from).await?);
        cleared.push(jobs::failures_cleared(&mut tx, &query, &failures, from).await?);
    }
    let bad_fires = statuses(
        &query,
        [AttentionKind::FireFailed, AttentionKind::FireMissed],
    );
    if !bad_fires.is_empty() {
        items.extend(fires::open(&mut tx, &query, &bad_fires, from).await?);
        cleared.push(fires::cleared(&mut tx, &query, &bad_fires, from).await?);
    }
    if query.asks(AttentionKind::ApprovalWaiting) {
        items.extend(jobs::approvals(&mut tx, &query).await?);
        cleared.push(jobs::approvals_cleared(&mut tx, &query, from).await?);
    }
    if query.asks(AttentionKind::AgentOffline) {
        let before = now - OFFLINE_AFTER;
        items.extend(agents::offline(&mut tx, workspace, connected, before).await?);
    }

    // Keep one page, then find the node of each job of the page.
    let mut list = AttentionList::page(items, &query, cleared.into_iter().flatten().max());
    let ids: Vec<_> = list
        .items
        .iter()
        .filter_map(|item| match &item.subject {
            Subject::Job(job) if item.kind != AttentionKind::ApprovalWaiting => Some(job.id),
            _ => None,
        })
        .collect();
    let found = jobs::nodes(&mut tx, &ids).await?;
    tx.commit().await?;
    for item in &mut list.items {
        if let Subject::Job(job) = &mut item.subject {
            if let Some((_, nodes, runs)) = found.iter().find(|(id, _, _)| *id == job.id) {
                job.node = ItemNode::of(job.status, nodes, runs);
            }
        }
    }
    Ok(list)
}

/// Returns the states in the database of the job or fire kinds that the query asks for.
fn statuses(query: &AttentionQuery, kinds: [AttentionKind; 2]) -> Vec<&'static str> {
    kinds
        .into_iter()
        .filter(|kind| query.asks(*kind))
        .map(|kind| match kind {
            AttentionKind::JobFailed => "failed",
            AttentionKind::JobOrphaned => "orphaned",
            AttentionKind::FireFailed => "error",
            _ => "missed",
        })
        .collect()
}

/// Keeps the projects of a workspace in a reach to some projects.
async fn in_workspace(
    tx: &mut Tx<'_>,
    workspace: WorkspaceId,
    reach: &Reach,
) -> Result<Reach, Error> {
    match reach {
        Reach::Projects(projects) => {
            let kept = jobs::projects_in(tx, workspace, projects).await?;
            Ok(Reach::of(Some(kept)))
        }
        Reach::Nothing | Reach::Workspace => Ok(reach.clone()),
    }
}
