//! The queue of an account mail: the legal links of the instance in its footer, then the outbox,
//! in the transaction of the change that makes the mail. The task of the outbox sends it.

use scylla_domain::platform::legal::{DocumentContent, DocumentKind, LegalSettings};
use scylla_extension::{Mail, Mailer, Tx};

use super::super::letter::{Legal, Letter};
use super::super::{AccountContext, AccountSettings};
use super::store;
use crate::features::platform::legal;
use crate::infra::logs::mask_email;
use crate::Error;

/// The legal links of the footer, in their order.
const LINKS: [(DocumentKind, &str); 2] = [
    (DocumentKind::Privacy, "Privacy policy"),
    (DocumentKind::LegalNotice, "Legal notice"),
];

/// Adds the legal links of the instance to a mail, and puts it in the outbox in the transaction
/// of the change that makes it. A rollback removes the mail. With no SMTP server, the mailer
/// logs the mail at once.
pub(crate) async fn queue(
    tx: &mut Tx<'_>,
    ctx: &AccountContext,
    kind: &'static str,
    letter: Letter,
) -> Result<(), Error> {
    let mail = compose(tx, ctx, letter).await?;
    if !ctx.mailer.delivers() {
        log_only(ctx, kind, mail).await;
        return Ok(());
    }
    store::insert(tx, kind, mail).await
}

/// Adds the legal links of the instance to a mail, and makes its text and HTML bodies.
pub(crate) async fn compose(
    tx: &mut Tx<'_>,
    ctx: &AccountContext,
    letter: Letter,
) -> Result<Mail, Error> {
    let settings = legal::read(tx).await?;
    let legal = footer(&settings, &ctx.settings);
    Ok(Letter { legal, ..letter }.compose())
}

/// Puts a mail in the outbox in its own transaction, for a mail with no change. The request
/// does not fail: a failure goes to the log.
pub(crate) async fn send(ctx: &AccountContext, kind: &'static str, letter: Letter) {
    let to = mask_email(&letter.to);
    let queued = async {
        let mut tx = ctx.pool.begin().await?;
        queue(&mut tx, ctx, kind, letter).await?;
        tx.commit().await?;
        Ok::<(), Error>(())
    };
    if let Err(error) = queued.await {
        tracing::warn!(event = "mail.failed", kind, to, %error);
    }
}

/// Gives a mail to a mailer that sends nothing, and logs `mail.sent` or `mail.failed`.
async fn log_only(ctx: &AccountContext, kind: &'static str, mail: Mail) {
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
