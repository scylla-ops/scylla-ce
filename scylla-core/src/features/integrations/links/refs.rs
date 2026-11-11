//! The branches and the tags of a linked repository, for `ListRefs`: the default branch first,
//! then the order of the provider.

use scylla_domain::integrations::VcsProvider;

use super::calls::{github, gitlab};
use super::service;
use super::store::Linked;
use crate::features::integrations::connections::credentials;
use crate::features::integrations::IntegrationContext;
use crate::infra::vcs::ProviderError;
use crate::Error;

/// A branch or a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RefKind {
    /// A branch.
    Branch,
    /// A tag.
    Tag,
}

/// A branch or a tag of a linked repository.
#[derive(Clone, Debug)]
pub(super) struct RefInfo {
    /// The name.
    pub(super) name: String,
    /// A branch or a tag.
    pub(super) kind: RefKind,
    /// The full SHA of its commit.
    pub(super) sha: String,
    /// `true` for the default branch.
    pub(super) default_branch: bool,
    /// The page of the ref on the provider.
    pub(super) web_url: String,
}

impl RefKind {
    /// Returns the name of the kind in the routes of the providers.
    fn route(self) -> &'static str {
        match self {
            Self::Branch => "branches",
            Self::Tag => "tags",
        }
    }
}

/// Returns the branches and the tags of the repository of a link with this text in their name:
/// the branches, then the tags, with the default branch first. `None` gives both kinds.
pub(super) async fn list(
    ctx: &IntegrationContext,
    linked: &Linked,
    kind: Option<RefKind>,
    search: &str,
) -> Result<Vec<RefInfo>, Error> {
    let kinds = match kind {
        Some(kind) => vec![kind],
        None => vec![RefKind::Branch, RefKind::Tag],
    };
    let refs = match linked.provider {
        VcsProvider::GitHub => of_github(ctx, linked, &kinds).await?,
        VcsProvider::GitLab => of_gitlab(ctx, linked, &kinds, search).await?,
    };
    Ok(arrange(refs, &linked.link.default_branch, search))
}

/// Returns the refs of these kinds of a GitHub repository, with a token of the installation
/// narrowed to the repository.
async fn of_github(
    ctx: &IntegrationContext,
    linked: &Linked,
    kinds: &[RefKind],
) -> Result<Vec<RefInfo>, Error> {
    // Take the App and the ids of the repository.
    let connection = service::connection(ctx, linked).await?;
    let app = credentials::app_of(ctx, &connection).await?;
    let repository = github::Repository {
        installation: number(&connection.external_id)?,
        id: number(&linked.link.external_id)?,
        full_name: &linked.link.full_name,
    };

    // Read each kind.
    let mut refs = Vec::new();
    for &kind in kinds {
        let found = github::refs(&ctx.clients, &app, &repository, kind.route()).await;
        let found = found.map_err(|error| failed(linked, error))?;
        refs.extend(found.into_iter().map(|item| RefInfo {
            web_url: format!("{}/tree/{}", linked.link.web_url, item.name),
            name: item.name,
            kind,
            sha: item.commit.sha,
            default_branch: false,
        }));
    }
    Ok(refs)
}

/// Returns the refs of these kinds of a GitLab project with this text in their name, with the
/// token of the service account.
async fn of_gitlab(
    ctx: &IntegrationContext,
    linked: &Linked,
    kinds: &[RefKind],
    search: &str,
) -> Result<Vec<RefInfo>, Error> {
    let token = credentials::gitlab_token(ctx, linked.link.connection_id).await?;
    let project = &linked.link.external_id;
    let mut refs = Vec::new();
    for &kind in kinds {
        let found = gitlab::refs(&ctx.clients, &token, project, kind.route(), search).await;
        let found = found.map_err(|error| failed(linked, error))?;
        let page = match kind {
            RefKind::Branch => "tree",
            RefKind::Tag => "tags",
        };
        refs.extend(found.into_iter().map(|item| {
            RefInfo {
                web_url: item
                    .web_url
                    .unwrap_or_else(|| format!("{}/-/{page}/{}", linked.link.web_url, item.name)),
                name: item.name,
                kind,
                sha: item.commit.id,
                default_branch: false,
            }
        }));
    }
    Ok(refs)
}

/// Keeps the refs with this text in their name, in any case, marks the default branch, and puts
/// it first.
pub(super) fn arrange(refs: Vec<RefInfo>, default_branch: &str, search: &str) -> Vec<RefInfo> {
    let text = search.to_lowercase();
    let mut refs: Vec<RefInfo> = refs
        .into_iter()
        .filter(|item| item.name.to_lowercase().contains(&text))
        .map(|mut item| {
            item.default_branch = item.kind == RefKind::Branch && item.name == default_branch;
            item
        })
        .collect();
    if let Some(index) = refs.iter().position(|item| item.default_branch) {
        let default = refs.remove(index);
        refs.insert(0, default);
    }
    refs
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
