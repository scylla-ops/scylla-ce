//! The test of the webhook of a linked repository, for `TestDelivery`: Scylla asks the provider
//! for a delivery, then waits 10 seconds for it. GitHub sends again the last delivery of the
//! installation (`POST /app/hook/deliveries/{id}/attempts`), and GitLab sends a test push to the
//! webhook of the project (`POST /projects/:id/hooks/:hook_id/test/push_events`). A delivery that
//! arrives moves the link back to the webhook mode, and one that does not arrive moves it to the
//! polling mode. A GitHub App with no webhook moves the link to the polling mode at once.

use std::time::Duration;

use reqwest::Method;
use scylla_domain::integrations::connections::{ConnectionState, VcsConnection};
use scylla_domain::integrations::VcsProvider;
use serde::Deserialize;
use serde_json::Value;
use time::OffsetDateTime;

use super::calls::github;
use super::service;
use super::store::Linked;
use crate::features::integrations::connections::credentials;
use crate::features::integrations::polling::{self, store as polls};
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, ProviderError};
use crate::Error;

/// The longest wait for the delivery.
const WAIT: Duration = Duration::from_secs(10);

/// The time between two reads of the arrival.
const STEP: Duration = Duration::from_millis(250);

/// The most deliveries of the App that Scylla reads.
const DELIVERIES: u32 = 100;

/// The result of a test of the webhook of a link.
pub(super) struct Delivery {
    /// `true` when the test webhook arrived.
    pub(super) delivered: bool,
    /// Why the test failed, as a short text, or an empty text.
    pub(super) error: String,
}

/// What the provider did with the request of a delivery.
enum Asked {
    /// It sends the delivery.
    Sent,
    /// It refused, or it has no delivery to send, for this reason.
    Refused(String),
    /// The GitHub App has no webhook.
    NoWebhook,
}

/// A delivery of the webhook of the App, from `GET /app/hook/deliveries`.
#[derive(Deserialize)]
struct Listed {
    /// The id of the delivery.
    id: u64,
    /// The installation of the delivery.
    installation_id: Option<u64>,
}

/// Asks the provider for a test webhook of a link, waits 10 seconds for it, and moves the link to
/// the mode that the result gives. A GitHub App with no webhook moves the link and its connection
/// to the polling mode with `APP_HAS_NO_WEBHOOK`, with no wait.
pub(super) async fn test(ctx: &IntegrationContext, linked: &Linked) -> Result<Delivery, Error> {
    // Refuse a link that cannot serve.
    if linked.link.state != ConnectionState::Active {
        return Ok(failed(
            "the repository link or its connection is not active",
        ));
    }
    let connection = service::connection(ctx, linked).await?;

    // Read the time of the last webhook, then ask the provider for a delivery.
    let before = arrival(ctx, linked).await?;
    let asked = match linked.provider {
        VcsProvider::GitHub => github(ctx, &connection).await?,
        VcsProvider::GitLab => gitlab(ctx, linked).await?,
    };
    match asked {
        Asked::Sent => {}
        Asked::Refused(reason) => return Ok(failed(&reason)),
        Asked::NoWebhook => {
            no_webhook(ctx, linked, &connection).await?;
            let reason = "the GitHub App has no webhook: Scylla polls the repository";
            return Ok(failed(reason));
        }
    }

    // Wait for the delivery, then move the link.
    let project = linked.link.project_id;
    let started = tokio::time::Instant::now();
    while started.elapsed() < WAIT {
        tokio::time::sleep(STEP).await;
        if arrival(ctx, linked).await? != before {
            let mut tx = ctx.pool.begin().await?;
            polls::links::to_webhook(&mut tx, project, polling::SAFETY_SECONDS).await?;
            tx.commit().await?;
            return Ok(Delivery {
                delivered: true,
                error: String::new(),
            });
        }
    }
    let github = linked.provider == VcsProvider::GitHub;
    polling::to_polling(ctx, project, polling::WEBHOOKS_NOT_ARRIVING, github).await?;
    Ok(failed(&format!(
        "no webhook arrived within 10 seconds: {} cannot reach this server",
        name(linked.provider)
    )))
}

