//! The texts of the account mails, and their sending. A failed mail goes to the log, with no
//! link.

use scylla_extension::Mail;

use super::AccountContext;

/// Sends a mail, and logs a failure. The request does not fail: the user can ask again.
pub(crate) async fn send(ctx: &AccountContext, mail: Mail) {
    let (to, subject) = (mail.to.clone(), mail.subject.clone());
    if let Err(error) = ctx.mailer.send(mail).await {
        tracing::warn!(%error, %to, %subject, "a mail was not sent");
    }
}

/// The mail with the link that verifies the email of a new user.
pub(super) fn verify_email(ctx: &AccountContext, to: &str, token: &str) -> Mail {
    let link = ctx.settings.link("verify-email", token);
    Mail {
        to: to.to_owned(),
        subject: "Verify your email".to_owned(),
        body: format!(
            "Open this link to verify your email and activate your Scylla account:\n\n{link}\n\n\
             The link works for one day."
        ),
    }
}

/// The mail with the link that resets a password.
pub(super) fn reset_password(ctx: &AccountContext, to: &str, token: &str) -> Mail {
    let link = ctx.settings.link("reset-password", token);
    Mail {
        to: to.to_owned(),
        subject: "Reset your password".to_owned(),
        body: format!(
            "Open this link to set a new password for your Scylla account:\n\n{link}\n\n\
             The link works for one hour. If you did not ask for it, you can ignore this mail."
        ),
    }
}

/// The mail to the owner of an email that a sign-up used again.
pub(super) fn account_exists(ctx: &AccountContext, to: &str) -> Mail {
    let app = &ctx.settings.app_url;
    Mail {
        to: to.to_owned(),
        subject: "Your Scylla account".to_owned(),
        body: format!(
            "Someone tried to create a Scylla account with this email, but you have an account \
             already. Sign in at {app}. If you forgot your password, ask for a reset on the \
             sign-in page. If it was not you, you can ignore this mail."
        ),
    }
}

/// The mail with the link of an invitation into a workspace.
pub(crate) fn invitation(ctx: &AccountContext, to: &str, workspace: &str, token: &str) -> Mail {
    let link = ctx.settings.link("accept-invitation", token);
    Mail {
        to: to.to_owned(),
        subject: format!("You are invited to the Scylla workspace {workspace}"),
        body: format!(
            "Open this link to join the workspace {workspace}:\n\n{link}\n\n\
             The link works for 7 days, for this email only."
        ),
    }
}
