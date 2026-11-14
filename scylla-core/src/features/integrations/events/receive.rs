//! The public routes of the webhooks of the providers: `POST /integrations/github/webhook` (the
//! App of the settings), `POST /integrations/github/{app}/webhook` (the App of the instance) and
//! `POST /integrations/gitlab/{project}/webhook` (the webhook of a linked GitLab project). They
//! have no token: the signature of the body is the credential, and each route checks it before
//! any write. A new delivery gets 202 at once, and a task does its work.

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use scylla_domain::integrations::apps::VcsAppId;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;
use serde_json::json;
use time::OffsetDateTime;
use tracing::Instrument;

use super::answers::{answer, bad_request, failure, header, rejected, unknown};
use super::dispatch::{self, Incoming, Scope};
use super::store::{self, Delivery};
use super::{github, gitlab};
use crate::features::integrations::apps::store as apps;
use crate::features::integrations::connections::credentials;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{check_github, check_gitlab, GitLabDelivery, Hidden};
use crate::Error;

/// The largest body, in bytes.
const MAX_BODY: usize = 5 << 20;

/// Where a webhook arrived.
enum Arrival<'a> {
    /// A GitHub installation, and the repository of the delivery.
    GitHub(Option<VcsAppId>, Option<&'a str>, Option<&'a str>),
    /// The GitLab webhook of a project.
    GitLab(ProjectId),
}

/// Makes the routes, with their limit of the body.
pub(super) fn router(ctx: IntegrationContext) -> Router {
    Router::new()
        .route("/integrations/github/webhook", post(github_settings))
        .route("/integrations/github/{app}/webhook", post(github_instance))
        .route(
            "/integrations/gitlab/{project}/webhook",
            post(gitlab_project),
        )
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .with_state(ctx)
}

/// Handles a delivery of the App of the settings. A server with no such App answers 404.
async fn github_settings(
    State(ctx): State<IntegrationContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(app) = &ctx.clients.github_app else {
        return unknown();
    };
    let secret = app.settings().webhook_secret.clone();
    github_delivery(&ctx, None, &secret, &headers, &body).await
}

/// Handles a delivery of the App of the instance. An unknown App answers 404.
async fn github_instance(
    State(ctx): State<IntegrationContext>,
    Path(app): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(app) = VcsAppId::parse(&app) else {
        return unknown();
    };
    match instance_secret(&ctx, app).await {
        Ok(Some(secret)) => github_delivery(&ctx, Some(app), &secret, &headers, &body).await,
        Ok(None) => unknown(),
        Err(error) => failure(error),
    }
}

/// Handles a GitHub delivery: 401 for a bad signature, 200 for a ping or a second delivery, 400
/// for a delivery with no id or a body that is not JSON, else 202.
async fn github_delivery(
    ctx: &IntegrationContext,
    app: Option<VcsAppId>,
    secret: &Hidden,
    headers: &HeaderMap,
    body: &[u8],
) -> Response {
    // Check the signature before any write.
    let signature = header(headers, "x-hub-signature-256");
    if let Err(error) = check_github(secret.expose().as_bytes(), signature, body) {
        return rejected(VcsProvider::GitHub, error, None);
    }

    // Answer a ping, and read the delivery.
    let event = header(headers, "x-github-event").unwrap_or_default();
    if event == "ping" {
        return answer(StatusCode::OK, json!({ "ping": true }));
    }
    let Some(key) = header(headers, "x-github-delivery").filter(|key| !key.is_empty()) else {
        return bad_request("the delivery has no id");
    };
    let read = match github::read(event, key, body) {
        Ok(read) => read,
        Err(reason) => return bad_request(&reason),
    };

    // Keep it one time, and do its work.
    let scope = read.installation.clone().map(|installation| Scope::GitHub {
        app,
        installation,
        repository: read.repository.clone(),
    });
    let delivery = Delivery {
        provider: VcsProvider::GitHub,
        key,
        app,
        connection: None,
        project: None,
        event,
        action: read.action.as_deref(),
    };
    let installation = read.installation.as_deref();
    let arrival = Arrival::GitHub(app, installation, read.repository.as_deref());
    accept(ctx, &delivery, arrival, scope, read.incoming).await
}

