mod display_name;
mod email;
mod name;
mod password;
mod password_hash;
mod password_reset;

pub use display_name::*;
pub use email::*;
pub use name::*;
pub use password::*;
pub use password_hash::*;
pub use password_reset::*;

use crate::domain::clock;
use crate::domain::errors::DomainResult;
use crate::domain::ids::UserId;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct User {
    id: UserId,
    username: Username,
    /// `None` for legacy username-only accounts.
    email: Option<Email>,
    display_name: Option<DisplayName>,
    password_hash: PasswordHash,
    is_active: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    /// The row version the value was read at. The store checks it on every write and bumps
    /// it itself; the domain never changes it.
    version: u64,
}

impl User {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_persistence(
        id: UserId,
        username: Username,
        email: Option<Email>,
        display_name: Option<DisplayName>,
        password_hash: PasswordHash,
        is_active: bool,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
        version: u64,
    ) -> Self {
        Self {
            id,
            username,
            email,
            display_name,
            password_hash,
            is_active,
            created_at,
            updated_at,
            version,
        }
    }

    #[must_use]
    pub fn create(username: Username, email: Option<Email>, password_hash: PasswordHash) -> Self {
        let now = clock::now();
        Self {
            id: UserId::generate(),
            username,
            email,
            display_name: None,
            password_hash,
            is_active: true,
            created_at: now,
            updated_at: now,
            version: 0,
        }
    }

    pub fn update_username(&mut self, username: Username) -> DomainResult<()> {
        self.username = username;
        self.updated_at = clock::now();
        Ok(())
    }

    pub fn update_email(&mut self, email: Email) {
        self.email = Some(email);
        self.updated_at = clock::now();
    }

    /// For a new account: `updated_at` stays equal to `created_at`.
    #[must_use]
    pub fn with_display_name(mut self, display_name: Option<DisplayName>) -> Self {
        self.display_name = display_name;
        self
    }

    pub fn set_display_name(&mut self, display_name: Option<DisplayName>) {
        self.display_name = display_name;
        self.updated_at = clock::now();
    }

    pub fn set_password_hash(&mut self, password_hash: PasswordHash) {
        self.password_hash = password_hash;
        self.updated_at = clock::now();
    }

    /// Returns `false` and changes nothing when the flag already has this value.
    pub fn set_active(&mut self, is_active: bool) -> bool {
        if self.is_active == is_active {
            return false;
        }
        self.is_active = is_active;
        self.updated_at = clock::now();
        true
    }

    #[must_use]
    pub fn id(&self) -> &UserId {
        &self.id
    }

    #[must_use]
    pub fn username(&self) -> &Username {
        &self.username
    }

    #[must_use]
    pub fn email(&self) -> Option<&Email> {
        self.email.as_ref()
    }

    #[must_use]
    pub fn display_name(&self) -> Option<&DisplayName> {
        self.display_name.as_ref()
    }

    #[must_use]
    pub fn password_hash(&self) -> &PasswordHash {
        &self.password_hash
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.is_active
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn updated_at(&self) -> DateTime<Utc> {
        self.updated_at
    }

    #[must_use]
    pub fn version(&self) -> u64 {
        self.version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user() -> User {
        User::create(
            Username::new("ada").unwrap(),
            None,
            PasswordHash::new("$argon2id$v=19$m=19456,t=2,p=1$abc$def").unwrap(),
        )
    }

    #[test]
    fn a_new_account_with_a_display_name_is_not_changed_yet() {
        let user = user().with_display_name(Some(DisplayName::new("Ada").unwrap()));
        assert_eq!(user.display_name().map(DisplayName::as_str), Some("Ada"));
        assert_eq!(user.updated_at(), user.created_at());
    }

    #[test]
    fn setting_the_current_active_flag_changes_nothing() {
        let mut user = user();
        let before = user.updated_at();
        assert!(!user.set_active(true));
        assert_eq!(user.updated_at(), before);
        assert!(user.set_active(false));
        assert!(!user.is_active());
    }
}
