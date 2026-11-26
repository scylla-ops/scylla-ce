//! The mailer of the instance: the relay of the mail settings when an instance admin set one, else
//! the default mailer of the server. A change of the settings applies to the next send.

use std::sync::{Arc, PoisonError, RwLock};

use scylla_extension::{async_trait, Mail, MailError, MailReceipt, Mailer};

/// The relay of the mail settings, ready to send.
struct Relay {
    /// The mailer of the relay.
    mailer: Arc<dyn Mailer>,
    /// The address of the sender of the relay.
    sender: String,
}

/// Sends each mail with the relay of the mail settings, else with the default mailer of the
/// server.
pub struct InstanceMailer {
    /// The mailer of the settings of the server: `SCYLLA_SMTP_URL`, a log, or a test mailer.
    default: Arc<dyn Mailer>,
    /// The relay of the mail settings, if an instance admin set one.
    relay: RwLock<Option<Relay>>,
}

impl InstanceMailer {
    /// Makes the mailer of an instance with no relay of the mail settings.
    pub fn new(default: Arc<dyn Mailer>) -> Self {
        Self {
            default,
            relay: RwLock::new(None),
        }
    }

    /// Sends the next mails with this mailer, from this sender address.
    pub fn set(&self, mailer: Arc<dyn Mailer>, sender: String) {
        let mut relay = self.relay.write().unwrap_or_else(PoisonError::into_inner);
        *relay = Some(Relay { mailer, sender });
    }

    /// Sends the next mails with the default mailer of the server.
    pub fn reset(&self) {
        let mut relay = self.relay.write().unwrap_or_else(PoisonError::into_inner);
        *relay = None;
    }

    /// Returns the address of the sender of the relay of the mail settings, if one is set.
    pub fn sender(&self) -> Option<String> {
        let relay = self.relay.read().unwrap_or_else(PoisonError::into_inner);
        relay.as_ref().map(|relay| relay.sender.clone())
    }

    /// Returns the host of the default mailer of the server, if it has a mail server.
    pub fn default_relay(&self) -> Option<String> {
        self.default.relay()
    }

    /// Returns the mailer of the next send.
    fn current(&self) -> Arc<dyn Mailer> {
        let relay = self.relay.read().unwrap_or_else(PoisonError::into_inner);
        relay.as_ref().map_or_else(
            || Arc::clone(&self.default),
            |relay| Arc::clone(&relay.mailer),
        )
    }
}

#[async_trait]
impl Mailer for InstanceMailer {
    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        self.current().send(mail).await
    }

    async fn send_traced(&self, mail: Mail) -> Result<MailReceipt, MailError> {
        self.current().send_traced(mail).await
    }

    fn delivers(&self) -> bool {
        self.current().delivers()
    }

    fn relay(&self) -> Option<String> {
        self.current().relay()
    }
}

/// A relay of the mail settings that the server cannot use, for example with a password that the
/// key of the server does not open. Each send is a transient error, so each mail waits in the
/// outbox until an admin changes the settings, for at most 24 hours.
pub struct BrokenRelay {
    /// The host of the relay.
    pub host: String,
    /// Why the server cannot use the relay, with no secret.
    pub reason: String,
}

#[async_trait]
impl Mailer for BrokenRelay {
    async fn send(&self, _mail: Mail) -> Result<(), MailError> {
        Err(MailError::transient(self.reason.clone()))
    }

    fn relay(&self) -> Option<String> {
        Some(self.host.clone())
    }
}
