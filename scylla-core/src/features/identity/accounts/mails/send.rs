//! The sending of an account mail: the legal links of the instance in its footer, then the log of
//! its kind and its masked recipient, never of its body or its link.

use scylla_domain::platform::legal::{DocumentContent, DocumentKind, LegalSettings};

use super::super::letter::{Legal, Letter};
use super::super::{AccountContext, AccountSettings};
use crate::features::platform::legal;
use crate::infra::logs::mask_email;

/// The legal links of the footer, in their order.
const LINKS: [(DocumentKind, &str); 2] = [
    (DocumentKind::Privacy, "Privacy policy"),
    (DocumentKind::LegalNotice, "Legal notice"),
];

/// Adds the legal links of the instance to a mail, sends it, and logs `mail.sent` or
/// `mail.failed`. The request does not fail: the user can ask again.
pub(crate) async fn send(ctx: &AccountContext, kind: &'static str, letter: Letter) {
    // Add the legal part of the footer. A failed read gives a footer with no legal part.
    let legal = match legal::get(&ctx.pool).await {
        Ok(settings) => footer(&settings, &ctx.settings),
        Err(error) => {
            tracing::warn!(%error, "the mail has no legal links");
            Legal::default()
        }
    };
    let mail = Letter { legal, ..letter }.compose();

    // Send the mail, and log the result.
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

/// Makes the legal part of the footer: the link of each document of `LINKS` that is on, and the
/// name and the address of the operator.
fn footer(legal: &LegalSettings, settings: &AccountSettings) -> Legal {
    let links = LINKS
        .into_iter()
        .filter_map(|(kind, label)| {
            let document = legal.document(kind)?;
            let url = match &document.values.content {
                DocumentContent::Url(url) => url.clone(),
                DocumentContent::Markdown(_) => settings.page(&format!("legal/{}", kind.as_str())),
            };
            Some((label, url))
        })
        .collect();
    let operator = [&legal.operator.name, &legal.operator.address]
        .into_iter()
        .flat_map(|text| text.lines())
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    Legal { links, operator }
}
