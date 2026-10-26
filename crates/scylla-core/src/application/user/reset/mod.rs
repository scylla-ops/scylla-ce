pub mod sender;

pub use sender::{PasswordResetDelivery, PasswordResetMessage, PasswordResetSender};

use super::AccountRepository;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::user::{Email, PasswordReset, ResetToken, User};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Duration;
use std::sync::Arc;
use tracing::warn;

pub const RESET_LINK_TTL: Duration = Duration::hours(1);
/// A request for an account that got a link within this time delivers nothing.
pub const RESET_COOLDOWN: Duration = Duration::seconds(60);
pub const RESET_PATH: &str = "/reset-password";

/// `<public_url>/reset-password#token=<token>`. The fragment does not go to the server in a
/// request. Without a public URL the link is relative.
#[derive(Debug, Clone, Default)]
pub struct ResetLinks {
    base: String,
}

impl ResetLinks {
    #[must_use]
    pub fn new(public_url: Option<&str>) -> Self {
        Self {
            base: public_url
                .map(|url| url.trim().trim_end_matches('/').to_owned())
                .unwrap_or_default(),
        }
    }

    #[must_use]
    pub fn link(&self, token: &ResetToken) -> String {
        format!("{}{RESET_PATH}#token={}", self.base, token.as_str())
    }
}

/// Here and not in the kernel: the random source is a security decision.
pub fn mint_reset_token() -> DomainResult<ResetToken> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| DomainError::internal(format!("no random bytes for a reset token: {e}")))?;
    ResetToken::new(URL_SAFE_NO_PAD.encode(bytes))
}

/// A link to store and its message.
#[derive(Debug, Clone)]
pub struct NewReset {
    pub reset: PasswordReset,
    pub message: PasswordResetMessage,
}

/// The reset links: `request_password_reset` and `reset_password` for a caller without a session,
/// `send_password_reset` for an administrator or the user.
pub struct PasswordResetUseCases {
    accounts: Arc<dyn AccountRepository>,
    sender: Arc<dyn PasswordResetSender>,
    links: ResetLinks,
}

impl PasswordResetUseCases {
    pub fn new(
        accounts: Arc<dyn AccountRepository>,
        sender: Arc<dyn PasswordResetSender>,
        links: ResetLinks,
    ) -> Self {
        Self {
            accounts,
            sender,
            links,
        }
    }

    pub fn new_reset(&self, user: &User, email: &Email) -> DomainResult<NewReset> {
        let reset = PasswordReset::create(user.id().clone(), mint_reset_token()?, RESET_LINK_TTL);
        let message = PasswordResetMessage {
            user_id: user.id().clone(),
            email: email.clone(),
            username: user.username().clone(),
            display_name: user.display_name().cloned(),
            link: self.links.link(reset.token()),
            expires_at: reset.expires_at(),
        };
        Ok(NewReset { reset, message })
    }

    /// After the response, so that its time and its errors do not depend on the account.
    pub fn issue_later(&self, new: NewReset) {
        let accounts = self.accounts.clone();
        let sender = self.sender.clone();
        tokio::spawn(async move {
            let user_id = new.reset.user_id().clone();
            if let Err(e) = issue_and_deliver(&*accounts, &*sender, new).await {
                warn!(user_id = %user_id, error = %e, "a reset request failed after its answer");
            }
        });
    }
}

/// The work of `request_password_reset` after its answer. It stores the link with the rule of
/// `RESET_COOLDOWN`, which also cancels the earlier links, then delivers it. `false` when the rule
/// stored and delivered nothing.
pub async fn issue_and_deliver(
    accounts: &dyn AccountRepository,
    sender: &dyn PasswordResetSender,
    new: NewReset,
) -> DomainResult<bool> {
    if !accounts
        .issue_reset(&new.reset, Some(RESET_COOLDOWN))
        .await?
    {
        return Ok(false);
    }
    sender.send(&new.message).await?;
    Ok(true)
}

#[cfg(test)]
mod tests;
