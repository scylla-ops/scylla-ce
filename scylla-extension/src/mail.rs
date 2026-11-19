//! The mails of the server: the check of an email, a password reset, an invitation, a change of
//! email.

use std::fmt;

use async_trait::async_trait;

/// A mail to send, with a text body and an HTML body of the same content. A body can hold a
/// one-time link, so `Debug` never shows it.
#[derive(Clone)]
pub struct Mail {
    /// The address of the recipient.
    pub to: String,
    /// The subject.
    pub subject: String,
    /// The text body.
    pub body: String,
    /// The HTML body, with each value escaped.
    pub html: String,
    /// The images that the HTML body shows with `cid:<id>`, sent in the mail.
    pub images: Vec<MailImage>,
}

/// An image of an HTML body, sent in the mail, so a mail client shows it with no download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailImage {
    /// The content id: the HTML body shows the image with `cid:<id>`.
    pub id: String,
    /// The media type, for example `image/png`.
    pub content_type: String,
    /// The bytes of the image.
    pub bytes: Vec<u8>,
}

impl fmt::Debug for Mail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Mail")
            .field("to", &self.to)
            .field("subject", &self.subject)
            .field("images", &self.images.len())
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
    /// Sends one mail, with its text body and its HTML body as two alternatives.
    async fn send(&self, mail: Mail) -> Result<(), MailError>;
}
