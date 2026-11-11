//! The token of a checkout on GitHub: an installation token of the App of the connection, narrowed
//! to the one repository and to `contents: read`. GitHub stops it after one hour.

use scylla_domain::integrations::VcsProvider;

use super::dispatch::basic;
use super::store::Target;
use crate::features::integrations::connections::credentials;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{Hidden, ProviderError, TokenScope};
use crate::Error;

/// Returns the header of a token that can only read the repository of a GitHub connection. A
/// connection whose App the server does not have gives `APP_MISSING`.
pub(super) async fn header(ctx: &IntegrationContext, target: &Target) -> Result<Hidden, Error> {
    // Take the App of the connection.
    let source = credentials::github_app(ctx)
        .await?
        .ok_or_else(credentials::app_missing)?;
    if target.app.is_some() && source.row != target.app {
        return Err(credentials::app_missing());
    }

    // Get a token of the installation for the one repository, with read access to its code.
    let installation = number(&target.account_id, "installation")?;
    let repository = number(&target.repository_id, "repository")?;
    let scope = TokenScope::default()
        .repository(repository)
        .permission("contents", "read");
    let token = source
        .app
        .installation_token(installation, &scope)
        .await
        .map_err(|error| {
            if matches!(error, ProviderError::Unauthorized) {
                source.app.forget(installation);
            }
            credentials::provider_failed(VcsProvider::GitHub, error)
        })?;
    Ok(basic("x-access-token", &token.token))
}

/// Reads a GitHub id of a row.
fn number(text: &str, what: &str) -> Result<u64, Error> {
    text.parse()
        .map_err(|_| Error::Internal(format!("the {what} id {text:?} is not a number")))
}
