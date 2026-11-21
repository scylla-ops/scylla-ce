//! Unit tests of the layout of the account mails.

use time::{OffsetDateTime, UtcOffset};

use super::content::{clock, LOGO_DARK_ID, LOGO_ID};
use super::html::escape;
use super::{Action, Badge, Fact, Letter, NotYou, Part};

/// Returns the time of a count of seconds after the Unix epoch.
fn at(seconds: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(seconds).expect("a valid time")
}

/// A mail with each block: a strong value with markup, two facts, a one-time link and a link
/// for a person who did not ask for the mail.
fn letter() -> Letter {
    Letter {
        subject: "Join <team>".to_owned(),
        preheader: "Join the workspace <team>.".to_owned(),
        badge: Badge::INVITATION,
        title: "Join <team>".to_owned(),
        lead: vec![Part::strong("Ann & Bob"), Part::plain(" invited you.")],
        facts: vec![
            Fact::new("Workspace", "<team>"),
            Fact::time("Sent", at(1_795_098_725)),
        ],
        action: Some(
            Action::new("Accept", "https://app/x#token=a&b".to_owned())
                .note("The link works for 7 days.")
                .one_time(),
        ),
        not_you: NotYou::new("Not you?", "Ignore this mail.")
            .link("Sign in", "https://app/login".to_owned()),
        reason: "You get this mail because Ann invited you.".to_owned(),
        ..Letter::new("ann@example.com", "scylla.example.com")
    }
}

/// Each character that HTML reads as markup is escaped.
#[test]
fn escape_replaces_the_markup_characters() {
    // given
    let value = r#"<a href="x">Tom & 'Jerry'</a>"#;

    // when
    let escaped = escape(value);

    // then
    assert_eq!(
        escaped,
        "&lt;a href=&quot;x&quot;&gt;Tom &amp; &#39;Jerry&#39;&lt;/a&gt;"
    );
}

/// The text body has each block and each link as it is, with no greeting, and the footer names
/// the recipient and the instance.
#[test]
fn compose_writes_the_text_body() {
    // when
    let mail = letter().compose();

    // then
    let expected = "Join <team>\n\nAnn & Bob invited you.\n\n\
                    Workspace: <team>\nSent: Nov 19, 2026 · 14:32 UTC\n\n\
                    Accept: https://app/x#token=a&b\nThe link works for 7 days.\n\n\
                    Not you?\nIgnore this mail.\nSign in: https://app/login\n\n\
                    --\nSent to ann@example.com by the Scylla instance at scylla.example.com.\n\
                    You get this mail because Ann invited you.\n";
    assert_eq!(mail.body, expected);
    assert_eq!(mail.subject, "Join <team>");
}

/// The HTML body escapes each value, hides the preheader, has the colors of the dark theme in
/// its style sheet, and shows the logo of the mail with its alt text.
#[test]
fn compose_writes_the_html_body_with_the_logo() {
    // when
    let mail = letter().compose();

    // then
    let html = &mail.html;
    assert!(html.starts_with("<!DOCTYPE html>"));
    assert!(!html.contains("<team>") && !html.contains("Hello,"));
    assert!(html.contains(">Join &lt;team&gt;</h1>"));
    assert!(html.contains(">Ann &amp; Bob</strong> invited you.</p>"));
    assert!(html.contains("mso-hide:all;opacity:0;"));
    assert!(html.contains(r#"<meta name="color-scheme" content="light dark">"#));
    assert!(html.contains("@media (prefers-color-scheme: dark)"));
    assert!(html.contains(&format!(r#"src="cid:{LOGO_ID}""#)));
    assert!(html.contains(&format!(r#"src="cid:{LOGO_DARK_ID}""#)));
    assert!(html.contains(r#"alt="Scylla""#));
    let ids: Vec<_> = mail.images.iter().map(|image| image.id.as_str()).collect();
    assert_eq!(ids, [LOGO_ID, LOGO_DARK_ID]);
    assert!(mail
        .images
        .iter()
        .all(|image| image.content_type == "image/png" && image.bytes.starts_with(b"\x89PNG")));
}

/// A one-time link is the button and a copy as text. Another link is the button only.
#[test]
fn compose_copies_only_a_one_time_link() {
    // given
    let mut other = letter();
    other.action = Some(Action::new(
        "Review",
        "https://app/account/security".to_owned(),
    ));

    // when
    let one_time = letter().compose().html;
    let other = other.compose().html;

    // then
    let link = r#"href="https://app/x#token=a&amp;b""#;
    assert_eq!(one_time.matches(link).count(), 2);
    assert!(one_time.contains("Button not working?"));
    assert_eq!(
        other
            .matches(r#"href="https://app/account/security""#)
            .count(),
        1
    );
    assert!(!other.contains("Button not working?"));
}

/// The facts and the badge take their tone, and a clock shows the hour and the minute in UTC.
#[test]
fn compose_writes_the_facts_and_the_badge() {
    // given
    let mut alert = letter();
    alert.badge = Badge::ALERT;
    alert.facts.push(Fact::code("Name", "ci-deploy"));

    // when
    let html = alert.compose().html;

    // then
    assert!(html.contains(">Workspace</td>"));
    assert!(html.contains(">Nov 19, 2026 · 14:32 UTC</td>"));
    assert!(html.contains("JetBrains Mono"));
    assert!(html.contains("background-color:#ECA851;"));
    assert!(html.contains(">Security alert</span>"));
    let paris = at(1_795_096_980).to_offset(UtcOffset::from_hms(1, 0, 0).unwrap());
    assert_eq!(clock(paris), "14:03 UTC");
}
