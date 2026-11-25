//! A mailer that keeps the mails, with their text and HTML bodies, so a test reads their links.
//! The server sends each mail from its outbox: a read first waits for the mails of the outbox.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use scylla_extension::{async_trait, Mail, MailError, Mailer};
use sqlx::PgPool;

/// The longest time that `wait` waits for a mail.
const WAIT: Duration = Duration::from_secs(5);

/// The mails that the server of a test sent, oldest first.
pub struct Mailbox {
    /// The database of the server, with its outbox.
    pool: PgPool,
    /// The mails.
    mails: Mutex<Vec<Mail>>,
    /// `true` if each send fails with a transient error.
    failing: AtomicBool,
    /// The number of next sends that fail with a transient error.
    failing_next: Mutex<usize>,
    /// `true` if each send fails with a permanent error.
    refusing: AtomicBool,
}

impl Mailbox {
    /// Makes an empty mailbox for the server of this database.
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            mails: Mutex::default(),
            failing: AtomicBool::default(),
            failing_next: Mutex::default(),
            refusing: AtomicBool::default(),
        }
    }

    /// Makes each next send fail, as an SMTP server that is down does.
    pub fn fail_sends(&self) {
        self.failing.store(true, Ordering::SeqCst);
    }

    /// Makes the next `count` sends fail, as an SMTP server that is down for a short time does.
    pub fn fail_next(&self, count: usize) {
        *self.failing_next.lock().expect("mailbox") = count;
    }

    /// Makes each next send fail with a permanent error, as an SMTP server that refuses the
    /// address does.
    pub fn refuse_sends(&self) {
        self.refusing.store(true, Ordering::SeqCst);
    }

    /// Returns the mails to an address, oldest first, after the first send of each mail of the
    /// outbox.
    pub async fn to(&self, address: &str) -> Vec<Mail> {
        self.settle().await;
        self.sent_to(address)
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
            let mails = self.sent_to(address);
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
    pub async fn token(&self, address: &str, page: &str) -> String {
        let marker = format!("/{page}#token=");
        self.to(address)
            .await
            .iter()
            .rev()
            .find_map(|mail| {
                let start = mail.body.find(&marker)? + marker.len();
                let token = mail.body[start..].split_whitespace().next()?;
                Some(token.to_owned())
            })
            .unwrap_or_else(|| panic!("no mail to {address} with a link to {page}"))
    }

    /// Returns the mails to an address that the server sent until now, oldest first.
    fn sent_to(&self, address: &str) -> Vec<Mail> {
        let mails = self.mails.lock().expect("mailbox");
        mails
            .iter()
            .filter(|mail| mail.to == address)
            .cloned()
            .collect()
    }

    /// Waits until no mail of the outbox waits for its first send.
    ///
    /// # Panics
    ///
    /// If a mail still waits after 5 seconds.
    async fn settle(&self) {
        let start = tokio::time::Instant::now();
        loop {
            let waiting = sqlx::query_scalar!(
                r#"SELECT count(*) AS "count!" FROM mail_outbox
                   WHERE state = 'pending' AND attempts = 0"#
            )
            .fetch_one(&self.pool)
            .await
            .expect("read the outbox");
            if waiting == 0 {
                return;
            }
            assert!(start.elapsed() < WAIT, "{waiting} mails wait in the outbox");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

#[async_trait]
impl Mailer for Mailbox {
    async fn send(&self, mail: Mail) -> Result<(), MailError> {
        if self.refusing.load(Ordering::SeqCst) {
            return Err(MailError::permanent(
                "the test mail server refused the mail",
            ));
        }
        let failing_next = {
            let mut count = self.failing_next.lock().expect("mailbox");
            let failing = *count > 0;
            *count = count.saturating_sub(1);
            failing
        };
        if failing_next || self.failing.load(Ordering::SeqCst) {
            return Err(MailError::transient("the test mail server is down"));
        }
        self.mails.lock().expect("mailbox").push(mail);
        Ok(())
    }
}
