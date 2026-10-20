use bon::bon;
use chrono::{DateTime, Utc};

use crate::domain::clock;
use crate::domain::ids::UserId;
use crate::domain::user::User;
use crate::domain::user::{DisplayName, Email, PasswordHash, Username};

pub const VALID_ARGON2_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$abc$xyz";

pub struct UserBuilder;

#[bon]
#[allow(clippy::new_ret_no_self, clippy::must_use_candidate)]
impl UserBuilder {
    #[builder(start_fn = new, finish_fn = build)]
    pub fn assemble(
        #[builder(start_fn, into)] username: String,
        id: Option<UserId>,
        #[builder(into)] email: Option<String>,
        #[builder(into)] display_name: Option<String>,
        #[builder(into, default = VALID_ARGON2_HASH.to_string())] password_hash: String,
        #[builder(default = true)] is_active: bool,
        created_at: Option<DateTime<Utc>>,
        updated_at: Option<DateTime<Utc>>,
        #[builder(default = 0)] version: u64,
    ) -> User {
        let now = created_at.unwrap_or_else(clock::now);
        let email = email.map(|e| Email::new(e).expect("test email invalid"));
        let display_name =
            display_name.map(|d| DisplayName::new(d).expect("test display name invalid"));
        User::from_persistence(
            id.unwrap_or_else(UserId::generate),
            Username::new(username).expect("test username invalid"),
            email,
            display_name,
            PasswordHash::new(password_hash).expect("test password hash invalid"),
            is_active,
            now,
            updated_at.unwrap_or(now),
            version,
        )
    }
}

#[must_use]
pub fn user(name: &str) -> User {
    UserBuilder::new(name).build()
}
