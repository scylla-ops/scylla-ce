//! The content of an account mail before its layout: a badge, a title, a lead, the facts, the
//! main link, the block for a person who did not ask for the mail, and the footer with the legal
//! links of the instance.

use scylla_extension::{Mail, MailImage};
use time::{OffsetDateTime, UtcOffset};

use super::{html, text, Legal};

/// The logo with dark letters, for a light client.
const LOGO: &[u8] = include_bytes!("logo.png");

/// The logo with light letters, for a client in dark mode.
const LOGO_DARK: &[u8] = include_bytes!("logo-dark.png");

/// The content id of the logo with dark letters.
pub(super) const LOGO_ID: &str = "logo.light@scylla";

/// The content id of the logo with light letters.
pub(super) const LOGO_DARK_ID: &str = "logo.dark@scylla";

/// The colors of a badge, as the badges of the web app.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    /// The neutral badge: an account mail or a security notice.
    Neutral,
    /// The badge in the color of the brand: an invitation.
    Brand,
    /// The warning badge: a security alert.
    Warning,
}

/// The badge above the title: what the mail is about.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Badge {
    /// The text.
    pub(crate) label: &'static str,
    /// The colors.
    pub(crate) tone: Tone,
}

impl Badge {
    /// A mail about the account: a check of an email, a sign-up, a new email.
    pub(crate) const ACCOUNT: Self = Self::new("Account", Tone::Neutral);
    /// A notice about the security of the account.
    pub(crate) const SECURITY: Self = Self::new("Security", Tone::Neutral);
    /// A notice that the person must read: a closed session, a locked account.
    pub(crate) const ALERT: Self = Self::new("Security alert", Tone::Warning);
    /// An invitation into a workspace.
    pub(crate) const INVITATION: Self = Self::new("Invitation", Tone::Brand);

    /// Makes a badge.
    const fn new(label: &'static str, tone: Tone) -> Self {
        Self { label, tone }
    }
}

/// A part of the lead: a plain text, or a strong value such as an email.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// A plain text.
    Plain(String),
    /// A value that the person must see.
    Strong(String),
}

impl Part {
    /// Makes a plain text.
    pub(crate) fn plain(text: impl Into<String>) -> Self {
        Self::Plain(text.into())
    }

    /// Makes a strong value.
    pub(crate) fn strong(text: impl Into<String>) -> Self {
        Self::Strong(text.into())
    }
}

/// A fact of the mail: a label and its value, on one row of a table.
#[derive(Clone, Debug)]
pub(crate) struct Fact {
    /// The label.
    pub(crate) label: &'static str,
    /// The value.
    pub(crate) value: String,
    /// `true` for a value in the code font, such as the name of a token.
    pub(crate) code: bool,
}

impl Fact {
    /// Makes a fact with a text value.
    pub(crate) fn new(label: &'static str, value: impl Into<String>) -> Self {
        let value = value.into();
        Self {
            label,
            value,
            code: false,
        }
    }

    /// Makes a fact with a value in the code font.
    pub(crate) fn code(label: &'static str, value: impl Into<String>) -> Self {
        Self {
            code: true,
            ..Self::new(label, value)
        }
    }

    /// Makes a fact with a time in UTC: `Nov 19, 2026 · 14:32 UTC`.
    pub(crate) fn time(label: &'static str, at: OffsetDateTime) -> Self {
        Self::new(label, format!("{} · {}", day(at), clock(at)))
    }

    /// Makes a fact with a day: `Feb 17, 2027`.
    pub(crate) fn day(label: &'static str, at: OffsetDateTime) -> Self {
        Self::new(label, day(at))
    }
}

/// The main link of the mail: a button, and a note under it.
#[derive(Clone, Debug)]
pub(crate) struct Action {
    /// The text of the button.
    pub(crate) label: &'static str,
    /// The address of the link.
    pub(crate) url: String,
    /// The note under the button, if one helps.
    pub(crate) note: Option<String>,
    /// `true` for a one-time link: the mail also shows it as text, for a client where the button
    /// does not work.
    pub(crate) one_time: bool,
}

