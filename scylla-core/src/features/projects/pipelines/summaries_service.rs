//! The logic of the summaries of the pipelines for the dashboard.

use scylla_domain::projects::pipelines::{ListSummaries, SummaryPage};
use scylla_domain::projects::Visible;
use sqlx::PgPool;

use super::store;
use crate::Error;

/// Returns one page of the summaries of the pipelines that a caller can see in a workspace, with
/// the counts of all the pages and the recent jobs of each pipeline of the page.
pub(super) async fn list(
    pool: &PgPool,
    query: ListSummaries,
    visible: Visible,
) -> Result<SummaryPage, Error> {
    // Read the summaries of all the pipelines, then keep one page.
    let mut tx = pool.begin().await?;
    let all = store::summaries(&mut tx, &query, &visible).await?;
    let mut page = SummaryPage::of(all, query.order, query.page);

    // Add the recent jobs of the pipelines of the page.
    let ids: Vec<_> = page
        .items
        .iter()
        .map(|summary| summary.pipeline_id)
        .collect();
    let mut recent = store::recent_jobs(&mut tx, &ids, query.runs).await?;
    tx.commit().await?;
    for summary in &mut page.items {
        summary.recent = recent.remove(&summary.pipeline_id).unwrap_or_default();
    }
    Ok(page)
}
