//! What the sender does after a failed call: a later call at the time of a limit or after a
//! wait that grows, a connection that needs a person after a refusal, or a stop.

use scylla_domain::integrations::VcsProvider;
use time::{Duration, OffsetDateTime};

use crate::infra::vcs::ProviderError;

/// The wait after the first failed call.
const FIRST_WAIT: Duration = Duration::seconds(10);

/// The longest wait between two calls.
const LONGEST_WAIT: Duration = Duration::minutes(15);

/// A call that failed.
#[derive(Debug)]
pub(super) enum Failure {
    /// The provider refused or failed the call.
    Provider(ProviderError),
    /// The server cannot make the call: no App, no key, or bad data. A later call cannot work.
    Server(String),
}

/// What the sender does after a failed call.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Next {
    /// Call again at this time.
    Retry(OffsetDateTime),
    /// The provider refused the credentials: the connection needs a person, for this reason.
    Broken(&'static str),
    /// Stop the report.
    Stop,
}

impl From<ProviderError> for Failure {
    fn from(error: ProviderError) -> Self {
        Self::Provider(error)
    }
}

impl Failure {
    /// Returns what the sender does after this failure, for a report with `attempts` failed
    /// calls, at the time `now`. A GitLab token whose expiry passed gives `TOKEN_EXPIRED`.
    pub(super) fn next(
        &self,
        provider: VcsProvider,
        attempts: i32,
        expired: bool,
        now: OffsetDateTime,
    ) -> Next {
        let Self::Provider(error) = self else {
            return Next::Stop;
        };
        match (provider, error) {
            (_, ProviderError::RateLimited(until)) => Next::Retry((*until).max(now)),
            (_, ProviderError::Transient(_)) => Next::Retry(now + wait(attempts)),
            (_, ProviderError::Invalid(_)) => Next::Stop,
            (VcsProvider::GitHub, ProviderError::Forbidden) => {
                Next::Broken("PERMISSIONS_NOT_ACCEPTED")
            }
            (VcsProvider::GitHub, _) => Next::Broken("APP_UNINSTALLED"),
            (VcsProvider::GitLab, ProviderError::Forbidden) => Next::Broken("TOKEN_SCOPE_MISSING"),
            (VcsProvider::GitLab, _) if expired => Next::Broken("TOKEN_EXPIRED"),
            (VcsProvider::GitLab, _) => Next::Broken("TOKEN_REVOKED"),
        }
    }

    /// Returns the error of the report: a short text with no secret, that names the provider.
    pub(super) fn text(&self, provider: VcsProvider) -> String {
        let name = match provider {
            VcsProvider::GitHub => "GitHub",
            VcsProvider::GitLab => "GitLab",
        };
        match self {
            Self::Provider(error) => format!("{name}: {error}"),
            Self::Server(reason) => reason.clone(),
        }
    }
}

/// Returns the wait after a failed call, for a report with `attempts` failed calls before it: 10
/// seconds, then twice as long after each failure, at most 15 minutes.
pub(super) fn wait(attempts: i32) -> Duration {
    let doubled = 2_i32.saturating_pow(u32::try_from(attempts.clamp(0, 16)).unwrap_or(16));
    (FIRST_WAIT * doubled).min(LONGEST_WAIT)
}
