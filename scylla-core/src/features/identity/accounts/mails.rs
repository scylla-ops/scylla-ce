//! The texts of the account mails, and their sending. Each mail goes to the log with its kind
//! and its masked recipient, never with its body or its link.

use scylla_extension::Mail;

use super::letter::Letter;
use super::AccountContext;
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

/// The mail with the link that verifies the email of a new user.
pub(super) fn verify_email(ctx: &AccountContext, to: &str, token: &str) -> Mail {
    Letter {
        to,
        subject: "Verify your email".to_owned(),
        intro: vec![
            "Open this link to verify your email and activate your Scylla account.".to_owned(),
        ],
        action: Some((
            "Verify your email",
            ctx.settings.link("verify-email", token),
        )),
        outro: vec![
            "The link works for one day, one time only.".to_owned(),
            "If you did not create an account, you can ignore this mail.".to_owned(),
        ],
    }
    .compose()
}

/// The mail with the link that resets a password.
pub(super) fn reset_password(ctx: &AccountContext, to: &str, token: &str) -> Mail {
    Letter {
        to,
        subject: "Reset your password".to_owned(),
        intro: vec![
            "We got a request to reset the password of your Scylla account. Open this link to \
             set a new password."
                .to_owned(),
        ],
        action: Some((
            "Reset your password",
            ctx.settings.link("reset-password", token),
        )),
        outro: vec![
            "The link works for one hour, one time only.".to_owned(),
            "If you did not ask for it, you can ignore this mail. Your password does not change."
                .to_owned(),
        ],
    }
    .compose()
}

/// The mail to the owner of an email that a sign-up used again.
pub(super) fn account_exists(ctx: &AccountContext, to: &str) -> Mail {
    Letter {
        to,
        subject: "Your Scylla account".to_owned(),
        intro: vec![
            "Someone tried to create a Scylla account with this email, but you have an account \
             already."
                .to_owned(),
        ],
        action: Some(("Sign in", ctx.settings.app_url.clone())),
        outro: vec![
            "If you forgot your password, ask for a reset on the sign-in page. If it was not \
             you, you can ignore this mail."
                .to_owned(),
        ],
    }
    .compose()
}

/// The mail to the owner of an account that got a new way to sign in.
pub(crate) fn identity_linked(ctx: &AccountContext, to: &str, provider: &str) -> Mail {
    Letter {
        to,
        subject: format!("A new sign-in method on your Scylla account: {provider}"),
        intro: vec![format!(
            "You can now sign in to your Scylla account with {provider}."
        )],
        action: Some(("Open Scylla", ctx.settings.app_url.clone())),
        outro: vec![
            "If it was not you, sign in, change your password, remove this method on the \
             Security page of your account, and tell an admin."
                .to_owned(),
        ],
    }
    .compose()
}

/// The mail with the link of an invitation into a workspace.
pub(crate) fn invitation(ctx: &AccountContext, to: &str, workspace: &str, token: &str) -> Mail {
    Letter {
        to,
        subject: format!("You are invited to the Scylla workspace {workspace}"),
        intro: vec![format!(
            "You are invited to join the workspace {workspace} on Scylla."
        )],
        action: Some((
            "Accept the invitation",
            ctx.settings.link("accept-invitation", token),
        )),
        outro: vec![
            "The link works for 7 days, for this email only.".to_owned(),
            "If you did not expect this mail, you can ignore it.".to_owned(),
        ],
    }
    .compose()
}

/// The mail to a new email, with the link that makes it the email of an account.
pub(super) fn change_email(ctx: &AccountContext, to: &str, token: &str) -> Mail {
    Letter {
        to,
        subject: "Confirm your new email".to_owned(),
        intro: vec![
            "We got a request to use this email for a Scylla account. Open this link to confirm \
             it."
            .to_owned(),
        ],
        action: Some((
            "Confirm your email",
            ctx.settings.link("change-email", token),
        )),
        outro: vec![
            "The link works for one day, one time only.".to_owned(),
            "If you did not ask for it, you can ignore this mail.".to_owned(),
        ],
    }
    .compose()
}

/// The notice to the old email of an account, after a change of its email.
pub(super) fn email_changed(to: &str, new: &str) -> Mail {
    Letter {
        to,
        subject: "Your Scylla email changed".to_owned(),
        intro: vec![format!(
            "The email of your Scylla account is now {new}. You cannot sign in with this email \
             any more."
        )],
        action: None,
        outro: vec![
            "If you did not make this change, contact the admin of your Scylla instance at once."
                .to_owned(),
        ],
    }
    .compose()
}
