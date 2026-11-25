//! The mailers of the core: SMTP with a text part, an HTML part with its images and a
//! `Message-ID`, and a log for a server with no SMTP server.

use std::sync::Arc;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::extension::ClientId;
use lettre::transport::smtp::response::Response;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use scylla_extension::{async_trait, Mail, MailError, MailImage, MailReceipt, Mailer};
use url::{Host, Url};
use uuid::Uuid;

use super::logs::mask_email;
use crate::{Config, Error};

/// Makes the mailer of the settings: SMTP with `SCYLLA_SMTP_URL`, else a log of the mails.
pub fn from_config(config: &Config) -> Result<Arc<dyn Mailer>, Error> {
    let Some(url) = &config.smtp_url else {
        return Ok(Arc::new(LogMailer));
    };
    let mailer =
        SmtpMailer::new_for_app(url, &config.mail_from, &config.app_url).map_err(|reason| {
            Error::Config {
                name: "SCYLLA_SMTP_URL",
                reason,
            }
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
        Self::with_hello(url, from, None)
    }

    /// Makes a mailer as `new` does, which gives the host of `app_url` in `EHLO`, unless the SMTP
    /// URL has a name in its path.
    pub fn new_for_app(url: &str, from: &str, app_url: &str) -> Result<Self, String> {
        let named = Url::parse(url).is_ok_and(|url| url.path().len() > 1);
        Self::with_hello(url, from, hello_name(app_url).filter(|_| !named))
    }

    /// Makes a mailer that gives this name in `EHLO`, else the name of the SMTP URL or of lettre.
    fn with_hello(url: &str, from: &str, hello: Option<ClientId>) -> Result<Self, String> {
        let mut builder = AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
            .map_err(|error| format!("is not an SMTP URL: {error}"))?;
        if let Some(name) = hello {
            builder = builder.hello_name(name);
        }
        let transport = builder.build();
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
        self.send_traced(mail).await.map(|_| ())
    }

    async fn send_traced(&self, mail: Mail) -> Result<MailReceipt, MailError> {
        let message = self.message(mail)?;
        let message_id = message.headers().get_raw("Message-ID").map(str::to_owned);
        let response = self.transport.send(message).await.map_err(|error| {
            let reason = format!("the SMTP server refused the mail: {error}");
            match error.is_permanent() {
                true => MailError::permanent(reason),
                false => MailError::transient(reason),
            }
        })?;
        Ok(MailReceipt {
            message_id,
            reply: Some(reply(&response)),
        })
    }
}

/// Returns the name that the mailer gives in `EHLO`: the host of the web app. With no host, it
/// returns `None`, and lettre gives `[127.0.0.1]`.
fn hello_name(app_url: &str) -> Option<ClientId> {
    match Url::parse(app_url).ok()?.host()? {
        Host::Domain(domain) => Some(ClientId::Domain(domain.to_owned())),
        Host::Ipv4(address) => Some(ClientId::Ipv4(address)),
        Host::Ipv6(address) => Some(ClientId::Ipv6(address)),
    }
}

/// Returns the answer of the SMTP server on one line: its code, then its text.
fn reply(response: &Response) -> String {
    let text: Vec<&str> = response.message().collect();
    format!("{} {}", response.code(), text.join(" "))
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
mod tests;
