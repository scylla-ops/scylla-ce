//! The gRPC handler of `VcsConnectionService`: authenticate, parse, authorize on the workspace,
//! call the service, answer. The routes that make a connection and the list of the repositories
//! are in `setup_grpc.rs`. No answer holds a token.

use scylla_domain::authz::Permission;
use scylla_domain::integrations::connections::{VcsConnectionHolder, VcsConnectionId};
use scylla_domain::tenancy::workspaces::{Workspace, WorkspaceId};
use scylla_extension::{AccessError, Principal};
use scylla_proto::integrations::v1 as pb;
use scylla_proto::integrations::v1::vcs_connection_service_server::{
    VcsConnectionService, VcsConnectionServiceServer,
};
use tonic::{Request, Response, Status};

use super::service::Counted;
use super::{convert, service};
use crate::api::parse::{self, ToProto};
use crate::features::integrations::IntegrationContext;
use crate::Error;

/// Makes the service.
pub(super) fn server(ctx: IntegrationContext) -> VcsConnectionServiceServer<ConnectionsGrpc> {
    VcsConnectionServiceServer::new(ConnectionsGrpc { ctx })
}

/// The handler.
pub(super) struct ConnectionsGrpc {
    /// What the parts of the integration share.
    pub(super) ctx: IntegrationContext,
}

#[tonic::async_trait]
impl VcsConnectionService for ConnectionsGrpc {
    async fn get_vcs_providers(
        &self,
        request: Request<pb::GetVcsProvidersRequest>,
    ) -> Result<Response<pb::GetVcsProvidersResponse>, Status> {
        Ok(Response::new(self.providers(request).await?))
    }

    async fn list_vcs_connections(
        &self,
        request: Request<pb::ListVcsConnectionsRequest>,
    ) -> Result<Response<pb::ListVcsConnectionsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn get_vcs_connection(
        &self,
        request: Request<pb::GetVcsConnectionRequest>,
    ) -> Result<Response<pb::GetVcsConnectionResponse>, Status> {
        Ok(Response::new(self.get(request).await?))
    }

    async fn start_git_hub_install(
        &self,
        request: Request<pb::StartGitHubInstallRequest>,
    ) -> Result<Response<pb::StartGitHubInstallResponse>, Status> {
        Ok(Response::new(self.start_install(request).await?))
    }

    async fn complete_git_hub_install(
        &self,
        request: Request<pb::CompleteGitHubInstallRequest>,
    ) -> Result<Response<pb::CompleteGitHubInstallResponse>, Status> {
        Ok(Response::new(self.complete_install(request).await?))
    }

    async fn connect_git_lab(
        &self,
        request: Request<pb::ConnectGitLabRequest>,
    ) -> Result<Response<pb::ConnectGitLabResponse>, Status> {
        Ok(Response::new(self.connect_gitlab(request).await?))
    }

    async fn replace_git_lab_token(
        &self,
        request: Request<pb::ReplaceGitLabTokenRequest>,
    ) -> Result<Response<pb::ReplaceGitLabTokenResponse>, Status> {
        Ok(Response::new(self.replace_token(request).await?))
    }

    async fn delete_vcs_connection(
        &self,
        request: Request<pb::DeleteVcsConnectionRequest>,
    ) -> Result<Response<pb::DeleteVcsConnectionResponse>, Status> {
        Ok(Response::new(self.delete(request).await?))
    }

    async fn list_connection_repositories(
        &self,
        request: Request<pb::ListConnectionRepositoriesRequest>,
    ) -> Result<Response<pb::ListConnectionRepositoriesResponse>, Status> {
        Ok(Response::new(self.repositories(request).await?))
    }
}

impl ConnectionsGrpc {
    /// Handles `GetVcsProviders`: GitHub, then GitLab, and what the workspace can connect.
    async fn providers(
        &self,
        request: Request<pb::GetVcsProvidersRequest>,
    ) -> Result<pb::GetVcsProvidersResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let workspace = parse::required(request.into_inner().workspace_id, "workspace_id")?;

