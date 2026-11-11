//! The gRPC routes of `VcsConnectionService` that make a connection (the install of the GitHub
//! App, the token of GitLab) and that list the repositories of a connection.

use scylla_domain::integrations::connections::VcsConnectionHolder;
use scylla_domain::integrations::links::RepositoryAccess;
use scylla_domain::projects::ProjectId;
use scylla_domain::PageSize;
use scylla_extension::AccessError;
use scylla_proto::common::v1::PageResponse;
use scylla_proto::integrations::v1 as pb;
use tonic::Request;

use super::grpc::ConnectionsGrpc;
use super::install::{self, Redirect};
use super::{convert, repositories, service, tokens};
use crate::api::parse;
use crate::infra::vcs::Hidden;
use crate::Error;

/// The longest token, in characters.
const MAX_TOKEN: usize = 1024;

/// The longest search text, in characters.
const MAX_SEARCH: usize = 255;

impl ConnectionsGrpc {
    /// Handles `StartGitHubInstall`: the install page with a new state.
    pub(super) async fn start_install(
        &self,
        request: Request<pb::StartGitHubInstallRequest>,
    ) -> Result<pb::StartGitHubInstallResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let workspace = parse::required(request.into_inner().workspace_id, "workspace_id")?;

        // Check the route permission, then make the state.
        let permission = VcsConnectionHolder::START_GIT_HUB_INSTALL;
        self.authorize(&caller, permission, workspace).await?;
        let (url, expires_at) = install::start(&self.ctx, workspace, caller.actor.id).await?;
        Ok(pb::StartGitHubInstallResponse {
            install_url: url,
            expires_at: Some(parse::timestamp(expires_at)),
        })
    }

    /// Handles `CompleteGitHubInstall`, on the workspace of the state.
    pub(super) async fn complete_install(
        &self,
        request: Request<pb::CompleteGitHubInstallRequest>,
    ) -> Result<pb::CompleteGitHubInstallResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        if request.state.is_empty() {
            return Err(Error::invalid("state", "is required"));
        }
        let installation = match request.installation_id.trim() {
            "" => None,
            id => Some(
                id.parse::<u64>()
                    .map_err(|_| Error::invalid("installation_id", "must be a number"))?,
            ),
        };
        let install = installation.is_some() && request.setup_action != "request";
        if install && request.code.is_empty() {
            return Err(Error::invalid("code", "is required"));
        }

        // Find the workspace of the state, and check the route permission there.
        let actor = caller.actor.id;
        let workspace = install::workspace_of(&self.ctx, &request.state, actor).await?;
        let permission = VcsConnectionHolder::COMPLETE_GIT_HUB_INSTALL;
        self.authorize(&caller, permission, workspace).await?;

        // Connect the installation.
        let redirect = Redirect {
            installation,
            code: request.code,
            setup_action: request.setup_action,
        };
        let counted = install::complete(&self.ctx, &request.state, actor, redirect).await?;
        Ok(pb::CompleteGitHubInstallResponse {
            connection: Some(self.answer(counted)),
        })
    }

    /// Handles `ConnectGitLab`: a new connection with the token of a service account.
    pub(super) async fn connect_gitlab(
        &self,
        request: Request<pb::ConnectGitLabRequest>,
    ) -> Result<pb::ConnectGitLabResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace = parse::required(request.workspace_id, "workspace_id")?;
        let token = token(request.token)?;

        // Check the route permission, then check and seal the token.
        let permission = VcsConnectionHolder::CONNECT_GIT_LAB;
        self.authorize(&caller, permission, workspace).await?;
        let (counted, warnings) =
            tokens::connect(&self.ctx, workspace, caller.actor.id, &token).await?;
        Ok(pb::ConnectGitLabResponse {
            connection: Some(self.answer(counted)),
            warnings: warnings.into_iter().map(str::to_owned).collect(),
        })
    }

    /// Handles `ReplaceGitLabToken`: a new token of the same account.
    pub(super) async fn replace_token(
        &self,
        request: Request<pb::ReplaceGitLabTokenRequest>,
    ) -> Result<pb::ReplaceGitLabTokenResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let id = parse::required(request.connection_id, "connection_id")?;
        let token = token(request.token)?;

        // Check the route permission, then check and seal the token.
        let permission = VcsConnectionHolder::REPLACE_GIT_LAB_TOKEN;
        let found = self.authorize_on(&caller, permission, id).await?;
        let (counted, warnings) = tokens::replace(&self.ctx, &found.connection, &token).await?;
        Ok(pb::ReplaceGitLabTokenResponse {
            connection: Some(self.answer(counted)),
            warnings: warnings.into_iter().map(str::to_owned).collect(),
        })
    }

    /// Handles `ListConnectionRepositories`, for the link of a project: one page of the
    /// repositories of a connection of the workspace of the project.
    pub(super) async fn repositories(
        &self,
        request: Request<pb::ListConnectionRepositoriesRequest>,
    ) -> Result<pb::ListConnectionRepositoriesResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let project: ProjectId = parse::required(request.project_id, "project_id")?;
        let id = parse::required(request.connection_id, "connection_id")?;
        let search = request.search.trim().to_owned();
        if search.chars().count() > MAX_SEARCH {
            return Err(Error::invalid("search", "must have at most 255 characters"));
        }
        let (offset, size) = page(request.page.unwrap_or_default())?;

        // Check the route permission on the project. An unknown project gets a refusal.
        let check = RepositoryAccess::LIST_CONNECTION_REPOSITORIES.on(project);
        let Some(workspace) = service::project_workspace(&self.ctx, project).await? else {
            return Err(AccessError::denied(&caller, &check).into());
        };
        self.ctx
            .gate
            .authorize(&caller, check.within(workspace))
            .await?;

        // Refuse a connection of another workspace as an unknown one.
        let found = service::get(&self.ctx, id).await?;
        let Some(found) = found.filter(|found| found.connection.workspace_id == workspace) else {
            let permission = RepositoryAccess::LIST_CONNECTION_REPOSITORIES.name();
            let object = format!("vcs_connection:{id}");
            let reason = format!("{permission} on {object} is denied");
            return Err(AccessError::refused(&caller, permission, &object, reason).into());
        };

        // Read the repositories, and give one page.
        let seen = repositories::list(&self.ctx, &found.connection, &search).await?;
        let next = (seen.len() > offset + size).then(|| (offset + size).to_string());
        let items = seen.into_iter().skip(offset).take(size);
        Ok(pb::ListConnectionRepositoriesResponse {
            repositories: items.map(convert::repository).collect(),
            page: Some(PageResponse {
                next_token: next.unwrap_or_default(),
            }),
        })
    }
}

/// Reads a token of a request: required, at most 1024 characters, with no space around it.
fn token(text: String) -> Result<Hidden, Error> {
    let text = text.trim();
    if text.is_empty() {
        return Err(Error::invalid("token", "is required"));
    }
    if text.chars().count() > MAX_TOKEN || text.chars().any(char::is_whitespace) {
        return Err(Error::invalid("token", "is not a valid token"));
    }
    Ok(Hidden::new(text))
}

/// Reads the page of a list of repositories: the token is the number of items before the page.
fn page(page: scylla_proto::common::v1::PageRequest) -> Result<(usize, usize), Error> {
    let size = PageSize::parse(page.size)
        .map_err(|error| Error::invalid("page.size", error.to_string()))?;
    let offset = match page.token.as_str() {
        "" => 0,
        token => token
            .parse()
            .map_err(|_| Error::invalid("page.token", "is not a token of this list"))?,
    };
    Ok((offset, size.get()))
}
