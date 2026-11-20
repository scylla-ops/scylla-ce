//! The calls of the merge blocks on the provider: the change and the read of the ruleset of
//! Scylla on GitHub, and of "Pipelines must succeed" on GitLab.

use std::collections::BTreeSet;
use std::sync::Arc;

use scylla_domain::integrations::connections::VcsConnection;
use scylla_domain::integrations::VcsProvider;

use super::calls::github::Rulesets;
use super::calls::gitlab;
use super::rules::{self, Observed, Refusal};
use crate::features::integrations::connections::credentials;
use crate::features::integrations::links::store::Linked;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{GitHubApp, Hidden, InstallationToken, ProviderError, TokenScope};
use crate::Error;

/// The credentials of the provider of a link.
pub(super) enum Credentials {
    /// The GitHub App, the id of its installation and the id of the repository.
    GitHub(Arc<GitHubApp>, u64, u64),
    /// The token of the GitLab service account.
    GitLab(Hidden),
}

/// What the provider must have after a change.
pub(super) struct Wanted<'a> {
    /// The contexts of the GitHub ruleset. No context removes the ruleset.
    pub(super) contexts: &'a [String],
    /// `true` turns "Pipelines must succeed" on, on GitLab.
    pub(super) enabled: bool,
    /// The id of the GitHub ruleset of the link, or `None`.
    pub(super) ruleset: Option<i64>,
}

/// Opens the credentials of the provider of a link: the App of the connection on GitHub, the
/// token of the service account on GitLab.
pub(super) async fn credentials(
    ctx: &IntegrationContext,
    linked: &Linked,
    connection: &VcsConnection,
) -> Result<Credentials, Error> {
    match linked.provider {
        VcsProvider::GitHub => {
            let app = credentials::app_of(ctx, connection).await?;
            let installation = number(&connection.external_id, "installation")?;
            let repository = number(&linked.link.external_id, "repository")?;
            Ok(Credentials::GitHub(app, installation, repository))
        }
        VcsProvider::GitLab => {
            let token = credentials::gitlab_token(ctx, connection.id).await?;
            Ok(Credentials::GitLab(token))
        }
    }
}

/// Changes the rule on the provider. Returns the id of the GitHub ruleset after the change, or
/// the refusal when nothing changed.
pub(super) async fn apply(
    ctx: &IntegrationContext,
    credentials: &Credentials,
    linked: &Linked,
    wanted: &Wanted<'_>,
) -> Result<Option<i64>, Refusal> {
    let project = &linked.link.external_id;
    let changed = match credentials {
        Credentials::GitHub(app, installation, repository) => {
            // Get a token of the repository with `administration: write`.
            let scope = TokenScope::default()
                .repository(*repository)
                .permission("administration", "write");
            let token = match token(app, *installation, &scope).await {
                Ok(token) => token,
                Err(ProviderError::Invalid(_)) => {
                    return Err(rules::permission_missing(VcsProvider::GitHub));
                }
                Err(error) => return Err(failed(linked, &error)),
            };

            // Change the ruleset.
            let rulesets = Rulesets {
                clients: &ctx.clients,
                token: &token.token,
                full_name: &linked.link.full_name,
            };
            let app_id = app.settings().app_id;
            let changed = github(&rulesets, wanted, app_id).await;
            if matches!(changed, Err(ProviderError::Unauthorized)) {
                app.forget(*installation);
            }
            changed
        }
        Credentials::GitLab(token) => gitlab::set(&ctx.clients, token, project, wanted.enabled)
            .await
            .map(|()| None),
    };
    changed.map_err(|error| failed(linked, &error))
}

/// Reads the rule on the provider. A failed read gives `Unknown`, and a warning in the log.
pub(super) async fn observe(
    ctx: &IntegrationContext,
    credentials: &Credentials,
    linked: &Linked,
    ruleset: Option<i64>,
) -> Observed {
    // Read the ruleset of Scylla, or the setting of the GitLab project.
    let found = match credentials {
        Credentials::GitHub(app, installation, repository) => {
            let Some(id) = ruleset else {
                return Observed::GitHub(BTreeSet::new());
            };
            let scope = TokenScope::default()
                .repository(*repository)
                .permission("metadata", "read");
            match token(app, *installation, &scope).await {
                Ok(token) => {
                    let rulesets = Rulesets {
                        clients: &ctx.clients,
                        token: &token.token,
                        full_name: &linked.link.full_name,
                    };
                    let read = rulesets.read(id).await;
                    read.map(|found| Observed::GitHub(rules::required(found.as_ref())))
                }
                Err(error) => Err(error),
            }
        }
        Credentials::GitLab(token) => {
            let project = &linked.link.external_id;
            let read = gitlab::read(&ctx.clients, token, project).await;
            read.map(Observed::GitLab)
        }
    };

    // Keep the stored state after a failed read.
    found.unwrap_or_else(|error| {
        let project = linked.link.project_id;
        tracing::warn!(%project, %error, "the provider did not give the merge rule");
        Observed::Unknown
    })
}

/// Changes the ruleset of Scylla: it removes the ruleset when no context stays, replaces it, or
/// makes it when the repository does not have it. Returns its id.
async fn github(
    rulesets: &Rulesets<'_>,
    wanted: &Wanted<'_>,
    app_id: u64,
) -> Result<Option<i64>, ProviderError> {
    // Find the ruleset of Scylla.
    let id = match wanted.ruleset {
        Some(id) => Some(id),
        None => rulesets.find().await?,
    };

    // Remove it when no context stays.
    if wanted.contexts.is_empty() {
        if let Some(id) = id {
            rulesets.delete(id).await?;
        }
        return Ok(None);
    }

    // Replace it, or make it when it is not there.
    let body = rules::ruleset_body(wanted.contexts, app_id);
    if let Some(id) = id {
        match rulesets.update(id, body.clone()).await {
            Ok(()) => return Ok(Some(id)),
            Err(ProviderError::NotFound) => {}
            Err(error) => return Err(error),
        }
    }
    rulesets.create(body).await.map(Some)
}

/// Returns an installation token for a scope. A refused token drops the tokens of the
/// installation.
async fn token(
    app: &GitHubApp,
    installation: u64,
    scope: &TokenScope,
) -> Result<InstallationToken, ProviderError> {
    let token = app.installation_token(installation, scope).await;
    if matches!(token, Err(ProviderError::Unauthorized)) {
        app.forget(installation);
    }
    token
}

/// Returns the refusal of a failed call, and logs the error with no secret.
fn failed(linked: &Linked, error: &ProviderError) -> Refusal {
    let project = linked.link.project_id;
    tracing::warn!(%project, %error, "the provider did not change the merge rule");
    rules::refusal(linked.provider, linked.link.private, error)
}

/// Reads a GitHub id of a row.
fn number(text: &str, what: &str) -> Result<u64, Error> {
    text.parse()
        .map_err(|_| Error::Internal(format!("the {what} id {text:?} is not a number")))
}
