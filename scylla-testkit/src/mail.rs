//! A mailer that keeps the mails, so a test reads their links.

use std::sync::Mutex;

use scylla_extension::{async_trait, Mail, MailError, Mailer};

/// The mails that the server of a test sent, oldest first.
#[derive(Default)]
pub struct Mailbox {
    /// The mails.
    mails: Mutex<Vec<Mail>>,
}

impl Mailbox {
    /// Returns the mails to an address, oldest first.
    pub fn to(&self, address: &str) -> Vec<Mail> {
        let mails = self.mails.lock().expect("mailbox");
        mails
            .iter()
            .filter(|mail| mail.to == address)
            .cloned()
            .collect()
    }

    /// Returns the token of the last link to a page of the web app in a mail to an address.
    ///
    /// # Panics
    ///
    /// If no mail to the address has such a link.
    pub fn token(&self, address: &str, page: &str) -> String {
        let marker = format!("/{page}?token=");
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
        self.mails.lock().expect("mailbox").push(mail);
        Ok(())
    }
}
