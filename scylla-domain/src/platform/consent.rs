//! The consent of a browser to the trackers that need consent: the choice of each purpose, and
//! the record that proves a choice.

use std::collections::HashSet;

use crate::identity::actors::Actor;
use crate::{DomainError, Id};

/// The marker of a consent id.
pub enum Consent {}

/// The id of the consent of one browser: the records of its choices share it.
pub type ConsentId = Id<Consent>;

/// The marker of the id of a record.
pub enum ConsentRecord {}

/// The id of the record of one choice.
pub type ConsentRecordId = Id<ConsentRecord>;

/// What the person did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentAction {
    /// The person accepted each purpose.
    AcceptAll,
    /// The person refused each purpose.
    RejectAll,
    /// The person accepted some purposes and refused the others.
    Custom,
    /// The person refused a purpose that the browser had accepted before.
    Withdraw,
}

impl ConsentAction {
    /// Returns the name of the action in the database.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AcceptAll => "accept_all",
            Self::RejectAll => "reject_all",
            Self::Custom => "custom",
            Self::Withdraw => "withdraw",
        }
    }
}

/// Where the person made the choice.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConsentSource {
    /// The banner at the bottom of the page.
    Banner,
    /// The cookie settings, from the banner, the footer, the user menu or the cookie page.
    Preferences,
    /// The cookie settings, from the privacy section of the account.
    Account,
}

impl ConsentSource {
    /// Returns the name of the source in the database.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Banner => "banner",
            Self::Preferences => "preferences",
            Self::Account => "account",
        }
    }
}

/// The name of a purpose: 1 to 32 lower case letters, digits and dashes, for example `analytics`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Purpose(String);

impl Purpose {
    /// The most characters of a name.
    pub const MAX_LEN: usize = 32;

    /// Checks the form of a name.
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        if raw.is_empty() || raw.len() > Self::MAX_LEN || !raw.chars().all(allowed) {
            return Err(DomainError::NotAPurpose);
        }
        Ok(Self(raw.to_owned()))
    }

    /// Returns the name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The choice of one purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    /// The purpose.
    pub purpose: Purpose,
    /// `true` when the person accepted the purpose.
    pub granted: bool,
}

/// A choice of a browser, checked, before its record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewConsentRecord {
    /// The consent of the browser, or `None` for its first choice.
    pub consent: Option<ConsentId>,
    /// The version of the banner.
    pub banner_version: String,
    /// The SHA-256 of the text that the person saw, in lower case hexadecimal.
    pub banner_hash: String,
    /// One choice for each purpose of the banner.
    pub choices: Vec<Choice>,
    /// What the person did.
    pub action: ConsentAction,
    /// Where the person made the choice.
    pub source: ConsentSource,
}

impl NewConsentRecord {
    /// The most characters of a version.
    pub const VERSION_MAX: usize = 64;
    /// The most choices of a record.
    pub const CHOICES_MAX: usize = 16;
    /// The characters of a SHA-256 in hexadecimal.
    const HASH_LEN: usize = 64;

    /// Checks the values of a record. Returns the field and the error of a bad value.
    pub fn new(
        consent: Option<ConsentId>,
        banner_version: &str,
        banner_hash: &str,
        choices: Vec<Choice>,
        action: ConsentAction,
        source: ConsentSource,
    ) -> Result<Self, (&'static str, DomainError)> {
        Ok(Self {
            consent,
            banner_version: version(banner_version).map_err(|error| ("banner_version", error))?,
            banner_hash: hash(banner_hash).map_err(|error| ("banner_hash", error))?,
            choices: checked(choices, action).map_err(|error| ("choices", error))?,
            action,
            source,
        })
    }
}

/// Checks a version: one line of 1 to `VERSION_MAX` characters, with no control character.
fn version(raw: &str) -> Result<String, DomainError> {
    let max = NewConsentRecord::VERSION_MAX;
    if raw.trim().is_empty() {
        return Err(DomainError::Empty);
    }
    if raw.chars().count() > max {
        return Err(DomainError::TooLong { max });
    }
    if raw.chars().any(char::is_control) {
        return Err(DomainError::NotOneLine);
    }
    Ok(raw.to_owned())
}

/// Checks a SHA-256: 64 lower case hexadecimal characters.
fn hash(raw: &str) -> Result<String, DomainError> {
    let hex = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
    if raw.len() == NewConsentRecord::HASH_LEN && raw.chars().all(hex) {
        Ok(raw.to_owned())
    } else {
        Err(DomainError::NotASha256)
    }
}

/// Checks the choices: 1 to `CHOICES_MAX`, each purpose once, and each one accepted for
/// `AcceptAll` and refused for `RejectAll`.
fn checked(choices: Vec<Choice>, action: ConsentAction) -> Result<Vec<Choice>, DomainError> {
    let max = NewConsentRecord::CHOICES_MAX;
    if choices.is_empty() {
        return Err(DomainError::Empty);
    }
    if choices.len() > max {
        return Err(DomainError::TooMany { max });
    }
    let mut seen = HashSet::new();
    if !choices.iter().all(|choice| seen.insert(&choice.purpose)) {
        return Err(DomainError::Duplicate);
    }
    let fits = match action {
        ConsentAction::AcceptAll => choices.iter().all(|choice| choice.granted),
        ConsentAction::RejectAll => choices.iter().all(|choice| !choice.granted),
        ConsentAction::Custom | ConsentAction::Withdraw => true,
    };
    if fits {
        Ok(choices)
    } else {
        Err(DomainError::NotTheAction)
    }
}

crate::authz! {
    /// The route of the consent on the caller itself. It needs no token: with a token, the record
    /// names the user.
    extend Actor as ConsentCaller {
        /// The route `RecordConsent`.
        permission record_consent = nil;
    }
}

#[cfg(test)]
mod tests;
