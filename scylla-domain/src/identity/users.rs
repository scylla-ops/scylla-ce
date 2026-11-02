//! The users: the persons. A user signs in with a password or with an identity provider, and
//! each sign-in opens a session.

use std::fmt;

use time::OffsetDateTime;

use crate::identity::actors::{Actor, ActorId, ActorKind};
use crate::Name;
use crate::{DomainError, Id};

/// The actor kind of a user.
pub const KIND: ActorKind = ActorKind::new("user");

/// The marker of a session id.
pub enum Session {}

/// The id of a session: one sign-in on one device.
pub type SessionId = Id<Session>;

/// The marker of an identity provider id.
pub enum Provider {}

/// The id of an identity provider.
pub type ProviderId = Id<Provider>;

/// An email address: trimmed, lower case, with one `@` and text on each side.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Email(String);

impl Email {
    /// The maximum number of characters.
    pub const MAX_LEN: usize = 254;

    /// Trims the text, makes it lower case, and checks its form.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let email = raw.trim().to_lowercase();
        if email.chars().count() > Self::MAX_LEN {
            return Err(DomainError::TooLong { max: Self::MAX_LEN });
        }
        let valid = match email.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty()
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
                    && !domain.contains('@')
            }
            None => false,
        };
        if !valid || email.chars().any(char::is_whitespace) {
            return Err(DomainError::NotAnEmail);
        }
        Ok(Self(email))
    }

    /// Returns the address as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A password: `MIN_LEN` to `MAX_LEN` characters, with no other rule. Its text never goes to a
/// log.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(String);

impl Password {
    /// The minimum number of characters, for a password that is the only factor.
    pub const MIN_LEN: usize = 15;
    /// The maximum number of characters.
    pub const MAX_LEN: usize = 128;

    /// Checks the length. The text is kept as it is: spaces count.
    pub fn parse(raw: String) -> Result<Self, DomainError> {
        let length = raw.chars().count();
        if length < Self::MIN_LEN {
            return Err(DomainError::TooShort { min: Self::MIN_LEN });
        }
        if length > Self::MAX_LEN {
            return Err(DomainError::TooLong { max: Self::MAX_LEN });
        }
        Ok(Self(raw))
    }

    /// Returns the text, for the hash only.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Password {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Password(***)")
    }
}

/// The state of a user account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UserStatus {
    /// The email is not verified yet: the user cannot sign in.
    Pending,
    /// The user can sign in.
    Active,
    /// An operator stopped the account: each token and session of the user stops.
    Disabled,
}

impl UserStatus {
    /// Returns the name in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Disabled => "disabled",
        }
    }

    /// Reads a name of the database. An unknown name gives `Disabled`, so it fails closed.
    pub fn from_name(name: &str) -> Self {
        match name {
            "pending" => Self::Pending,
            "active" => Self::Active,
            _ => Self::Disabled,
        }
    }
}

/// A user, as the store returns it.
#[derive(Clone, Debug)]
pub struct User {
    /// The id of the user, an actor id.
    pub id: ActorId,
    /// The display name.
    pub display_name: String,
    /// The primary email, if the user has one.
    pub email: Option<String>,
    /// The state of the account.
    pub status: UserStatus,
    /// The creation time, set by the database.
    pub created_at: OffsetDateTime,
}

/// The input of a user creation.
#[derive(Clone, Debug)]
pub struct NewUser {
    /// The primary email.
    pub email: Email,
    /// `true` if a link or a provider proved the email already.
    pub email_verified: bool,
    /// The display name.
    pub display_name: Name,
    /// The password, for a sign-in with a password.
    pub password: Option<Password>,
}

crate::authz! {
    /// The routes of the account on the caller itself.
    extend Actor as AccountCaller {
        /// The route `GetMe`.
        permission get_me = nil;
        /// The route `ChangePassword`.
        permission change_password = nil;
        /// The route `ListIdentities`.
        permission list_identities = nil;
        /// The route `UnlinkIdentity`.
        permission unlink_identity = nil;
        /// The route `ListSessions`.
        permission list_sessions = nil;
        /// The route `RevokeSession`.
        permission revoke_session = nil;
    }
}
