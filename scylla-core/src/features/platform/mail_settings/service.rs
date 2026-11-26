//! The logic of the mail settings: the relay of the instance in the database, with its password
//! sealed with the key of the secrets, and the mailer of the instance, which uses it at the next
//! send with no restart.

use std::sync::Arc;
use std::time::Duration;

use scylla_domain::identity::users::Email;
use scylla_domain::platform::mail::MailRelay;
use scylla_extension::{MailReceipt, Mailer, Rejection};

use super::store;
use crate::features::identity::accounts::{mails, AccountContext};
use crate::infra::logs::mask_email;
use crate::infra::mail::{self, BrokenRelay, SmtpMailer};
use crate::infra::secret::Sealer;
use crate::infra::vcs::Hidden;
use crate::Error;

/// The associated data of the seal of a password: a sealed password opens only as a password of
/// the mail settings.
const PASSWORD_DATA: &[u8] = b"mail_settings.password";

/// The longest wait for the answer of the relay to a test mail.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// What a change does to the saved password.
pub(super) enum Password {
    /// The saved password stays.
    Keep,
    /// A new password replaces it.
    Replace(Hidden),
    /// The relay has no password.
    Remove,
}

/// The mail settings that the API shows.
pub(super) struct Settings {
    /// The relay of the instance, and `true` when the server keeps its password.
    pub(super) relay: Option<(MailRelay, bool)>,
    /// The host of the default mailer of the server, if it has a mail server.
    pub(super) default_host: Option<String>,
}

/// The result of a test mail.
pub(super) struct TestOutcome {
    /// The host of the relay that got the mail.
    pub(super) host: String,
    /// The answer of the relay, or why it did not accept the mail.
    pub(super) result: Result<MailReceipt, String>,
}

/// Reads the relay of the mail settings at the start of the server, and gives it to the mailer of
/// the instance. A relay that the server cannot use makes each mail wait in the outbox. Settings
/// that the server cannot read do not stop its start: it keeps its default mailer.
pub(crate) async fn load(ctx: &AccountContext) {
    let saved = match saved(ctx).await {
        Ok(Some(saved)) => saved,
        Ok(None) => return,
        Err(error) => {
            let error =
                format!("the mail settings cannot be read, the default mailer stays: {error}");
            tracing::warn!(event = "mail.settings_unusable", error);
            return;
        }
    };
    let relay = saved.relay;
    let mailer: Arc<dyn Mailer> = match mailer_of(ctx, &relay, saved.password.as_deref()) {
        Ok(mailer) => Arc::new(mailer),
        Err(reason) => {
            tracing::warn!(event = "mail.settings_unusable", host = %relay.host, error = %reason);
            Arc::new(BrokenRelay {
                host: relay.host.clone(),
                reason: format!("the mail settings cannot be used: {reason}"),
            })
        }
    };
    ctx.mailer.set(mailer, relay.from_address);
}

/// Reads the saved relay in its own transaction.
async fn saved(ctx: &AccountContext) -> Result<Option<store::Saved>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let saved = store::get(&mut tx).await?;
    tx.commit().await?;
    Ok(saved)
}

/// Returns the mail settings of the instance.
pub(super) async fn get(ctx: &AccountContext) -> Result<Settings, Error> {
    let mut tx = ctx.pool.begin().await?;
    let saved = store::get(&mut tx).await?;
    tx.commit().await?;
    let relay = saved.map(|saved| (saved.relay, saved.password.is_some()));
    Ok(settings(ctx, relay))
}

/// Sets the relay of the instance, and sends the next mails with it.
pub(super) async fn update(
    ctx: &AccountContext,
    relay: MailRelay,
    password: Password,
) -> Result<Settings, Error> {
    // Choose the password of the relay: the new one, the saved one, or none.
    let mut tx = ctx.pool.begin().await?;
    let saved = store::get(&mut tx).await?.and_then(|saved| saved.password);
    let required = || Error::invalid("password", "is required with a user name");
    let (plain, sealed) = match (relay.authenticates(), password) {
        (false, _) => (None, None),
        (true, Password::Remove) => return Err(required()),
        (true, Password::Replace(plain)) => {
            let sealed = key(ctx)?.seal_with(plain.expose().as_bytes(), PASSWORD_DATA);
            (Some(plain), Some(sealed))
        }
        (true, Password::Keep) => {
            let sealed = saved.ok_or_else(required)?;
            let plain = open(key(ctx)?, &sealed).map_err(|reason| {
                Error::invalid("password", format!("must be given again: {reason}"))
            })?;
            (Some(plain), Some(sealed))
        }
    };

    // Make the mailer of the relay, then save the relay.
    mail::sender_address(&relay.from_address)
        .map_err(|reason| Error::invalid("from_address", reason))?;
    let mailer = SmtpMailer::for_relay(&relay, plain.as_ref(), &ctx.settings.app_url)
        .map_err(|reason| Error::invalid("host", reason))?;
    store::save(&mut tx, &relay, sealed.as_deref()).await?;
    tx.commit().await?;

    // Send the next mails with the new relay.
    ctx.mailer.set(Arc::new(mailer), relay.from_address.clone());
    tracing::info!(
        event = "mail.settings_changed",
        host = %relay.host,
        port = relay.port,
        security = relay.security.as_str(),
        auth = relay.authenticates(),
        from = %mask_email(&relay.from_address)
    );
    Ok(settings(ctx, Some((relay, sealed.is_some()))))
}

