//! The settings of the retention: the time that the server keeps the personal data of a row
//! after its use.

use std::time::Duration;

use clap::Args;

use crate::cli::values::days;

/// The number of seconds in one day.
const DAY: u64 = 24 * 60 * 60;

/// The time that the server keeps a row with personal data after its use. Each flag also reads
/// the environment variable in its doc.
#[derive(Args, Clone, Debug, PartialEq, Eq)]
#[group(id = "retention")]
pub struct RetentionSettings {
    /// The time that the server keeps a closed session, with its IP address and its user agent,
    /// after its end or its revocation, in days.
    #[arg(long, env = "SCYLLA_SESSION_HISTORY_DAYS", default_value = "90", value_parser = days)]
    pub session_history: Duration,

    /// The time that the server keeps the link of a mail (a check of an email, a password reset,
    /// a change of email) after its use or its end, in days.
    #[arg(long, env = "SCYLLA_ONE_TIME_TOKEN_RETENTION_DAYS", default_value = "30", value_parser = days)]
    pub one_time_token_retention: Duration,

    /// The time that the server keeps an invitation, with its email, after its acceptance or its
    /// end, in days.
    #[arg(long, env = "SCYLLA_INVITATION_RETENTION_DAYS", default_value = "90", value_parser = days)]
    pub invitation_retention: Duration,
}

impl Default for RetentionSettings {
    fn default() -> Self {
        Self {
            session_history: Duration::from_secs(90 * DAY),
            one_time_token_retention: Duration::from_secs(30 * DAY),
            invitation_retention: Duration::from_secs(90 * DAY),
        }
    }
}
