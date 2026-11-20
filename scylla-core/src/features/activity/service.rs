//! The logic of the feed that people read, and the delete of the old rows: one transaction for
//! each call.

use scylla_domain::activity::{ActivityPage, ListActivity, RETENTION, SINCE_CAP};
use scylla_domain::Page;
use sqlx::PgPool;
use time::OffsetDateTime;

use super::store;
use crate::Error;

/// Returns one page of the rows that a reader sees, newest first, and the number of rows since
/// the time of the query. A reader that sees nothing gets no row and reads nothing.
pub(super) async fn list(pool: &PgPool, query: ListActivity) -> Result<ActivityPage, Error> {
    let visible = &query.visible;
    if visible.areas.is_empty() && visible.projects.is_empty() {
        return Ok(ActivityPage {
            page: Page {
                items: Vec::new(),
                next_after: None,
            },
            since_count: 0,
        });
    }
    let mut tx = pool.begin().await?;
    let items = store::list(&mut tx, query.workspace, visible, query.page).await?;
    let since_count = match query.since {
        Some(since) => {
            store::count_since(&mut tx, query.workspace, visible, since, SINCE_CAP).await?
        }
        None => 0,
    };
    tx.commit().await?;
    Ok(ActivityPage {
        page: Page::from_overfetch(items, query.page.size, |row| row.id),
        since_count: i32::try_from(since_count).unwrap_or(SINCE_CAP),
    })
}

/// Deletes the rows older than `RETENTION`. Returns their number.
pub(crate) async fn purge(pool: &PgPool) -> Result<u64, Error> {
    let before = OffsetDateTime::now_utc() - RETENTION;
    let mut tx = pool.begin().await?;
    let count = store::purge(&mut tx, before).await?;
    tx.commit().await?;
    Ok(count)
}
