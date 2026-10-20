use crate::domain::errors::{DomainError, DomainResult};
use std::fmt::Display;

pub(crate) fn to_db(version: u64) -> i64 {
    i64::try_from(version).unwrap_or(i64::MAX)
}

pub(crate) fn from_db(version: i64) -> u64 {
    u64::try_from(version).unwrap_or(0)
}

/// The outcome of a versioned write. A write that matched no row is a stale read if `read`
/// still finds the row, else the `NotFound` of `read`.
pub(crate) async fn written<T, R>(
    outcome: Option<T>,
    entity_type: &'static str,
    id: impl Display,
    read: impl Future<Output = DomainResult<R>>,
) -> DomainResult<T> {
    if let Some(written) = outcome {
        return Ok(written);
    }
    read.await?;
    Err(DomainError::stale(entity_type, id))
}