        // Check the route permission, then tell each provider.
        let permission = VcsConnectionHolder::GET_VCS_PROVIDERS;
        self.authorize(&caller, permission, workspace).await?;
        let providers = service::providers(&self.ctx).await?;
        Ok(pb::GetVcsProvidersResponse {
            providers: providers.into_iter().map(convert::provider_info).collect(),
        })
    }

    /// Handles `ListVcsConnections`: one page of the connections of a workspace, oldest first.
    async fn list(
        &self,
        request: Request<pb::ListVcsConnectionsRequest>,
    ) -> Result<pb::ListVcsConnectionsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.ctx.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace = parse::required(request.workspace_id, "workspace_id")?;
        let page = parse::optional(request.page, "page")?;

        // Check the route permission, then read one page.
        let permission = VcsConnectionHolder::LIST_VCS_CONNECTIONS;
        self.authorize(&caller, permission, workspace).await?;
        let page = service::list(&self.ctx, workspace, page).await?;
        Ok(pb::ListVcsConnectionsResponse {
            connections: page.items.into_iter().map(|c| self.answer(c)).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `GetVcsConnection`.
    async fn get(
        &self,
        request: Request<pb::GetVcsConnectionRequest>,
    ) -> Result<pb::GetVcsConnectionResponse, Error> {
        let caller = self.ctx.gate.caller(&request).await?;
        let id = parse::required(request.into_inner().connection_id, "connection_id")?;
        let permission = VcsConnectionHolder::GET_VCS_CONNECTION;
        let found = self.authorize_on(&caller, permission, id).await?;
        Ok(pb::GetVcsConnectionResponse {
            connection: Some(self.answer(found)),
        })
    }

    /// Handles `DeleteVcsConnection`: the links go with it, and the GitLab webhooks first.
    async fn delete(
        &self,
        request: Request<pb::DeleteVcsConnectionRequest>,
    ) -> Result<pb::DeleteVcsConnectionResponse, Error> {
        let caller = self.ctx.gate.caller(&request).await?;
        let id = parse::required(request.into_inner().connection_id, "connection_id")?;
        let permission = VcsConnectionHolder::DELETE_VCS_CONNECTION;
        let found = self.authorize_on(&caller, permission, id).await?;
        service::delete(&self.ctx, &found).await?;
        Ok(pb::DeleteVcsConnectionResponse {})
    }

    /// Makes the proto value of a connection.
    pub(super) fn answer(&self, counted: Counted) -> pb::VcsConnection {
        convert::connection(counted, &self.ctx.settings)
    }

    /// Checks a route permission of the connections on a workspace.
    pub(super) async fn authorize(
        &self,
        caller: &Principal,
        permission: Permission<Workspace>,
        workspace: WorkspaceId,
    ) -> Result<(), Error> {
        self.ctx
            .gate
            .authorize(caller, permission.on(workspace))
            .await?;
        Ok(())
    }

    /// Checks a route permission on the workspace of a connection, and returns the connection.
    /// An unknown connection and a refusal get the same answer, which names the connection.
    pub(super) async fn authorize_on(
        &self,
        caller: &Principal,
        permission: Permission<Workspace>,
        id: VcsConnectionId,
    ) -> Result<Counted, Error> {
        // Find the connection. An unknown connection gets the answer of a refusal.
        let object = format!("vcs_connection:{id}");
        let reason = format!("{} on {object} is denied", permission.name());
        let Some(found) = service::get(&self.ctx, id).await? else {
            let error = AccessError::refused(caller, permission.name(), &object, reason);
            return Err(error.into());
        };

        // Check the permission on its workspace, and hide the workspace in a refusal.
        let check = permission.on(found.connection.workspace_id);
        match self.ctx.gate.authorize(caller, check).await {
            Ok(()) => Ok(found),
            Err(AccessError::Denied(_)) => Err(AccessError::Denied(reason).into()),
            Err(error) => Err(error.into()),
        }
    }
}
