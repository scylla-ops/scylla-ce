//! The state of the web app for one user: small JSON values that the server keeps and does not
//! read.

use time::OffsetDateTime;

use crate::identity::actors::Actor;
use crate::DomainError;

/// The largest JSON text of one value, in bytes.
pub const MAX_VALUE_BYTES: usize = 16 << 10;

/// The largest number of entries of one user.
pub const MAX_ENTRIES: i64 = 32;

/// The largest size of all the entries of one user, in bytes: the keys and the values.
pub const MAX_TOTAL_BYTES: i64 = 64 << 10;

/// The key of an entry: 1 to 128 lower case letters, digits, dots, colons, dashes or underscores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateKey(String);

impl StateKey {
    /// The maximum number of characters.
    pub const MAX_LEN: usize = 128;

    /// Checks the form of a key.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let allowed = |c: char| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | ':' | '-' | '_')
        };
        if (1..=Self::MAX_LEN).contains(&raw.len()) && raw.chars().all(allowed) {
            Ok(Self(raw))
        } else {
            Err(DomainError::NotAStateKey)
        }
    }

    /// Returns the key as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One entry of the state of a user, as the store returns it.
#[derive(Clone, Debug)]
pub struct UserStateEntry {
    /// The key.
    pub key: String,
    /// The value, as a JSON text.
    pub value_json: String,
    /// The version: 1 at the first write, and one more at each write.
    pub version: i64,
    /// The time of the last write.
    pub updated_at: OffsetDateTime,
}

crate::authz! {
    /// The routes of a user on its own state.
    extend Actor as UserStateCaller {
        /// The route `GetUserState`.
        permission get_user_state = nil;
        /// The route `PutUserStateEntry`.
        permission put_user_state_entry = nil;
    }
}
