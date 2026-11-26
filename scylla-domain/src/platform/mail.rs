//! The mail settings of the instance: the SMTP relay that sends the mails of the server, which an
//! instance admin sets at runtime in place of the default relay of the server.

use super::Platform;
use crate::identity::users::Email;
use crate::DomainError;

/// How the server makes its connection to the relay secure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailSecurity {
    /// TLS from the start of the connection (SMTPS), usually on port 465.
    Tls,
    /// A plain connection that STARTTLS makes secure, usually on port 587.
    StartTls,
    /// No TLS, for a relay on the same host or on a private network.
    None,
}

impl MailSecurity {
    /// Returns the name in the database: `tls`, `starttls` or `none`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tls => "tls",
            Self::StartTls => "starttls",
            Self::None => "none",
        }
    }

    /// Reads a name of `as_str`.
    pub fn parse(text: &str) -> Option<Self> {
        [Self::Tls, Self::StartTls, Self::None]
            .into_iter()
            .find(|security| security.as_str() == text)
    }
}

/// The relay of the instance, checked, with no password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailRelay {
    /// The host name or the IP address of the relay, in lower case.
    pub host: String,
    /// The TCP port of the relay.
    pub port: u16,
    /// The security of the connection.
    pub security: MailSecurity,
    /// The user name of the SMTP authentication. Empty: no authentication.
    pub username: String,
    /// The address of the sender of the mails.
    pub from_address: String,
    /// The name of the sender. Empty: the address only.
    pub from_name: String,
}

/// The values of a relay that an admin gives, before their check.
#[derive(Clone, Debug, Default)]
pub struct RawMailRelay {
    /// The host name or the IP address of the relay.
    pub host: String,
    /// The TCP port of the relay.
    pub port: u32,
    /// The security of the connection, if the admin gave one.
    pub security: Option<MailSecurity>,
    /// The user name of the SMTP authentication.
    pub username: String,
    /// The address of the sender of the mails.
    pub from_address: String,
    /// The name of the sender.
    pub from_name: String,
}

impl MailRelay {
    /// The most characters of a host name.
    pub const HOST_MAX: usize = 253;
    /// The most characters of a user name.
    pub const USERNAME_MAX: usize = 255;
    /// The most characters of the name of the sender.
    pub const NAME_MAX: usize = 100;
    /// The most bytes of a password.
    pub const PASSWORD_MAX: usize = 1024;

    /// Checks the values that an admin gives. A bad value gives the name of its field.
    pub fn parse(raw: RawMailRelay) -> Result<Self, (&'static str, DomainError)> {
        let host = host(&raw.host).map_err(|error| ("host", error))?;
        let port = u16::try_from(raw.port)
            .ok()
            .filter(|port| *port > 0)
            .ok_or(("port", DomainError::OutOfRange { min: 1, max: 65535 }))?;
        let security = raw.security.ok_or(("security", DomainError::Required))?;
        let username =
            line(&raw.username, Self::USERNAME_MAX).map_err(|error| ("username", error))?;
        let from_address = Email::parse(raw.from_address)
            .map_err(|error| ("from_address", error))?
            .as_str()
            .to_owned();
        let from_name =
            line(&raw.from_name, Self::NAME_MAX).map_err(|error| ("from_name", error))?;
        Ok(Self {
            host,
            port,
            security,
            username,
            from_address,
            from_name,
        })
    }

    /// Returns `true` when the relay needs a user name and a password.
    pub fn authenticates(&self) -> bool {
        !self.username.is_empty()
    }
}

/// Checks a password, or an API key that a relay takes as a password: one line of at most
/// `MailRelay::PASSWORD_MAX` bytes, kept with no trim.
pub fn password(raw: &str) -> Result<&str, DomainError> {
    if raw.len() > MailRelay::PASSWORD_MAX {
        return Err(DomainError::TooLarge {
            max: MailRelay::PASSWORD_MAX,
        });
    }
    if raw.chars().any(char::is_control) {
        return Err(DomainError::NotOneLine);
    }
    Ok(raw)
}

/// Checks a host name or an IP address: letters, digits, dots, dashes or colons, with no dot or
/// dash at its ends. It returns the host in lower case.
fn host(raw: &str) -> Result<String, DomainError> {
    let host = raw.trim().to_ascii_lowercase();
    if host.is_empty() {
        return Err(DomainError::Empty);
    }
    if host.chars().count() > MailRelay::HOST_MAX {
        return Err(DomainError::TooLong {
            max: MailRelay::HOST_MAX,
        });
    }
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':');
    let ends = [host.starts_with(['.', '-']), host.ends_with(['.', '-'])];
    if !host.chars().all(allowed) || ends.contains(&true) {
        return Err(DomainError::NotAHost);
    }
    Ok(host)
}

/// Checks a text of one line: trimmed, at most `max` characters, with no control character.
fn line(raw: &str, max: usize) -> Result<String, DomainError> {
    let trimmed = raw.trim();
    if trimmed.chars().count() > max {
        return Err(DomainError::TooLong { max });
    }
    if trimmed.chars().any(char::is_control) {
        return Err(DomainError::NotOneLine);
    }
    Ok(trimmed.to_owned())
}

crate::authz! {
    /// The routes on the mail settings of the instance.
    extend Platform as MailSettingsHolder {
        /// The route `GetMailSettings`.
        permission get_mail_settings = nil;
        /// The route `UpdateMailSettings`.
        permission update_mail_settings = nil;
        /// The route `ResetMailSettings`.
        permission reset_mail_settings = nil;
        /// The route `SendTestMail`.
        permission send_test_mail = nil;
    }
}

#[cfg(test)]
mod tests;
