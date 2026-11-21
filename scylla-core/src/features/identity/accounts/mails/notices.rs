//! The notices about the security of an account: a new email, a new password, a new or a removed
//! way to sign in, a new personal token, a closed session and a locked account.

use scylla_domain::identity::api_tokens::ApiTokenInfo;
use time::OffsetDateTime;

use super::super::letter::{clock, Action, Badge, Fact, Letter, NotYou, Part};
use super::super::AccountContext;
use crate::features::identity::sessions::Client;

/// Why the owner of an account gets a notice about its security.
const SECURITY: &str =
    "Scylla sends this notice after each change to the security of your account.";

/// The notice to the old email of an account, after a change of its email.
pub(crate) fn email_changed(ctx: &AccountContext, old: &str, new: &str) -> Letter {
    Letter {
        subject: "The email of your Scylla account changed".to_owned(),
        preheader: format!("Your account now signs in with {new}, not with this email."),
        badge: Badge::SECURITY,
        title: "Your email changed".to_owned(),
        lead: vec![
            Part::plain("You can no longer sign in with "),
            Part::strong(old),
            Part::plain("."),
        ],
        facts: vec![
            Fact::new("Old email", old),
            Fact::new("New email", new),
            Fact::time("Changed", OffsetDateTime::now_utc()),
        ],
        not_you: NotYou::new(
            "Not you?",
            "Contact the admin of your Scylla instance at once.",
        ),
        reason: "Scylla sends this notice to the old email after each change of email.".to_owned(),
        ..Letter::new(old, ctx.settings.instance())
    }
}

