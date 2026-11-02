//! The mailers of the core: SMTP, and a log for a server with no SMTP server.

use lettre::message::header::ContentType;
use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use scylla_extension::{async_trait, Mail, MailError, Mailer};

/// Sends the mails to an SMTP server.
pub struct SmtpMailer {
    /// The connection to the server.
    transport: AsyncSmtpTransport<Tokio1Executor>,
    /// The sender of the mails.
    from: Mailbox,
}

impl SmtpMailer {
    /// Makes a mailer for `smtp://host:port`, or `smtps://user:password@host` for TLS.
    pub fn new(url: &str, from: &str) -> Result<Self, String> {
        let transport = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .map_err(|error| format!("is not an SMTP URL: {error}"))?
            .build();
        let from = from
            .parse()
            .map_err(|error| format!("the sender is not an address: {error}"))?;
        Ok(Self { transport, from })
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        let to: Mailbox = mail
            .to
            .parse()
            .map_err(|error| MailError(format!("the recipient is not an address: {error}")))?;
        let message = Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(mail.subject)
            .header(ContentType::TEXT_PLAIN)
            .body(mail.body)
            .map_err(|error| MailError(format!("the mail is not valid: {error}")))?;
        self.transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(|error| MailError(format!("the SMTP server refused the mail: {error}")))
    }
}

/// Sends no mail: it logs the recipient and the subject, never the body with its link.
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        tracing::warn!(to = %mail.to, subject = %mail.subject, "no SMTP server: the mail is not sent");
        Ok(())
    }
}
