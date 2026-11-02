use prost_types::Timestamp;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::Error;

pub fn uuid(value: &str, field: &'static str) -> Result<Uuid, Error> {
    Uuid::parse_str(value).map_err(|_| Error::InvalidArgument {
        field,
        reason: "must be a UUID",
    })
}

pub fn timestamp(at: OffsetDateTime) -> Timestamp {
    Timestamp {
        seconds: at.unix_timestamp(),
        nanos: at.nanosecond() as i32,
    }
}
