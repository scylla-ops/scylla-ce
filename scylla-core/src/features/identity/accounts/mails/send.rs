//! The sending of an account mail: the log of its kind and its masked recipient, never of its
//! body or its link.

use scylla_extension::Mail;

use super::super::AccountContext;
use crate::infra::logs::mask_email;

/// Sends a mail of a kind, and logs `mail.sent` or `mail.failed`. The request does not fail:
/// the user can ask again.
pub(crate) async fn send(ctx: &AccountContext, kind: &'static str, mail: Mail) {
    let address = mail.to.clone();
    let to = mask_email(&address);
    match ctx.mailer.send(mail).await {
        Ok(()) => tracing::info!(event = "mail.sent", kind, to),
        Err(error) => {
            let error = error.to_string().replace(&address, &to);
            tracing::warn!(event = "mail.failed", kind, to, error);
        }
    }
}
