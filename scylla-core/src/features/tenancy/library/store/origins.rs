//! The JSON form of the origin of a fragment in `fragments.origin`, and its change.

use scylla_domain::tenancy::library::{FragmentId, NewOrigin, Origin};
use scylla_extension::Tx;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

use crate::Error;

/// An origin in JSON. The time is in unix microseconds.
#[derive(Serialize, Deserialize)]
struct OriginJson {
    /// The URL of the registry.
    registry: String,
    /// The id of the package.
    package_id: String,
    /// The name of the package.
    package_name: String,
    /// The versions come from the package only.
    linked: bool,
    /// The version that the fragment started from.
    base_version: u32,
    /// The versions that a linked fragment chose not to take.
    skipped: Vec<u32>,
    /// The version that a fork chose not to see.
    dismissed: Option<u32>,
    /// The time of the add.
    added_at: i64,
}

/// Returns the JSON of the origin of a new fragment.
pub(crate) fn new_json(origin: &NewOrigin, at: OffsetDateTime) -> Value {
    json(&Origin {
        registry: origin.registry.clone(),
        package_id: origin.package_id.clone(),
        package_name: origin.package_name.clone(),
        linked: origin.linked,
        base_version: origin.version,
        skipped: Vec::new(),
        dismissed: None,
        added_at: at,
    })
}

/// Returns the JSON of an origin.
fn json(origin: &Origin) -> Value {
    let item = OriginJson {
        registry: origin.registry.clone(),
        package_id: origin.package_id.clone(),
        package_name: origin.package_name.clone(),
        linked: origin.linked,
        base_version: origin.base_version,
        skipped: origin.skipped.clone(),
        dismissed: origin.dismissed,
        added_at: (origin.added_at.unix_timestamp_nanos() / 1000) as i64,
    };
    serde_json::to_value(item).unwrap_or(Value::Null)
}

/// Reads an origin from its JSON. Bad data in the database gives `INTERNAL`.
pub(crate) fn read(value: Value) -> Result<Origin, Error> {
    let bad = |error: String| Error::Internal(format!("bad origin of a fragment: {error}"));
    let item: OriginJson = serde_json::from_value(value).map_err(|e| bad(e.to_string()))?;
    let added_at = OffsetDateTime::from_unix_timestamp_nanos(i128::from(item.added_at) * 1000)
        .map_err(|e| bad(e.to_string()))?;
    Ok(Origin {
        registry: item.registry,
        package_id: item.package_id,
        package_name: item.package_name,
        linked: item.linked,
        base_version: item.base_version,
        skipped: item.skipped,
        dismissed: item.dismissed,
        added_at,
    })
}

/// Saves the origin of a locked fragment.
pub(crate) async fn save(
    tx: &mut Tx<'_>,
    id: FragmentId,
    origin: &Origin,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE fragments SET origin = $2 WHERE id = $1",
        id.to_db(),
        json(origin),
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}
