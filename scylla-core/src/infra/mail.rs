//! The mailers of the core: SMTP with a text part, an HTML part with its images and a
//! `Message-ID`, and a log for a server with no SMTP server.

use std::sync::Arc;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use scylla_extension::{async_trait, Mail, MailError, MailImage, Mailer};
use uuid::Uuid;

use super::logs::mask_email;
use crate::{Config, Error};

/// Makes the mailer of the settings: SMTP with `SCYLLA_SMTP_URL`, else a log of the mails.
pub fn from_config(config: &Config) -> Result<Arc<dyn Mailer>, Error> {
    let Some(url) = &config.smtp_url else {
        return Ok(Arc::new(LogMailer));
    };
    let mailer = SmtpMailer::new(url, &config.mail_from).map_err(|reason| Error::Config {
        name: "SCYLLA_SMTP_URL",
        reason,
    })?;
    Ok(Arc::new(mailer))
}

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

    /// Makes the message of a mail, with a new `Message-ID` on the domain of the sender:
    /// `<uuid@domain>`.
    fn message(&self, mail: Mail) -> Result<Message, MailError> {
        let to: Mailbox = mail.to.parse().map_err(|error| {
            MailError::permanent(format!("the recipient is not an address: {error}"))
        })?;
        let html = html_part(mail.html, mail.images)?;
        let id = format!("<{}@{}>", Uuid::now_v7(), self.from.email.domain());
        Message::builder()
            .from(self.from.clone())
            .to(to)
            .subject(mail.subject)
            .message_id(Some(id))
            .multipart(
                MultiPart::alternative()
                    .singlepart(SinglePart::plain(mail.body))
                    .multipart(html),
            )
            .map_err(|error| MailError::permanent(format!("the mail is not valid: {error}")))
    }
}

#[async_trait]
impl Mailer for SmtpMailer {
    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        let message = self.message(mail)?;
        self.transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(|error| {
                let reason = format!("the SMTP server refused the mail: {error}");
                match error.is_permanent() {
                    true => MailError::permanent(reason),
                    false => MailError::transient(reason),
                }
            })
    }
}

/// Makes the HTML part with its images: `multipart/related`, so a client shows each image of a
/// `cid:` address with no download.
fn html_part(html: String, images: Vec<MailImage>) -> Result<MultiPart, MailError> {
    let mut related = MultiPart::related().singlepart(SinglePart::html(html));
    for image in images {
        let kind = ContentType::parse(&image.content_type)
            .map_err(|error| MailError::permanent(format!("the image is not valid: {error}")))?;
        related = related.singlepart(Attachment::new_inline(image.id).body(image.bytes, kind));
    }
    Ok(related)
}

/// Sends no mail: it logs the masked recipient and the subject, never the body with its link.
pub struct LogMailer;

#[async_trait]
impl Mailer for LogMailer {
    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        let to = mask_email(&mail.to);
        tracing::warn!(%to, subject = %mail.subject, "no SMTP server: the mail is not sent");
        Ok(())
    }

    fn delivers(&self) -> bool {
        false
    }
}

/// Returns the email address of a sender of the settings: the address of `Name <address>`, or the
/// text with no change when it is not an address.
pub fn address_of(from: &str) -> String {
    from.parse::<Mailbox>().map_or_else(
        |_| from.trim().to_owned(),
        |sender| sender.email.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use scylla_extension::Mail;

    use super::{address_of, SmtpMailer};

    /// Makes a mail to one address, with no image.
    fn mail() -> Mail {
        Mail {
            to: "ada@example.com".to_owned(),
            subject: "Verify your email".to_owned(),
            body: "Open the link.".to_owned(),
            html: "<p>Open the link.</p>".to_owned(),
            images: Vec::new(),
        }
    }

    /// Each mail has its own `Message-ID` on the domain of the sender, so a spam filter does
    /// not count a missing one.
    #[test]
    fn each_mail_has_a_message_id_on_the_domain_of_the_sender() {
        // given
        let from = "Scylla <no-reply@mail.scylla.example>";
        let mailer = SmtpMailer::new("smtp://127.0.0.1:51025", from).unwrap();

        // when
        let first = mailer.message(mail()).unwrap();
        let second = mailer.message(mail()).unwrap();

        // then
        let first = first.headers().get_raw("Message-ID").unwrap().to_owned();
        let second = second.headers().get_raw("Message-ID").unwrap();
        assert!(first.starts_with('<'), "{first}");
        assert!(first.ends_with("@mail.scylla.example>"), "{first}");
        assert_ne!(first, second);
    }

    /// A recipient that is not an address is a permanent error: a later send cannot work.
    #[test]
    fn a_bad_recipient_is_a_permanent_error() {
        // given
        let mailer = SmtpMailer::new("smtp://127.0.0.1:51025", "no-reply@scylla.example").unwrap();
        let bad = Mail {
            to: "not an address".to_owned(),
            ..mail()
        };

        // when
        let error = mailer.message(bad).unwrap_err();

        // then
        assert!(error.permanent, "{error}");
    }

    /// The address of a sender has no name, and a text that is not an address stays as it is.
    #[test]
    fn the_address_of_a_sender_has_no_name() {
        // given
        let senders = [
            "Scylla <no-reply@scylla.example>",
            "no-reply@scylla.example",
            " not an address ",
        ];

        // when
        let addresses = senders.map(address_of);

        // then
        assert_eq!(
            addresses,
            [
                "no-reply@scylla.example",
                "no-reply@scylla.example",
                "not an address"
            ]
        );
    }
}