/// Asks GitHub to send again the last delivery of the installation of a connection, when the App
/// has a webhook.
async fn github(ctx: &IntegrationContext, connection: &VcsConnection) -> Result<Asked, Error> {
    // Check that the App has a webhook.
    let app = credentials::app_of(ctx, connection).await?;
    let refused = |error: ProviderError| Asked::Refused(format!("GitHub: {error}"));
    match github::app_has_webhook(&ctx.clients, &app).await {
        Ok(true) => {}
        Ok(false) => return Ok(Asked::NoWebhook),
        Err(error) => return Ok(refused(error)),
    }

    // Find the last delivery of the installation.
    let jwt = match app.jwt(OffsetDateTime::now_utc()) {
        Ok(jwt) => jwt,
        Err(error) => return Ok(refused(error)),
    };
    let listed = ctx
        .clients
        .github
        .get("/app/hook/deliveries")
        .auth(Auth::Bearer(&jwt))
        .query("per_page", DELIVERIES)
        .read::<Vec<Listed>>()
        .await;
    let listed = match listed {
        Ok(listed) => listed,
        Err(error) => return Ok(refused(error)),
    };
    let installation = connection.external_id.parse::<u64>().ok();
    let Some(last) = listed.iter().find(|delivery| {
        delivery.installation_id.is_some() && delivery.installation_id == installation
    }) else {
        let reason =
            "GitHub has no webhook of this installation to send again: push a commit first";
        return Ok(Asked::Refused(reason.to_owned()));
    };

    // Ask for a new delivery of it.
    let path = format!("/app/hook/deliveries/{}/attempts", last.id);
    let sent = ctx
        .clients
        .github
        .call(Method::POST, &path)
        .auth(Auth::Bearer(&jwt))
        .send::<Value>()
        .await;
    Ok(sent.map_or_else(refused, |_| Asked::Sent))
}

/// Asks GitLab for a test push on the webhook of a project. Gives the reason when the link has no
/// webhook or GitLab refuses.
async fn gitlab(ctx: &IntegrationContext, linked: &Linked) -> Result<Asked, Error> {
    // Find the webhook of the link.
    let project = linked.link.project_id;
    let mut tx = ctx.pool.begin().await?;
    let hook = polls::links::hook_id(&mut tx, project).await?;
    tx.commit().await?;
    let Some(hook) = hook else {
        let reason = "the project has no webhook: Scylla needs the Maintainer role to make one";
        return Ok(Asked::Refused(reason.to_owned()));
    };

    // Ask GitLab for a test push.
    let token = credentials::gitlab_token(ctx, linked.link.connection_id).await?;
    let path = format!(
        "/projects/{}/hooks/{hook}/test/push_events",
        linked.link.external_id
    );
    let sent = ctx
        .clients
        .gitlab
        .call(Method::POST, &path)
        .auth(Auth::PrivateToken(&token))
        .send::<Value>()
        .await;
    Ok(sent.map_or_else(
        |error| Asked::Refused(format!("GitLab: {error}")),
        |_| Asked::Sent,
    ))
}

/// Moves the link of a GitHub App with no webhook, and its connection, to the polling mode with
/// `APP_HAS_NO_WEBHOOK`.
async fn no_webhook(
    ctx: &IntegrationContext,
    linked: &Linked,
    connection: &VcsConnection,
) -> Result<(), Error> {
    let project = linked.link.project_id;
    let reason = polling::APP_HAS_NO_WEBHOOK;
    let mut tx = ctx.pool.begin().await?;
    let moved = polls::links::to_polling_for(&mut tx, project, reason).await?;
    polls::links::connection_to_polling(&mut tx, connection.id).await?;
    tx.commit().await?;
    if moved.is_some() {
        tracing::info!(%project, reason, "a repository link moved to the polling mode");
    }
    Ok(())
}

/// Returns the time of the last webhook of a link: of its connection on GitHub, where one
/// webhook serves all the repositories of an installation, and of the link on GitLab.
async fn arrival(
    ctx: &IntegrationContext,
    linked: &Linked,
) -> Result<Option<OffsetDateTime>, Error> {
    match linked.provider {
        VcsProvider::GitHub => Ok(service::connection(ctx, linked).await?.last_event_at),
        VcsProvider::GitLab => {
            let found = service::get(ctx, linked.link.project_id).await?;
            Ok(found.and_then(|found| found.link.last_event_at))
        }
    }
}

/// Returns the name of a provider.
fn name(provider: VcsProvider) -> &'static str {
    match provider {
        VcsProvider::GitHub => "GitHub",
        VcsProvider::GitLab => "GitLab",
    }
}

/// Returns a failed test with its reason.
fn failed(reason: &str) -> Delivery {
    Delivery {
        delivered: false,
        error: reason.to_owned(),
    }
}
