//! The time zones: the names of the IANA database.

use std::str::FromStr;

use crate::DomainError;

/// A zone of the IANA database, for example `Europe/Paris`. `chrono-tz` holds the database.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeZone(chrono_tz::Tz);

impl TimeZone {
    /// The zone of a new workspace.
    pub const UTC: Self = Self(chrono_tz::Tz::UTC);

    /// Trims the text, and checks that it is the name of a zone, in the case of the database.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        chrono_tz::Tz::from_str(raw.trim())
            .map(Self)
            .map_err(|_| DomainError::NotATimeZone)
    }

    /// Returns the name of the zone.
    pub fn as_str(&self) -> &'static str {
        self.0.name()
    }
}

impl Default for TimeZone {
    fn default() -> Self {
        Self::UTC
    }
}
