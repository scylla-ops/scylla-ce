//! The SQL of the `consent_records` table: one row for each choice of a browser.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::consent::{ConsentId, ConsentRecordId, NewConsentRecord};
use scylla_extension::Tx;
use serde_json::{Map, Value};
use time::OffsetDateTime;

/// Inserts the record of a choice, and returns its time.
pub(super) async fn insert(
    tx: &mut Tx<'_>,
    consent: ConsentId,
    record: &NewConsentRecord,
    user: Option<ActorId>,
) -> Result<OffsetDateTime, sqlx::Error> {
    let choices: Map<String, Value> = record
        .choices
        .iter()
        .map(|choice| (choice.purpose.as_str().to_owned(), choice.granted.into()))
        .collect();
    sqlx::query_scalar!(
        "INSERT INTO consent_records
             (id, consent_id, user_id, banner_version, banner_hash, choices, action, source)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING recorded_at",
        ConsentRecordId::generate().to_db(),
        consent.to_db(),
        user.map(ActorId::to_db),
        record.banner_version,
        record.banner_hash,
        Value::Object(choices),
        record.action.as_str(),
        record.source.as_str(),
    )
    .fetch_one(&mut **tx)
    .await
}
