//! The GitLab deploy tokens of the checkouts: one token with `read_repository` for each dispatch
//! of a job, until the end of the longest run of the job. Scylla keeps its row, and deletes the
//! token when no job needs it.

use reqwest::Method;
use scylla_domain::integrations::checkout::{token_life, CheckoutToken, CheckoutTokenId};
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::jobs::Job;
use serde::Deserialize;
use serde_json::{json, Value};
use time::OffsetDateTime;

use super::dispatch::basic;
use super::store::{self, Target};
use crate::features::integrations::connections::credentials;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, Hidden, ProviderError};
use crate::Error;

/// The answer of `POST /projects/:id/deploy_tokens`.
#[derive(Deserialize)]
struct Created {
    /// The id of the token.
    id: u64,
    /// The user name of the token, for Git over HTTP.
    username: String,
    /// The token.
    token: Hidden,
}

/// The end of the delete of a deploy token.
pub(super) enum Deleted {
    /// GitLab deleted the token, or does not know it.
    Gone,
    /// GitLab limits the calls until this time.
    Limited(OffsetDateTime),
    /// The delete failed, with the reason.
    Failed(String),
}

/// Makes a deploy token that can only read the repository of a GitLab connection, keeps its row
/// for the job, and returns its header.
pub(super) async fn header(
    ctx: &IntegrationContext,
    target: &Target,
    job: &Job,
) -> Result<Hidden, Error> {
    // Make a token that reads the repository until the end of the longest run of the job.
    let token = credentials::gitlab_token(ctx, target.connection).await?;
    let expires_at = OffsetDateTime::now_utc() + token_life(&job.nodes);
    let body = json!({
        "name": format!("scylla-job-{}", job.id),
        "scopes": ["read_repository"],
        "expires_at": rfc3339(expires_at)?,
    });
    let path = format!("/projects/{}/deploy_tokens", target.repository_id);
    let call = ctx.clients.gitlab.call(Method::POST, &path);
    let created: Created = call
        .auth(Auth::PrivateToken(&token))
        .json(body)
        .read()
        .await
        .map_err(|error| credentials::provider_failed(VcsProvider::GitLab, error))?;

    // Keep its row, so Scylla deletes it when no job needs it.
    let row = CheckoutToken {
        id: CheckoutTokenId::generate(),
        job_id: Some(job.id),
        connection_id: target.connection,
        project_external_id: target.repository_id.clone(),
        token_external_id: created.id.to_string(),
        expires_at,
    };
    let mut tx = ctx.pool.begin().await?;
    store::insert(&mut tx, &row).await?;
    tx.commit().await?;
    Ok(basic(&created.username, &created.token))
}

/// Deletes a deploy token on GitLab with the token of the service account of its connection.
pub(super) async fn delete(ctx: &IntegrationContext, row: &CheckoutToken) -> Deleted {
    // Open the token of the service account.
    let token = match credentials::gitlab_token(ctx, row.connection_id).await {
        Ok(token) => token,
        Err(error) => return Deleted::Failed(error.to_string()),
    };

    // Delete the deploy token.
    let path = format!(
        "/projects/{}/deploy_tokens/{}",
        row.project_external_id, row.token_external_id
    );
    let call = ctx.clients.gitlab.call(Method::DELETE, &path);
    match call.auth(Auth::PrivateToken(&token)).read::<Value>().await {
        Ok(_) | Err(ProviderError::NotFound) => Deleted::Gone,
        Err(ProviderError::RateLimited(until)) => Deleted::Limited(until),
        Err(error) => Deleted::Failed(error.to_string()),
    }
}

/// Writes a time in RFC 3339, in UTC, to the second.
fn rfc3339(at: OffsetDateTime) -> Result<String, Error> {
    chrono::DateTime::from_timestamp(at.unix_timestamp(), 0)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .ok_or_else(|| Error::Internal(format!("{at} is not a time of RFC 3339")))
}