impl Action {
    /// Makes a link with no note.
    pub(crate) fn new(label: &'static str, url: String) -> Self {
        Self {
            label,
            url,
            note: None,
            one_time: false,
        }
    }

    /// Adds the note under the button.
    pub(crate) fn note(self, note: impl Into<String>) -> Self {
        let note = Some(note.into());
        Self { note, ..self }
    }

    /// Marks a link with a token, which the mail also shows as text.
    pub(crate) fn one_time(self) -> Self {
        Self {
            one_time: true,
            ..self
        }
    }
}

/// The block for a person who did not ask for the mail: a question, what to do, and a link if
/// one helps.
#[derive(Clone, Debug)]
pub(crate) struct NotYou {
    /// The question.
    pub(crate) title: &'static str,
    /// What to do.
    pub(crate) text: String,
    /// A link: the text of its button, and its address.
    pub(crate) link: Option<(&'static str, String)>,
}

impl NotYou {
    /// Makes the block with no link.
    pub(crate) fn new(title: &'static str, text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            title,
            text,
            link: None,
        }
    }

    /// Adds a link: the text of its button, and its address.
    pub(crate) fn link(self, label: &'static str, url: String) -> Self {
        let link = Some((label, url));
        Self { link, ..self }
    }
}

/// The content of a mail.
#[derive(Clone, Debug)]
pub(crate) struct Letter {
    /// The address of the recipient.
    pub(crate) to: String,
    /// The address of the instance, with no scheme: `scylla.example.com`.
    pub(crate) instance: String,
    /// The subject.
    pub(crate) subject: String,
    /// The text that a client shows after the subject in a list of mails.
    pub(crate) preheader: String,
    /// The badge above the title.
    pub(crate) badge: Badge,
    /// The title.
    pub(crate) title: String,
    /// The text under the title.
    pub(crate) lead: Vec<Part>,
    /// The facts, in a table.
    pub(crate) facts: Vec<Fact>,
    /// The main link.
    pub(crate) action: Option<Action>,
    /// The block for a person who did not ask for the mail.
    pub(crate) not_you: NotYou,
    /// Why the recipient gets the mail, in the footer.
    pub(crate) reason: String,
    /// The legal links and the operator of the instance, at the end of the footer.
    pub(crate) legal: Legal,
}

impl Letter {
    /// Makes an empty mail to a recipient, from an instance.
    pub(crate) fn new(to: &str, instance: &str) -> Self {
        Self {
            to: to.to_owned(),
            instance: instance.to_owned(),
            subject: String::new(),
            preheader: String::new(),
            badge: Badge::ACCOUNT,
            title: String::new(),
            lead: Vec::new(),
            facts: Vec::new(),
            action: None,
            not_you: NotYou::new("", ""),
            reason: String::new(),
            legal: Legal::default(),
        }
    }

    /// Makes the mail: a text body, an HTML body with each value escaped, and the two logos.
    pub(crate) fn compose(self) -> Mail {
        let image = |id: &str, bytes: &[u8]| MailImage {
            id: id.to_owned(),
            content_type: "image/png".to_owned(),
            bytes: bytes.to_vec(),
        };
        Mail {
            to: self.to.clone(),
            body: text::body(&self),
            html: html::body(&self),
            subject: self.subject,
            images: vec![image(LOGO_ID, LOGO), image(LOGO_DARK_ID, LOGO_DARK)],
        }
    }
}

/// Writes the hour and the minute of a time in UTC: `14:33 UTC`.
pub(crate) fn clock(at: OffsetDateTime) -> String {
    let at = at.to_offset(UtcOffset::UTC);
    format!("{:02}:{:02} UTC", at.hour(), at.minute())
}

/// Writes the day of a time in UTC: `Feb 17, 2027`.
fn day(at: OffsetDateTime) -> String {
    let at = at.to_offset(UtcOffset::UTC);
    let month = at.month().to_string();
    let short = month.get(..3).unwrap_or(&month);
    format!("{short} {}, {}", at.day(), at.year())
}