/// Removes the relay of the instance, and sends the next mails with the default mailer of the
/// server.
pub(super) async fn reset(ctx: &AccountContext) -> Result<Settings, Error> {
    let mut tx = ctx.pool.begin().await?;
    let removed = store::delete(&mut tx).await?;
    tx.commit().await?;
    ctx.mailer.reset();
    if removed {
        let default_host = ctx.mailer.default_relay();
        tracing::info!(event = "mail.settings_reset", default_host);
    }
    Ok(settings(ctx, None))
}

/// Sends a test mail to an address at once, through the relay that the server uses now, and
/// returns the answer of the relay. A server that sends no mail gives `MAIL_RELAY_MISSING`.
pub(super) async fn test(ctx: &AccountContext, to: &Email) -> Result<TestOutcome, Error> {
    // Refuse a server that logs the mails and sends none.
    if !ctx.mailer.delivers() {
        let message = "the server has no mail relay: set one, or set SCYLLA_SMTP_URL";
        return Err(Error::Rejected(Rejection::new(
            "MAIL_RELAY_MISSING",
            message,
        )));
    }

    // Make the test mail with the layout and the legal links of the account mails.
    let host = ctx.mailer.relay().unwrap_or_default();
    let mut tx = ctx.pool.begin().await?;
    let letter = mails::test_mail(ctx, to.as_str(), &host);
    let mail = mails::compose(&mut tx, ctx, letter).await?;
    tx.commit().await?;

    // Send it, and log the answer of the relay with the masked recipient.
    let sent = tokio::time::timeout(TEST_TIMEOUT, ctx.mailer.send_traced(mail)).await;
    let result = match sent {
        Ok(Ok(receipt)) => Ok(receipt),
        Ok(Err(error)) => Err(error.reason),
        Err(_) => Err(format!(
            "the relay did not answer in {} seconds",
            TEST_TIMEOUT.as_secs()
        )),
    };
    let masked = mask_email(to.as_str());
    let mask = |text: &str| text.replace(to.as_str(), &masked);
    match &result {
        Ok(receipt) => {
            let reply = receipt.reply.as_deref().map(mask);
            tracing::info!(event = "mail.tested", to = %masked, relay = %host, reply);
        }
        Err(error) => {
            let error = mask(error);
            tracing::warn!(event = "mail.tested", to = %masked, relay = %host, error);
        }
    }
    Ok(TestOutcome { host, result })
}

/// Makes the mailer of a saved relay, with its sealed password.
fn mailer_of(
    ctx: &AccountContext,
    relay: &MailRelay,
    sealed: Option<&[u8]>,
) -> Result<SmtpMailer, String> {
    let password = match sealed {
        Some(sealed) => {
            let sealer = ctx.sealer.as_deref().ok_or("the server has no key")?;
            Some(open(sealer, sealed)?)
        }
        None => None,
    };
    SmtpMailer::for_relay(relay, password.as_ref(), &ctx.settings.app_url)
}

/// Opens a sealed password.
fn open(sealer: &Sealer, sealed: &[u8]) -> Result<Hidden, String> {
    let plain = sealer.open_with(sealed, PASSWORD_DATA)?;
    String::from_utf8(plain)
        .map(Hidden::new)
        .map_err(|_| "the saved password is not text".to_owned())
}

/// Returns the key of the secrets, or `SECRET_KEY_MISSING` if the server has none.
fn key(ctx: &AccountContext) -> Result<&Sealer, Error> {
    ctx.sealer.as_deref().ok_or_else(|| {
        let message = "the server has no key for the secrets: set SCYLLA_SECRET_KEY";
        Error::Rejected(Rejection::new("SECRET_KEY_MISSING", message))
    })
}

/// Makes the settings that the API shows.
fn settings(ctx: &AccountContext, relay: Option<(MailRelay, bool)>) -> Settings {
    Settings {
        relay,
        default_host: ctx.mailer.default_relay(),
    }
}
