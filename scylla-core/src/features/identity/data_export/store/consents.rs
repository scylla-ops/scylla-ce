//! The reads of the records of the consent of a user for its export, as JSON.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::consent::ConsentId;
use scylla_extension::Tx;
use serde_json::{json, Value};

use super::time;
use crate::Error;

/// Returns the records of the choices that a user made on the trackers that need consent, oldest
/// first.
pub(crate) async fn consents(tx: &mut Tx<'_>, user: ActorId) -> Result<Vec<Value>, Error> {
    let rows = sqlx::query!(
        "SELECT consent_id, banner_version, banner_hash, choices, action, source, recorded_at
         FROM consent_records
         WHERE user_id = $1
         ORDER BY recorded_at, id",
        user.to_db()
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "consent_id": ConsentId::from_db(row.consent_id).to_string(),
                "banner_version": row.banner_version,
                "banner_hash": row.banner_hash,
                "choices": row.choices,
                "action": row.action,
                "source": row.source,
                "recorded_at": time(row.recorded_at),
            })
        })
        .collect())
}