/// Handles a delivery of the GitLab webhook of a project: 404 for a project with no webhook,
/// 401 for a bad signature, 200 for a second delivery, 400 for a body that is not JSON, else
/// 202.
async fn gitlab_project(
    State(ctx): State<IntegrationContext>,
    Path(project): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Find the signing token of the webhook.
    let Ok(project) = ProjectId::parse(&project) else {
        return unknown();
    };
    let (hook, secret) = match hook_secret(&ctx, project).await {
        Ok(Some(found)) => found,
        Ok(None) => return unknown(),
        Err(error) => return failure(error),
    };

    // Check the signature before any write.
    let signed = GitLabDelivery {
        id: header(&headers, "webhook-id"),
        timestamp: header(&headers, "webhook-timestamp"),
        signature: header(&headers, "webhook-signature"),
    };
    let now = OffsetDateTime::now_utc();
    if let Err(error) = check_gitlab(&secret, &signed, &body, now) {
        return rejected(VcsProvider::GitLab, error, Some(project));
    }

    // Read the delivery. A delivery about another GitLab project starts nothing.
    let key = header(&headers, "idempotency-key")
        .filter(|key| !key.is_empty())
        .or(signed.id)
        .unwrap_or_default();
    let read = match gitlab::read(key, &body) {
        Ok(read) => read,
        Err(reason) => return bad_request(&reason),
    };
    let incoming = match read.project.as_deref() == Some(hook.external_id.as_str()) {
        true => read.incoming,
        false => Incoming::Ignored,
    };

    // Keep it one time, and do its work.
    let delivery = Delivery {
        provider: VcsProvider::GitLab,
        key,
        app: None,
        connection: Some(hook.connection),
        project: Some(project),
        event: &read.event,
        action: read.action.as_deref(),
    };
    let scope = Some(Scope::Project(project));
    accept(&ctx, &delivery, Arrival::GitLab(project), scope, incoming).await
}

/// Keeps a delivery one time and notes the arrival of the webhook, then starts its work in a
/// task: 202, or 200 for a second delivery.
async fn accept(
    ctx: &IntegrationContext,
    delivery: &Delivery<'_>,
    arrival: Arrival<'_>,
    scope: Option<Scope>,
    incoming: Incoming,
) -> Response {
    // Keep the delivery one time, and note the arrival of the webhook.
    let kept = async {
        let mut tx = ctx.pool.begin().await?;
        let new = store::receive(&mut tx, delivery).await?;
        match arrival {
            Arrival::GitHub(app, Some(installation), repository) => {
                store::github_arrived(&mut tx, app, installation, repository).await?;
            }
            Arrival::GitHub(..) => {}
            Arrival::GitLab(project) => store::gitlab_arrived(&mut tx, project).await?,
        }
        tx.commit().await?;
        Ok::<bool, Error>(new)
    };
    match kept.await {
        Ok(true) => {}
        Ok(false) => return answer(StatusCode::OK, json!({ "duplicate": true })),
        Err(error) => return failure(error),
    }

    // Do the work in a task.
    let (ctx, provider, key) = (ctx.clone(), delivery.provider, delivery.key.to_owned());
    let work = async move {
        dispatch::finish(&ctx, provider, &key, scope.as_ref(), &incoming).await;
    };
    tokio::spawn(work.in_current_span());
    answer(StatusCode::ACCEPTED, json!({ "accepted": true }))
}

/// Opens the webhook secret of the App of the instance, or `None` when the instance has no App
/// with this id.
async fn instance_secret(ctx: &IntegrationContext, id: VcsAppId) -> Result<Option<Hidden>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let row = apps::github(&mut tx).await?;
    tx.commit().await?;
    let Some(row) = row.filter(|row| row.id == id) else {
        return Ok(None);
    };
    let sealer = credentials::key(ctx.sealer.as_deref())?;
    open(sealer.open_with(&row.webhook_secret, &id.to_bytes())).map(Some)
}

/// Returns the GitLab webhook of a project and its signing token, or `None` for a project with
/// no webhook.
async fn hook_secret(
    ctx: &IntegrationContext,
    project: ProjectId,
) -> Result<Option<(store::Hook, Hidden)>, Error> {
    let mut tx = ctx.pool.begin().await?;
    let hook = store::hook(&mut tx, project).await?;
    tx.commit().await?;
    let Some(hook) = hook else {
        return Ok(None);
    };
    let sealer = credentials::key(ctx.sealer.as_deref())?;
    let secret = open(sealer.open_with(&hook.sealed, &project.to_bytes()))?;
    Ok(Some((hook, secret)))
}

/// Reads an opened secret as a text. The error never has the secret.
fn open(plain: Result<Vec<u8>, String>) -> Result<Hidden, Error> {
    let plain = plain.map_err(|reason| Error::Internal(format!("a webhook secret: {reason}")))?;
    let text = String::from_utf8(plain)
        .map_err(|_| Error::Internal("a webhook secret is not UTF-8".to_owned()))?;
    Ok(Hidden::new(text))
}
