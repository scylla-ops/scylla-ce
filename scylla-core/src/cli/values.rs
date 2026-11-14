//! The value parsers of the settings: the durations in days, seconds and milliseconds.

use std::time::Duration;

/// Reads a duration in days.
pub(crate) fn days(text: &str) -> Result<Duration, String> {
    text.parse::<u64>()
        .map(|days| Duration::from_secs(days * 24 * 60 * 60))
        .map_err(|_| format!("{text:?} is not a number of days"))
}

/// Reads a duration in seconds.
pub(crate) fn seconds(text: &str) -> Result<Duration, String> {
    text.parse()
        .map(Duration::from_secs)
        .map_err(|_| format!("{text:?} is not a number of seconds"))
}

/// Reads a duration in milliseconds.
pub(crate) fn millis(text: &str) -> Result<Duration, String> {
    text.parse()
        .map(Duration::from_millis)
        .map_err(|_| format!("{text:?} is not a number of milliseconds"))
}