/// The notice to the owner of an account after a change of its password, with the client that
/// made the change.
pub(crate) fn password_changed(ctx: &AccountContext, to: &str, from: Option<&Client>) -> Letter {
    let mut facts = vec![Fact::time("Changed", OffsetDateTime::now_utc())];
    facts.extend(from.map(|client| Fact::new("From", client.describe())));
    Letter {
        subject: "Your Scylla password changed".to_owned(),
        preheader: format!("The password of {to} changed. Not you? Reset it now."),
        badge: Badge::SECURITY,
        title: "Your password changed".to_owned(),
        lead: vec![
            Part::plain("The password of your Scylla account "),
            Part::strong(to),
            Part::plain(" changed. If you made this change, you have nothing to do."),
        ],
        facts,
        action: Some(review(ctx, "Review your security", "account/security")),
        not_you: NotYou::new(
            "Not you?",
            "Someone may know your password. Reset it now, and tell an admin.",
        )
        .link("Reset your password", ctx.settings.page("forgot-password")),
        reason: SECURITY.to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The notice to the owner of an account that got a new way to sign in.
pub(crate) fn identity_linked(ctx: &AccountContext, to: &str, provider: &str) -> Letter {
    Letter {
        subject: format!("A new sign-in method on your Scylla account: {provider}"),
        preheader: format!("You can now sign in with {provider}. Not you? Remove it now."),
        badge: Badge::SECURITY,
        title: format!("New sign-in method: {provider}"),
        lead: vec![
            Part::plain("You can now sign in to your Scylla account with "),
            Part::strong(provider),
            Part::plain("."),
        ],
        facts: vec![
            Fact::new("Method", provider),
            Fact::time("Added", OffsetDateTime::now_utc()),
        ],
        action: Some(review(ctx, "Review sign-in methods", "account/security")),
        not_you: NotYou::new(
            "Not you?",
            format!("Remove {provider}, change your password, and tell an admin."),
        ),
        reason: SECURITY.to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The notice to the owner of an account that lost a way to sign in.
pub(crate) fn identity_unlinked(ctx: &AccountContext, to: &str, provider: &str) -> Letter {
    Letter {
        subject: format!("A sign-in method left your Scylla account: {provider}"),
        preheader: format!(
            "You can no longer sign in with {provider}. Your other methods still work."
        ),
        badge: Badge::SECURITY,
        title: format!("{provider} was removed"),
        lead: vec![
            Part::plain("You can no longer sign in to Scylla with "),
            Part::strong(provider),
            Part::plain("."),
        ],
        facts: vec![
            Fact::new("Method", provider),
            Fact::time("Removed", OffsetDateTime::now_utc()),
        ],
        action: Some(review(ctx, "Review sign-in methods", "account/security")),
        not_you: NotYou::new(
            "Not you?",
            "Change your password at once, and tell an admin of your instance.",
        ),
        reason: SECURITY.to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The notice to the owner of an account that got a new personal token, with its expiry.
pub(crate) fn token_created(ctx: &AccountContext, to: &str, token: &ApiTokenInfo) -> Letter {
    let name = token.name.as_str();
    Letter {
        subject: format!("A new personal token on your Scylla account: {name}"),
        preheader: format!("The token {name} can now act as your account. Not you? Revoke it now."),
        badge: Badge::SECURITY,
        title: "New personal token".to_owned(),
        lead: vec![
            Part::plain("The token "),
            Part::strong(name),
            Part::plain(" can now act as your Scylla account."),
        ],
        facts: vec![
            Fact::code("Name", name),
            Fact::day("Expires", token.expires_at),
            Fact::time("Created", token.created_at),
        ],
        action: Some(review(ctx, "Review your tokens", "account/tokens")),
        not_you: NotYou::new(
            "Not you?",
            "Revoke this token, change your password, and tell an admin.",
        ),
        reason: SECURITY.to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The alert to the owner of a session that closed because someone used an old refresh token:
/// the device of the session, and the last address that used it.
pub(crate) fn session_reuse(ctx: &AccountContext, to: &str, session: &Client) -> Letter {
    let mut facts = vec![Fact::new("Session", session.device())];
    facts.extend(
        session
            .ip
            .map(|ip| Fact::new("Last address", ip.to_string())),
    );
    facts.push(Fact::time("Closed", OffsetDateTime::now_utc()));
    Letter {
        subject: "A Scylla session of your account closed".to_owned(),
        preheader: "An old token of one of your sessions came back, so Scylla closed the session."
            .to_owned(),
        badge: Badge::ALERT,
        title: "We closed a session".to_owned(),
        lead: vec![Part::plain(
            "Someone used an old token of one of your sessions, so Scylla closed it. A copy of \
             this token may be on another device.",
        )],
        facts,
        action: Some(review(ctx, "Review your security", "account/security")),
        not_you: NotYou::new(
            "Did not expect it?",
            "Change your password, close your other sessions, and tell an admin.",
        ),
        reason: SECURITY.to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// The alert to the owner of an account that many failed passwords locked, with the end of the
/// lock and the count of the failures.
pub(crate) fn account_locked(
    ctx: &AccountContext,
    to: &str,
    until: OffsetDateTime,
    failures: i32,
) -> Letter {
    Letter {
        subject: format!("Your Scylla account is locked until {}", clock(until)),
        preheader: "Too many wrong passwords. Sign in again after the lock, or choose a new \
                    password."
            .to_owned(),
        badge: Badge::ALERT,
        title: "Your account is locked".to_owned(),
        lead: vec![
            Part::plain("Too many wrong passwords locked "),
            Part::strong(to),
            Part::plain(" for a short time. Wait for the lock to end, or choose a new password."),
        ],
        facts: vec![
            Fact::time("Locked until", until),
            Fact::new("Wrong passwords", format!("{failures} in a row")),
        ],
        action: Some(
            Action::new("Reset your password", ctx.settings.page("forgot-password"))
                .note("A new password ends the lock at once."),
        ),
        not_you: NotYou::new(
            "Not you?",
            "Someone may try your password. Choose a new one, and tell an admin.",
        ),
        reason: SECURITY.to_owned(),
        ..Letter::new(to, ctx.settings.instance())
    }
}

/// Makes the link to a page of the account in the web app.
fn review(ctx: &AccountContext, label: &'static str, page: &str) -> Action {
    Action::new(label, ctx.settings.page(page))
}
