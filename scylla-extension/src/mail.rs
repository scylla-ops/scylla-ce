//! The mails of the server: the check of an email, a password reset, an invitation.

use std::fmt;

use async_trait::async_trait;

/// A mail to send. Its body can hold a one-time link, so `Debug` never shows it.
#[derive(Clone)]
pub struct Mail {
    /// The address of the recipient.
    pub to: String,
    /// The subject.
    pub subject: String,
    /// The text body.
    pub body: String,
}

impl fmt::Debug for Mail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mail")
            .field("to", &self.to)
            .field("subject", &self.subject)
            .finish_non_exhaustive()
    }
}

/// Why a mail is not sent.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct MailError(pub String);

/// Sends the mails of the server. The core has an SMTP mailer; an edition or a test can give
/// another one.
#[async_trait]
pub trait Mailer: Send + Sync + 'static {
    /// Sends one mail.
    async fn send(&self, mail: Mail) -> Result<(), MailError>;
}
