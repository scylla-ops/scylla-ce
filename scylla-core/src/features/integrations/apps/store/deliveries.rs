//! The SQL of the last webhook of a GitHub App in `vcs_deliveries`.

use scylla_domain::integrations::apps::VcsAppId;
use scylla_extension::Tx;
use time::OffsetDateTime;

use crate::Error;

/// The last webhook of an App that the server kept.
pub(crate) struct LastDelivery {
    /// The time of its arrival.
    pub(crate) received_at: OffsetDateTime,
    /// The HTTP status of the answer of the server, or `None`.
    pub(crate) status: Option<i32>,
}

/// Returns the last webhook of the App of the instance with this id, or of the App of the
/// settings for `None`. `None` before the first one.
pub(crate) async fn last(
    tx: &mut Tx<'_>,
    app: Option<VcsAppId>,
) -> Result<Option<LastDelivery>, Error> {
    let row = sqlx::query!(
        "SELECT received_at, status FROM vcs_deliveries
         WHERE provider = 'github' AND app_id IS NOT DISTINCT FROM $1
         ORDER BY received_at DESC LIMIT 1",
        app.map(VcsAppId::to_db),
    )
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| LastDelivery {
        received_at: row.received_at,
        status: row.status,
    }))
}
