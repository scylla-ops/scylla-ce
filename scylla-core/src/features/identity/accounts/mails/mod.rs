//! The account mails: the mails with a one-time link, the notices about the security of an
//! account, the test mail of the mail settings, their outbox and the task that sends them. Each
//! mail goes to the log with its kind and its masked recipient, never with its body or its link.

mod links;
mod notices;
mod outbox;
mod send;
mod store;
mod test_mail;

pub(crate) use links::{account_exists, change_email, invitation, reset_password, verify_email};
pub(crate) use notices::{
    account_locked, email_changed, identity_linked, identity_unlinked, password_changed,
    session_reuse, token_created,
};
pub(crate) use outbox::spawn;
pub(crate) use send::{compose, queue, send};
pub(crate) use test_mail::test_mail;
