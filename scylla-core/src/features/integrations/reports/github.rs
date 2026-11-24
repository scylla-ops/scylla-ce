//! The token of a check run on GitHub: an installation token narrowed to the repository of the
//! report and to one permission.

use std::sync::Arc;

use scylla_domain::integrations::apps::VcsAppId;
use serde::de::DeserializeOwned;

use super::failures::Failure;
use super::store::queue::Facts;
use crate::features::integrations::connections::credentials;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Auth, Call, GitHubApp, InstallationToken, ProviderError, TokenScope};

/// An installation token of the repository of a report, with the App that made it.
pub(super) struct RepositoryToken {
    /// The App of the connection.
    app: Arc<GitHubApp>,
    /// The id of the installation.
    installation: u64,
    /// The token.
    token: InstallationToken,
}

impl RepositoryToken {
    /// Sends a call with the token, and reads its answer. A refused token leaves the cache.
    pub(super) async fn read<'a, T: DeserializeOwned>(
        &'a self,
        call: Call<'a>,
    ) -> Result<T, ProviderError> {
        let call = call.auth(Auth::Bearer(&self.token.token));
        call.read().await.inspect_err(|error| {
            if matches!(error, ProviderError::Unauthorized) {
                self.app.forget(self.installation);
            }
        })
    }
}

/// Returns a token of the installation of the connection of a report, narrowed to its
/// repository and to `write` on one permission.
pub(super) async fn token(
    ctx: &IntegrationContext,
    facts: &Facts,
    permission: &str,
) -> Result<RepositoryToken, Failure> {
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
        .permission(permission, "write");
    let token = source.app.installation_token(installation, &scope).await?;
    Ok(RepositoryToken {
        app: source.app,
        installation,
        token,
    })
}

/// Reads a GitHub id of a row.
fn number(text: &str, what: &str) -> Result<u64, Failure> {
    text.parse()
        .map_err(|_| Failure::Server(format!("the {what} id {text:?} is not a number")))
}
