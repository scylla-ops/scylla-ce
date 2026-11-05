//! A mailer that keeps the mails, with their text and HTML bodies, so a test reads their links.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use scylla_extension::{async_trait, Mail, MailError, Mailer};

/// The longest time that `wait` waits for a mail.
const WAIT: Duration = Duration::from_secs(5);

/// The mails that the server of a test sent, oldest first.
#[derive(Default)]
pub struct Mailbox {
    /// The mails.
    mails: Mutex<Vec<Mail>>,
    /// `true` if each send fails.
    failing: AtomicBool,
}

impl Mailbox {
    /// Makes each next send fail, as an SMTP server that is down does.
    pub fn fail_sends(&self) {
        self.failing.store(true, Ordering::SeqCst);
    }

    /// Returns the mails to an address, oldest first.
    pub fn to(&self, address: &str) -> Vec<Mail> {
        let mails = self.mails.lock().expect("mailbox");
        mails
            .iter()
            .filter(|mail| mail.to == address)
            .cloned()
            .collect()
    }

    /// Waits until an address has `count` mails or more, for a mail that the server sends after
    /// its answer, and returns them.
    ///
    /// # Panics
    ///
    /// If the mails do not come in 5 seconds.
    pub async fn wait(&self, address: &str, count: usize) -> Vec<Mail> {
        let start = tokio::time::Instant::now();
        loop {
            let mails = self.to(address);
            if mails.len() >= count {
                return mails;
            }
            assert!(
                start.elapsed() < WAIT,
                "{address} got {} mails, not {count}",
                mails.len()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Returns the token of the last link to a page of the web app in a mail to an address. The
    /// token is in the fragment of the link: `/<page>#token=<token>`.
    ///
    /// # Panics
    ///
    /// If no mail to the address has such a link.
    pub fn token(&self, address: &str, page: &str) -> String {
        let marker = format!("/{page}#token=");
        self.to(address)
            .iter()
            .rev()
            .find_map(|mail| {
                let start = mail.body.find(&marker)? + marker.len();
                let token = mail.body[start..].split_whitespace().next()?;
                Some(token.to_owned())
            })
            .unwrap_or_else(|| panic!("no mail to {address} with a link to {page}"))
    }
}

#[async_trait]
impl Mailer for Mailbox {
    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(MailError("the test mail server is down".to_owned()));
        }
        self.mails.lock().expect("mailbox").push(mail);
        Ok(())
    }
}
