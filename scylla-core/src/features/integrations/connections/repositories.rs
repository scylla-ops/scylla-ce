//! The repositories that a connection sees, for the link of a project: the repositories of a
//! GitHub installation, or the GitLab projects where the service account is Developer or more.

use std::collections::HashMap;

use scylla_domain::integrations::connections::VcsConnection;
use scylla_domain::integrations::links::GitLabRole;
use scylla_domain::integrations::VcsProvider;
use scylla_domain::projects::ProjectId;

use super::calls::{github, gitlab};
use super::checks;
use super::credentials;
use super::store;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::ProviderError;
use crate::Error;

/// A repository that a connection sees.
pub(super) struct Seen {
    /// The id of the repository on the provider.
    pub(super) external_id: String,
    /// The full name.
    pub(super) full_name: String,
    /// The page of the repository.
    pub(super) web_url: String,
    /// The default branch, empty for an empty repository.
    pub(super) default_branch: String,
    /// `true` for a private or internal repository.
    pub(super) private: bool,
    /// The role of the service account on a GitLab project.
    pub(super) role: Option<GitLabRole>,
    /// The project of the workspace that links the repository.
    pub(super) linked: Option<ProjectId>,
}

/// Returns the repositories of a connection with this text in their full name, in the order of
/// the provider, each with the project that links it.
pub(super) async fn list(
    ctx: &IntegrationContext,
    connection: &VcsConnection,
    search: &str,
) -> Result<Vec<Seen>, Error> {
    // Read the repositories on the provider.
    let failed = |error: ProviderError| credentials::provider_failed(connection.provider, error);
    let mut seen: Vec<Seen> = match connection.provider {
        VcsProvider::GitHub => {
            let app = credentials::app_of(ctx, connection).await?;
            let installation = connection
                .external_id
                .parse()
                .map_err(|_| Error::Internal("an installation id is not a number".to_owned()))?;
            let found = github::repositories(&ctx.clients, &app, installation).await;
            found.map_err(failed)?.into_iter().map(of_github).collect()
        }
        VcsProvider::GitLab => {
            let token = credentials::gitlab_token(ctx, connection.id).await?;
            let found = gitlab::projects(&ctx.clients, &token, search).await;
            found.map_err(failed)?.into_iter().map(of_gitlab).collect()
        }
    };

    // Keep the repositories with the text, and add the projects that link them.
    let text = search.to_lowercase();
    seen.retain(|repository| repository.full_name.to_lowercase().contains(&text));
    let mut tx = ctx.pool.begin().await?;
    let links: HashMap<String, ProjectId> = store::links(&mut tx, connection.id)
        .await?
        .into_iter()
        .collect();
    tx.commit().await?;
    for repository in &mut seen {
        repository.linked = links.get(&repository.external_id).copied();
    }
    Ok(seen)
}

/// Makes the repository of a GitHub repository.
fn of_github(repository: github::GitHubRepository) -> Seen {
    Seen {
        external_id: repository.id.to_string(),
        full_name: repository.full_name,
        web_url: repository.html_url,
        default_branch: repository.default_branch.unwrap_or_default(),
        private: repository.private,
        role: None,
        linked: None,
    }
}

/// Makes the repository of a GitLab project.
fn of_gitlab(project: gitlab::GitLabProject) -> Seen {
    Seen {
        external_id: project.id.to_string(),
        role: checks::gitlab_role(project.level()),
        private: project.private(),
        full_name: project.path_with_namespace,
        web_url: project.web_url,
        default_branch: project.default_branch.unwrap_or_default(),
        linked: None,
    }
}
