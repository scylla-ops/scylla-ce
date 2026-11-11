//! The linked repository of a project, with the credentials of its connection: a token of the
//! installation of the GitHub App narrowed to the repository and to `contents: read`, or the token
//! of the GitLab service account.

use std::sync::Arc;

use base64::Engine;
use scylla_domain::integrations::VcsProvider;

use super::{github, gitlab};
use crate::features::integrations::connections::credentials;
use crate::features::integrations::connections::store as connections;
use crate::features::integrations::links::store::Linked;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::{GitHubApp, Hidden, ProviderError, TokenScope};
use crate::Error;

/// A file of the folders of the pipeline files, as the provider lists it.
pub(super) struct Entry {
    /// The path from the root of the repository.
    pub(super) path: String,
    /// The SHA of its blob.
    pub(super) blob_sha: String,
    /// The size in bytes, when the provider gives it.
    pub(super) size: Option<u64>,
}

/// The credentials of a call to the repository.
enum Access {
    /// An installation token of the GitHub App, with the App that forgets it after a 401.
    GitHub {
        /// The App.
        app: Arc<GitHubApp>,
        /// The installation.
        installation: u64,
        /// The token.
        token: Hidden,
    },
    /// The token of the GitLab service account.
    GitLab(Hidden),
}

/// The repository of a project, ready for calls.
pub(super) struct Repo {
    /// The link and the provider.
    pub(super) linked: Linked,
    /// The credentials.
    access: Access,
}

impl Repo {
    /// Opens the repository of a link with the credentials of its connection.
    pub(super) async fn open(ctx: &IntegrationContext, linked: Linked) -> Result<Self, Error> {
        let access = match linked.provider {
            VcsProvider::GitHub => {
                let mut tx = ctx.pool.begin().await?;
                let found = connections::find(&mut tx, linked.link.connection_id).await?;
                tx.commit().await?;
                let connection = found.ok_or(Error::NotFound("connection"))?.connection;
                let app = credentials::app_of(ctx, &connection).await?;
                let installation = number(&connection.external_id)?;
                let scope = TokenScope::default()
                    .repository(number(&linked.link.external_id)?)
                    .permission("contents", "read");
                let token = app.installation_token(installation, &scope).await;
                let token = token.map_err(|error| failed(&linked, error))?.token;
                Access::GitHub {
                    app,
                    installation,
                    token,
                }
            }
            VcsProvider::GitLab => {
                Access::GitLab(credentials::gitlab_token(ctx, linked.link.connection_id).await?)
            }
        };
        Ok(Self { linked, access })
    }

    /// Lists the files of `.scylla` and `.scylla/pipelines` at a commit. No folder gives no file.
    pub(super) async fn entries(
        &self,
        ctx: &IntegrationContext,
        commit: &str,
    ) -> Result<Vec<Entry>, Error> {
        let link = &self.linked.link;
        let listed = match &self.access {
            Access::GitHub { token, .. } => {
                github::entries(&ctx.clients, token, &link.full_name, commit).await
            }
            Access::GitLab(token) => {
                gitlab::entries(&ctx.clients, token, &link.external_id, commit).await
            }
        };
        listed.map_err(|error| self.failed(error))
    }

    /// Reads the bytes of a blob.
    pub(super) async fn blob(&self, ctx: &IntegrationContext, sha: &str) -> Result<Vec<u8>, Error> {
        let link = &self.linked.link;
        let read = match &self.access {
            Access::GitHub { token, .. } => {
                github::blob(&ctx.clients, token, &link.full_name, sha).await
            }
            Access::GitLab(token) => {
                gitlab::blob(&ctx.clients, token, &link.external_id, sha).await
            }
        };
        let read = read.and_then(|blob| decode(&blob.content, &blob.encoding));
        read.map_err(|error| self.failed(error))
    }

    /// Returns the page of a file at a commit on the provider.
    pub(super) fn page_of(&self, commit: &str, path: &str) -> String {
        let web = self.linked.link.web_url.trim_end_matches('/');
        match self.linked.provider {
            VcsProvider::GitHub => format!("{web}/blob/{commit}/{path}"),
            VcsProvider::GitLab => format!("{web}/-/blob/{commit}/{path}"),
        }
    }

    /// Returns the refusal of a failed call, and forgets a GitHub token that the provider
    /// refused.
    fn failed(&self, error: ProviderError) -> Error {
        if let (
            Access::GitHub {
                app, installation, ..
            },
            ProviderError::Unauthorized,
        ) = (&self.access, &error)
        {
            app.forget(*installation);
        }
        failed(&self.linked, error)
    }
}

/// A blob, as a provider gives it.
pub(super) struct Blob {
    /// The bytes, in the encoding.
    pub(super) content: String,
    /// The encoding: `base64`.
    pub(super) encoding: String,
}

/// Decodes the bytes of a blob in base64, with or without line breaks.
fn decode(content: &str, encoding: &str) -> Result<Vec<u8>, ProviderError> {
    if encoding != "base64" {
        return Err(ProviderError::Invalid(format!(
            "a blob has the encoding {encoding:?}"
        )));
    }
    let clean: String = content.split_whitespace().collect();
    base64::engine::general_purpose::STANDARD
        .decode(clean)
        .map_err(|error| ProviderError::Invalid(format!("a blob is not base64: {error}")))
}

/// Returns the refusal of a failed call to the provider of a link.
fn failed(linked: &Linked, error: ProviderError) -> Error {
    credentials::provider_failed(linked.provider, error)
}

/// Reads a GitHub id of a row.
fn number(text: &str) -> Result<u64, Error> {
    text.parse()
        .map_err(|_| Error::Internal(format!("the GitHub id {text:?} is not a number")))
}
