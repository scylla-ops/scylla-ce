//! The API tokens: long secrets that a program exchanges for short tokens of their actor. The
//! server keeps only a hash of each secret.

use time::{Duration, OffsetDateTime};

use crate::identity::actors::ActorId;
use crate::{Id, Name};

/// The start of the secret of an API token of an App.
pub const APP_TOKEN_PREFIX: &str = "scya_";

/// The number of last characters of a secret that its hint shows.
pub const HINT_LEN: usize = 4;

/// The marker of an API token id.
pub enum ApiToken {}

/// The id of an API token.
pub type ApiTokenId = Id<ApiToken>;

/// The name of an API token.
pub type TokenName = Name;

/// The life of a new API token. A token always expires.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TokenExpiry {
    /// 7 days.
    Days7,
    /// 30 days.
    Days30,
    /// 90 days.
    #[default]
    Days90,
    /// 365 days.
    Days365,
}

impl TokenExpiry {
    /// Returns the life of a token.
    pub fn duration(self) -> Duration {
        Duration::days(match self {
            Self::Days7 => 7,
            Self::Days30 => 30,
            Self::Days90 => 90,
            Self::Days365 => 365,
        })
    }
}

/// An API token, as the store returns it. It has no secret.
#[derive(Clone, Debug)]
pub struct ApiTokenInfo {
    /// The id.
    pub id: ApiTokenId,
    /// The actor of the token.
    pub actor_id: ActorId,
    /// The name.
    pub name: String,
    /// The prefix and the last characters of the secret.
    pub hint: String,
    /// The actor that created the token.
    pub created_by: ActorId,
    /// The creation time.
    pub created_at: OffsetDateTime,
    /// The end of its validity.
    pub expires_at: OffsetDateTime,
    /// The last exchange of the token.
    pub last_used_at: Option<OffsetDateTime>,
    /// The time of its revocation.
    pub revoked_at: Option<OffsetDateTime>,
}

/// Returns the hint of a secret: its prefix, `...`, and its last characters.
pub fn hint(prefix: &str, secret: &str) -> String {
    let start = secret.len().saturating_sub(HINT_LEN);
    let end = secret.get(start..).unwrap_or_default();
    format!("{prefix}...{end}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hint_shows_the_prefix_and_the_end_only() {
        assert_eq!(hint(APP_TOKEN_PREFIX, "scya_abcdefgh"), "scya_...efgh");
    }

    #[test]
    fn the_default_life_is_90_days_and_the_longest_is_one_year() {
        assert_eq!(TokenExpiry::default().duration(), Duration::days(90));
        assert_eq!(TokenExpiry::Days365.duration(), Duration::days(365));
    }
}
