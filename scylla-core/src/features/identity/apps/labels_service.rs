//! The logic of the labels that the admins give to an App: one transaction for each call.

use scylla_domain::identity::agents::AgentLabels;
use scylla_domain::identity::apps::{App, AppId};
use sqlx::PgPool;

use super::store::apps;
use crate::features::identity::agents::Registry;
use crate::Error;

/// Replaces the labels that the admins give to an App, then tells its stream and wakes the
/// streams: a pending job can fit the agent now. An unknown App gives `NotFound("app")`.
pub(crate) async fn set_labels(
    pool: &PgPool,
    registry: &Registry,
    id: AppId,
    labels: AgentLabels,
) -> Result<App, Error> {
    let mut tx = pool.begin().await?;
    let app = apps::set_labels(&mut tx, id, &labels)
        .await?
        .ok_or(Error::NotFound("app"))?;
    tx.commit().await?;
    let count = labels.as_slice().len();
    registry.relabel(id, labels);
    tracing::info!(event = "app.labels_changed", app = %id, count);
    Ok(app)
}
