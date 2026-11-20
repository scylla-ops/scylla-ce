//! A commit status on GitLab: `POST /projects/:id/statuses/:sha`, with the token of the service
//! account of the connection. The name of the status is the name of the report.

use reqwest::Method;
use scylla_domain::integrations::connections::VcsConnectionId;
use serde::Deserialize;
use serde_json::{json, Value};

use super::failures::Failure;
use super::sender::{Outgoing, Sent};
use super::store::queue::Facts;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, Hidden, ProviderError};

/// The refusal of GitLab for a status that it has already: the state did not change.
const SAME_STATE: &str = "Cannot transition status";

/// The answer of `POST /projects/:id/statuses/:sha`.
#[derive(Deserialize)]
struct Answer {
    /// The GitLab pipeline of the status, when GitLab gives it.
    pipeline_id: Option<u64>,
}

/// Posts the commit status of a report. `pipeline` is the GitLab pipeline of an earlier status
/// of the report, so each state goes to the same pipeline.
pub(super) async fn send(
    ctx: &IntegrationContext,
    connection: VcsConnectionId,
    facts: &Facts,
    outgoing: &Outgoing,
    pipeline: Option<&str>,
) -> Result<Sent, Failure> {
    // Open the token of the service account.
    let token = token(ctx, connection, facts)?;

    // Post the status, on the branch, the tag or the source branch of the pull request.
    let mut body = json!({
        "state": outgoing.state,
        "name": outgoing.context,
        "description": outgoing.description,
        "target_url": outgoing.target_url,
    });
    let git_ref = facts.branch.as_ref().or(facts.tag.as_ref());
    if let Some(git_ref) = git_ref.or(facts.pull_branch.as_ref()) {
        body["ref"] = Value::String(git_ref.clone());
    }
    if let Some(pipeline) = pipeline.and_then(|id| id.parse::<u64>().ok()) {
        body["pipeline_id"] = Value::from(pipeline);
    }
    let path = format!("/projects/{}/statuses/{}", facts.repository_id, facts.sha);
    let call = ctx
        .clients
        .gitlab
        .call(Method::POST, &path)
        .auth(Auth::PrivateToken(&token))
        .json(body);

    // Keep the pipeline of the status. A state that GitLab has already is sent.
    let answer: Option<Answer> = match call.read().await {
        Ok(answer) => Some(answer),
        Err(ProviderError::Invalid(message)) if message.contains(SAME_STATE) => None,
        Err(error) => return Err(error.into()),
    };
    let pipeline = answer.and_then(|answer| answer.pipeline_id);
    let external_url = match pipeline {
        Some(id) => format!("{}/-/pipelines/{id}", facts.url),
        None => format!("{}/-/commit/{}", facts.url, facts.sha),
    };
    Ok(Sent {
        state: outgoing.state.to_owned(),
        external_id: pipeline.map(|id| id.to_string()),
        external_url,
    })
}

/// Opens the token of the service account, sealed with the id of the connection.
fn token(
    ctx: &IntegrationContext,
    connection: VcsConnectionId,
    facts: &Facts,
) -> Result<Hidden, Failure> {
    let Some(sealer) = ctx.sealer.as_deref() else {
        let reason = "the server has no key of the secrets: set SCYLLA_SECRET_KEY";
        return Err(Failure::Server(reason.to_owned()));
    };
    let broken = |reason: &str| Failure::Server(format!("the token of the connection {reason}"));
    let sealed = facts
        .sealed_token
        .as_deref()
        .ok_or_else(|| broken("is missing"))?;
    let plain = sealer
        .open_with(sealed, &connection.to_bytes())
        .map_err(|_| broken("does not open"))?;
    String::from_utf8(plain)
        .map(Hidden::new)
        .map_err(|_| broken("is not UTF-8"))
}
