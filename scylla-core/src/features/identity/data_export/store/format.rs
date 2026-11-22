//! The format of the values of the export.

use serde_json::Value;
use time::OffsetDateTime;

/// Writes a time in RFC 3339, in UTC, to the second.
pub(crate) fn time(at: OffsetDateTime) -> Value {
    chrono::DateTime::from_timestamp(at.unix_timestamp(), 0).map_or(Value::Null, |at| {
        Value::String(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
    })
}
