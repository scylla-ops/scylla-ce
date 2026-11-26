//! The test mail of the mail settings: an instance admin sends it to check the relay of the
//! instance. It has the layout of the account mails, so a spam filter sees a real mail.

use time::OffsetDateTime;

use super::super::letter::{Fact, Letter, NotYou, Part};
use super::super::AccountContext;

/// The test mail to an address, through the relay of this host.
pub(crate) fn test_mail(ctx: &AccountContext, to: &str, relay: &str) -> Letter {
    let instance = ctx.settings.instance();
    Letter {
        subject: "Test mail from Scylla".to_owned(),
        title: "The mail relay works".to_owned(),
        lead: vec![
            Part::plain("An admin of "),
            Part::strong(instance),
            Part::plain(" sent this test mail to check the mail relay of the server."),
        ],
        facts: vec![
            Fact::new("Relay", relay),
            Fact::time("Sent", OffsetDateTime::now_utc()),
        ],
        not_you: NotYou::new(
            "Did not expect it?",
            "You can delete this mail. Scylla sends no other mail for this test.",
        ),
        reason: "An admin of this Scylla server sent this test mail from the mail settings."
            .to_owned(),
        ..Letter::new(to, instance)
    }
}
