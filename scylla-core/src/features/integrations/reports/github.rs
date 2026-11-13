//! A commit status on GitHub: `POST /repos/{owner}/{repo}/statuses/{sha}`, with an installation
//! token narrowed to the repository and to `statuses: write`.

use reqwest::Method;
use scylla_domain::integrations::apps::VcsAppId;
use serde::Deserialize;
use serde_json::json;

use super::failures::Failure;
use super::sender::{Outgoing, Sent};
use super::store::queue::Facts;
use crate::features::integrations::connections::credentials;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, ProviderError, TokenScope};

/// The answer of `POST /repos/{owner}/{repo}/statuses/{sha}`.
#[derive(Deserialize)]
struct Answer {
    /// The id of the status.
    id: u64,
}

/// Posts the commit status of a report.
pub(super) async fn send(
    ctx: &IntegrationContext,
    facts: &Facts,
    outgoing: &Outgoing,
) -> Result<Sent, Failure> {
    // Take the App of the connection.
    let source = credentials::github_app(ctx)
        .await
        .map_err(|error| Failure::Server(error.to_string()))?;
    let app_id = facts.app_id.map(VcsAppId::from_db);
    let source = source.filter(|source| app_id.is_none() || source.row == app_id);
    let Some(source) = source else {
        return Err(Failure::Server(
            "the server has no GitHub App for the connection".to_owned(),
        ));
    };

    // Get a token of the installation for the one repository.
    let installation = number(&facts.account_id, "installation")?;
    let repository = number(&facts.repository_id, "repository")?;
    let scope = TokenScope::default()
        .repository(repository)
        .permission("statuses", "write");
    let token = source.app.installation_token(installation, &scope).await?;

    // Post the status.
    let path = format!("/repos/{}/statuses/{}", facts.repository, facts.sha);
    let body = json!({
        "state": outgoing.state,
        "context": outgoing.context,
        "description": outgoing.description,
        "target_url": outgoing.target_url,
    });
    let call = ctx
        .clients
        .github
        .call(Method::POST, &path)
        .auth(Auth::Bearer(&token.token))
        .json(body);
    let answer: Answer = call.read().await.inspect_err(|error| {
        if matches!(error, ProviderError::Unauthorized) {
            source.app.forget(installation);
        }
    })?;
    Ok(Sent {
        state: outgoing.state.to_owned(),
        external_id: Some(answer.id.to_string()),
        external_url: format!("{}/commit/{}", facts.url, facts.sha),
    })
}

/// Reads a GitHub id of a row.
fn number(text: &str, what: &str) -> Result<u64, Failure> {
    text.parse()
        .map_err(|_| Failure::Server(format!("the {what} id {text:?} is not a number")))
}
