//! The account mails of a sign-up and of a one-time link: the check of an email, a sign-up with a
//! known email, the reset of a password, a new email, and an invitation into a workspace.

use time::OffsetDateTime;

use super::super::letter::{Action, Badge, Fact, Letter, NotYou, Part};
use super::super::AccountContext;
use crate::features::identity::sessions::Client;

/// The mail with the link that verifies the email of a new user.
pub(crate) fn verify_email(ctx: &AccountContext, to: &str, token: &str) -> Letter {
    let link = ctx.settings.link("verify-email", token);
    Letter {
        subject: "Verify your email".to_owned(),
        preheader: format!(
            "Confirm {to} to activate your Scylla account. The link works for 24 hours."
        ),
        badge: Badge::ACCOUNT,
        title: "Welcome to Scylla".to_owned(),
        lead: vec![
            Part::plain("Confirm "),
            Part::strong(to),
            Part::plain(" to activate your account."),
        ],
        action: Some(
            Action::new("Verify your email", link)
                .note("The link works for 24 hours, one time only.")
                .one_time(),
        ),
        not_you: NotYou::new(
            "Did not create an account?",
            "Ignore this mail. Without a verified email, the account stays inactive.",
        ),
        reason: "You get this mail because someone created a Scylla account with this email."
            .to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The mail to the owner of an email that a sign-up used again.
pub(crate) fn account_exists(ctx: &AccountContext, to: &str) -> Letter {
    Letter {
        subject: "You already have a Scylla account".to_owned(),
        preheader: format!("Someone tried to sign up with {to}. Sign in, or reset your password."),
        badge: Badge::ACCOUNT,
        title: "You already have an account".to_owned(),
        lead: vec![
            Part::plain("Someone tried to sign up with "),
            Part::strong(to),
            Part::plain(", which has an account already. Scylla created nothing new."),
        ],
        action: Some(
            Action::new("Sign in", ctx.settings.page("login"))
                .note("Forgot your password? Ask for a reset on the sign-in page."),
        ),
        not_you: NotYou::new(
            "Not you?",
            "Ignore this mail. Nothing changed on your account.",
        ),
        reason: "You get this mail because someone used this email to sign up on Scylla."
            .to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The mail with the link that resets a password. It names the client that asked for it, when a
/// person asked on the sign-in page.
pub(crate) fn reset_password(
    ctx: &AccountContext,
    to: &str,
    token: &str,
    from: Option<&Client>,
) -> Letter {
    let mut facts = vec![Fact::time("Requested", OffsetDateTime::now_utc())];
    facts.extend(from.map(|client| Fact::new("From", client.describe())));
    let link = ctx.settings.link("reset-password", token);
    Letter {
        subject: "Reset your Scylla password".to_owned(),
        preheader: "Choose a new password with the link in this mail. It works for 1 hour."
            .to_owned(),
        badge: Badge::SECURITY,
        title: "Reset your password".to_owned(),
        lead: vec![
            Part::plain("We got a request to reset the password of "),
            Part::strong(to),
            Part::plain(". Choose a new password with the button below."),
        ],
        facts,
        action: Some(
            Action::new("Choose a new password", link)
                .note("The link works for 1 hour, one time only.")
                .one_time(),
        ),
        not_you: NotYou::new(
            "Did not ask for it?",
            "Ignore this mail. Your password stays the same.",
        ),
        reason: "You get this mail because someone asked to reset the password of this email."
            .to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The mail to a new email, with the link that makes it the email of an account.
pub(crate) fn change_email(
    ctx: &AccountContext,
    current: Option<&str>,
    to: &str,
    token: &str,
) -> Letter {
    let mut facts: Vec<Fact> = current
        .map(|current| Fact::new("Current email", current))
        .into_iter()
        .collect();
    facts.push(Fact::new("New email", to));
    let link = ctx.settings.link("change-email", token);
    Letter {
        subject: "Confirm your new email".to_owned(),
        preheader: format!("Confirm {to} as the new email of your Scylla account."),
        badge: Badge::ACCOUNT,
        title: "Confirm your new email".to_owned(),
        lead: vec![Part::plain(
            "We got a request to use this email for a Scylla account. Confirm it, and you sign \
             in with it from now on.",
        )],
        facts,
        action: Some(
            Action::new("Confirm this email", link)
                .note("The link works for 24 hours, one time only.")
                .one_time(),
        ),
        not_you: NotYou::new(
            "Did not ask for it?",
            "Ignore this mail. The email of the account does not change.",
        ),
        reason: "You get this mail because someone entered this email on a Scylla account."
            .to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The mail with the link of an invitation into a workspace, with the person who invited and
/// the role.
pub(crate) fn invitation(
    ctx: &AccountContext,
    to: &str,
    workspace: &str,
    inviter: Option<&str>,
    role: &str,
    token: &str,
) -> Letter {
    let role = role_name(role);
    let article = if role.starts_with(['A', 'E', 'I', 'O', 'U']) {
        "an"
    } else {
        "a"
    };
    let (subject, lead) = match inviter {
        Some(name) => (
            format!("{name} invited you to {workspace} on Scylla"),
            vec![
                Part::strong(name),
                Part::plain(" invited you to a workspace on Scylla."),
            ],
        ),
        None => (
            format!("You are invited to {workspace} on Scylla"),
            vec![Part::plain("You are invited to a workspace on Scylla.")],
        ),
    };
    Letter {
        subject,
        preheader: format!(
            "Join the workspace {workspace} as {article} {role}. The invitation works for 7 days."
        ),
        badge: Badge::INVITATION,
        title: format!("Join {workspace}"),
        lead,
        facts: vec![Fact::new("Workspace", workspace), Fact::new("Role", role)],
        action: Some(
            Action::new(
                "Accept the invitation",
                ctx.settings.link("accept-invitation", token),
            )
            .note(format!("The invitation works for 7 days, for {to} only."))
            .one_time(),
        ),
        not_you: NotYou::new(
            "Did not expect it?",
            "Ignore this mail. You join the workspace only if you accept.",
        ),
        reason: format!("You get this mail because a member of {workspace} invited this email."),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// Writes the name of a role for a person: `billing_admin` gives `Billing admin`.
fn role_name(role: &str) -> String {
    let words = role.replace(['_', '-'], " ");
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}
