use crate::domain::clock;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{PasswordResetId, UserId};
use chrono::{DateTime, Duration, Utc};
use std::fmt;

/// One message for an unknown, used or expired link and for an inactive account, so the answer
/// does not tell which.
pub const RESET_LINK_INVALID: &str = "This reset link is not valid";

/// 32 random bytes in base64url without padding.
pub const RESET_TOKEN_LEN: usize = 43;

#[must_use]
pub fn reset_link_invalid() -> DomainError {
    DomainError::business_rule(RESET_LINK_INVALID)
}

/// The token of a reset link. A value of another shape is no token of this server, so it is the
/// error of an unknown link, not a validation error.
#[derive(Clone, PartialEq, Eq)]
pub struct ResetToken(String);

impl ResetToken {
    pub fn new(value: impl Into<String>) -> DomainResult<Self> {
        let value = value.into();
        let shaped = value.len() == RESET_TOKEN_LEN
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if shaped {
            Ok(Self(value))
        } else {
            Err(reset_link_invalid())
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ResetToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// A reset link of a user. The store keeps the SHA-256 of the token, never the token.
#[derive(Debug, Clone)]
pub struct PasswordReset {
    id: PasswordResetId,
    token: ResetToken,
    user_id: UserId,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    used_at: Option<DateTime<Utc>>,
}

impl PasswordReset {
    #[must_use]
    pub fn create(user_id: UserId, token: ResetToken, ttl: Duration) -> Self {
        let now = clock::now();
        Self {
            id: PasswordResetId::generate(),
            token,
            user_id,
            created_at: now,
            expires_at: now + ttl,
            used_at: None,
        }
    }

    #[must_use]
    pub fn from_persistence(
        id: PasswordResetId,
        token: ResetToken,
        user_id: UserId,
        created_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        used_at: Option<DateTime<Utc>>,
    ) -> Self {
        Self {
            id,
            token,
            user_id,
            created_at,
            expires_at,
            used_at,
        }
    }

    pub fn ensure_redeemable(&self) -> DomainResult<()> {
        if self.used_at.is_some() || clock::now() >= self.expires_at {
            return Err(reset_link_invalid());
        }
        Ok(())
    }

    #[must_use]
    pub fn id(&self) -> &PasswordResetId {
        &self.id
    }

    #[must_use]
    pub fn token(&self) -> &ResetToken {
        &self.token
    }

    #[must_use]
    pub fn user_id(&self) -> &UserId {
        &self.user_id
    }

    #[must_use]
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created_at
    }

    #[must_use]
    pub fn expires_at(&self) -> DateTime<Utc> {
        self.expires_at
    }

    #[must_use]
    pub fn used_at(&self) -> Option<DateTime<Utc>> {
        self.used_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_Z";

    fn token() -> ResetToken {
        ResetToken::new(TOKEN).unwrap()
    }

    #[test]
    fn a_token_of_another_shape_is_an_invalid_link() {
        for bad in ["", "short", &format!("{TOKEN}a"), &TOKEN.replace('Z', "=")] {
            let err = ResetToken::new(bad).unwrap_err();
            assert!(
                matches!(&err, DomainError::BusinessRule(m) if m == RESET_LINK_INVALID),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn a_token_never_prints() {
        assert_eq!(format!("{:?}", token()), "[REDACTED]");
        let reset = PasswordReset::create(UserId::new("u"), token(), Duration::hours(1));
        assert!(!format!("{reset:?}").contains(TOKEN));
    }

    #[test]
    fn a_fresh_link_is_redeemable_until_it_expires_or_is_used() {
        let now = clock::now();
        let fresh = PasswordReset::create(UserId::new("u"), token(), Duration::hours(1));
        assert!(fresh.ensure_redeemable().is_ok());
        assert_eq!(fresh.expires_at() - fresh.created_at(), Duration::hours(1));

        let expired = PasswordReset::from_persistence(
            PasswordResetId::generate(),
            token(),
            UserId::new("u"),
            now - Duration::hours(2),
            now - Duration::hours(1),
            None,
        );
        let used = PasswordReset::from_persistence(
            PasswordResetId::generate(),
            token(),
            UserId::new("u"),
            now,
            now + Duration::hours(1),
            Some(now),
        );
        for reset in [expired, used] {
            assert!(matches!(
                reset.ensure_redeemable(),
                Err(DomainError::BusinessRule(m)) if m == RESET_LINK_INVALID
            ));
        }
    }
}
