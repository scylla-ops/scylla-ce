//! A new link to verify an email, for an account that waits for the check of its email: on
//! request, and at a new sign-up with its email. At most one link each `COOLDOWN` for one account,
//! and a rate limit of the requests for each IP address and each email.

use std::net::IpAddr;
use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{Email, UserStatus};
use tracing::Instrument;

use super::one_time::{self, Purpose};
use super::{mails, store, AccountContext};
use crate::infra::rates::Rates;
use crate::Error;

/// The most requests of a new link from one IP address in one window.
const PER_ADDRESS: u32 = 20;

/// The most requests of a new link for one email in one window.
const PER_EMAIL: u32 = 5;

/// The window of the rate limits.
const WINDOW: Duration = Duration::from_secs(60 * 60);

/// The rate limits of the requests of a new link: one for each IP address, one for each email.
pub(super) struct Resends {
    /// The requests of each IP address. A request with no known address counts in one window.
    by_address: Rates<Option<IpAddr>>,
    /// The requests for each email.
    by_email: Rates<String>,
}

impl Default for Resends {
    fn default() -> Self {
        Self {
            by_address: Rates::new(PER_ADDRESS, WINDOW),
            by_email: Rates::new(PER_EMAIL, WINDOW),
        }
    }
}

impl Resends {
    /// Counts a request, and returns `false` over the limit of its address or of its email.
    pub(super) fn allow(&self, ip: Option<IpAddr>, email: &Email) -> bool {
        self.by_address.allow(ip) && self.by_email.allow(email.as_str().to_owned())
    }
}

/// Answers at once, then sends a new link to the email in a task, if its account waits for the
/// check of its email. The answer is the same for each email, and an error goes to the log only.
pub(super) fn request_link(ctx: &AccountContext, email: Email) {
    let ctx = ctx.clone();
    let task = async move {
        let sent = async {
            // Find the user of the email.
            let mut tx = ctx.pool.begin().await?;
            let account = store::account_of_email(&mut tx, &email).await?;
            tx.commit().await?;

            // Send a link to an account that waits for its check only.
            match account {
                Some(account) if account.status == UserStatus::Pending => {
                    send_link(&ctx, account.user, email.as_str()).await
                }
                _ => Ok(()),
            }
        };
        if let Err(error) = sent.await {
            tracing::warn!(%error, "a link to verify an email was not made");
        }
    };
    tokio::spawn(task.in_current_span());
}

/// Makes a new link to verify the email of a user that waits for its check, and puts its mail in
/// the outbox, unless the user got one less than `COOLDOWN` ago. The older links stay valid until
/// they expire.
pub(super) async fn send_link(
    ctx: &AccountContext,
    user: ActorId,
    email: &str,
) -> Result<(), Error> {
    // Lock the user, so two requests at the same time make one link only.
    let mut tx = ctx.pool.begin().await?;
    let pending = store::lock_user(&mut tx, user).await?
        && store::find_user(&mut tx, user)
            .await?
            .is_some_and(|found| found.status == UserStatus::Pending);
    if !pending || one_time::is_recent(&mut tx, Purpose::VerifyEmail, user).await? {
        return Ok(());
    }

    // Make the link, and put its mail in the outbox.
    let token = one_time::create(&mut tx, Purpose::VerifyEmail, user, email).await?;
    let mail = mails::verify_email(ctx, email, &token);
    mails::queue(&mut tx, ctx, "verify-email", mail).await?;
    tx.commit().await?;
    Ok(())
}
