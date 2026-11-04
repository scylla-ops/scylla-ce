//! The gRPC handler of `AppService`: create, read, list and delete the Apps. The keys and the
//! join codes are in `keys.rs`.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::identity::apps::{App, AppHolder, AppId, ListApps, NewApp};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{AccessError, Gate, Principal};
use scylla_proto::apps::v1 as pb;
use scylla_proto::apps::v1::app_service_server::{AppService, AppServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{convert, service};
use crate::api::parse::{self, ToProto};
use crate::features::identity::agents::Registry;
use crate::features::projects::jobs::runs;
use crate::Error;

/// Makes the service.
pub(crate) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    registry: Arc<Registry>,
) -> AppServiceServer<AppsGrpc> {
    AppServiceServer::new(AppsGrpc {
        pool,
        gate,
        registry,
    })
}

/// The handler.
pub(crate) struct AppsGrpc {
    /// The database pool.
    pub(super) pool: PgPool,
    /// The access gate.
    pub(super) gate: Arc<dyn Gate>,
    /// The open streams of the agents, for their presence.
    pub(super) registry: Arc<Registry>,
}

#[tonic::async_trait]
impl AppService for AppsGrpc {
    async fn create_app(
        &self,
        request: Request<pb::CreateAppRequest>,
    ) -> Result<Response<pb::CreateAppResponse>, Status> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn get_app(
        &self,
        request: Request<pb::GetAppRequest>,
    ) -> Result<Response<pb::GetAppResponse>, Status> {
        Ok(Response::new(self.get(request).await?))
    }

    async fn list_apps(
        &self,
        request: Request<pb::ListAppsRequest>,
    ) -> Result<Response<pb::ListAppsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn delete_app(
        &self,
        request: Request<pb::DeleteAppRequest>,
    ) -> Result<Response<pb::DeleteAppResponse>, Status> {
        Ok(Response::new(self.delete(request).await?))
    }

    async fn disable_app(
        &self,
        request: Request<pb::DisableAppRequest>,
    ) -> Result<Response<pb::DisableAppResponse>, Status> {
        Ok(Response::new(self.disable(request).await?))
    }

    async fn enable_app(
        &self,
        request: Request<pb::EnableAppRequest>,
    ) -> Result<Response<pb::EnableAppResponse>, Status> {
        Ok(Response::new(self.enable(request).await?))
    }

    async fn create_join_code(
        &self,
        request: Request<pb::CreateJoinCodeRequest>,
    ) -> Result<Response<pb::CreateJoinCodeResponse>, Status> {
        Ok(Response::new(self.join_code(request).await?))
    }

    async fn add_app_key(
        &self,
        request: Request<pb::AddAppKeyRequest>,
    ) -> Result<Response<pb::AddAppKeyResponse>, Status> {
        Ok(Response::new(self.add_key(request).await?))
    }

    async fn revoke_app_key(
        &self,
        request: Request<pb::RevokeAppKeyRequest>,
    ) -> Result<Response<pb::RevokeAppKeyResponse>, Status> {
        Ok(Response::new(self.revoke_key(request).await?))
    }

    async fn rename_app_key(
        &self,
        request: Request<pb::RenameAppKeyRequest>,
    ) -> Result<Response<pb::RenameAppKeyResponse>, Status> {
        Ok(Response::new(self.rename_key(request).await?))
    }

    async fn create_app_token(
        &self,
        request: Request<pb::CreateAppTokenRequest>,
    ) -> Result<Response<pb::CreateAppTokenResponse>, Status> {
        Ok(Response::new(self.create_token(request).await?))
    }

    async fn list_app_tokens(
        &self,
        request: Request<pb::ListAppTokensRequest>,
    ) -> Result<Response<pb::ListAppTokensResponse>, Status> {
        Ok(Response::new(self.list_tokens(request).await?))
    }

    async fn revoke_app_token(
        &self,
        request: Request<pb::RevokeAppTokenRequest>,
    ) -> Result<Response<pb::RevokeAppTokenResponse>, Status> {
        Ok(Response::new(self.revoke_token(request).await?))
    }

    async fn add_ci_identity(
        &self,
        request: Request<pb::AddCiIdentityRequest>,
    ) -> Result<Response<pb::AddCiIdentityResponse>, Status> {
        Ok(Response::new(self.add_ci(request).await?))
    }

    async fn list_ci_identities(
        &self,
        request: Request<pb::ListCiIdentitiesRequest>,
    ) -> Result<Response<pb::ListCiIdentitiesResponse>, Status> {
        Ok(Response::new(self.list_ci(request).await?))
    }

    async fn remove_ci_identity(
        &self,
        request: Request<pb::RemoveCiIdentityRequest>,
    ) -> Result<Response<pb::RemoveCiIdentityResponse>, Status> {
        Ok(Response::new(self.remove_ci(request).await?))
    }
}

impl AppsGrpc {
    /// Handles `CreateApp`, in a workspace.
    async fn create(
        &self,
        request: Request<pb::CreateAppRequest>,
    ) -> Result<pb::CreateAppResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let new = NewApp {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            created_by: caller.actor.id,
            name: parse::field(request.name, "name")?,
            runner: request.runner,
        };

        // Check the route permission on the workspace, create the App, and name its creator.
        self.gate
            .authorize(&caller, AppHolder::CREATE_APP.on(new.workspace_id))
            .await?;
        let (app, code) = service::create(&self.pool, new).await?;
        let creator = App::CREATOR.of(app.id, caller.actor.id);
        self.gate.write(&[creator]).await?;
        Ok(pb::CreateAppResponse {
            app: Some(convert::app(app, false, 0)),
            join_code: code.code,
            join_code_expires_at: Some(parse::timestamp(code.expires_at)),
        })
    }

    /// Handles `GetApp`: the App and its keys.
    async fn get(&self, request: Request<pb::GetAppRequest>) -> Result<pb::GetAppResponse, Error> {
        // Authenticate the caller, read the request, and check the route permission.
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::GET_APP, id).await?;

        // Read the App and its keys. A delete after the check gives `NOT_FOUND`.
        let app = service::get(&self.pool, id)
            .await?
            .ok_or(Error::NotFound("app"))?;
        let keys = service::keys(&self.pool, id).await?;
        Ok(pb::GetAppResponse {
            app: self.messages(vec![app]).await?.pop(),
            keys: keys.into_iter().map(ToProto::to_proto).collect(),
        })
    }

    /// Handles `ListApps`: one page of the Apps of a workspace.
    async fn list(
        &self,
        request: Request<pb::ListAppsRequest>,
    ) -> Result<pb::ListAppsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListApps {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            runner: request.runner,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the workspace, then read one page.
        self.gate
            .authorize(&caller, AppHolder::LIST_APPS.on(query.workspace_id))
            .await?;
        let page = service::list(&self.pool, query).await?;
        Ok(pb::ListAppsResponse {
            apps: self.messages(page.items).await?,
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Makes the messages of Apps, with their presence on this server and the number of jobs
    /// that each holds.
    pub(super) async fn messages(&self, apps: Vec<App>) -> Result<Vec<pb::App>, Error> {
        let ids: Vec<AppId> = apps.iter().map(|app| app.id).collect();
        let held = runs::held_counts(&self.pool, &ids).await?;
        let messages = apps.into_iter().map(|app| {
            let connected = self.registry.is_connected(app.id);
            let running_jobs = held.get(&app.id).copied().unwrap_or(0);
            convert::app(app, connected, running_jobs)
        });
        Ok(messages.collect())
    }

    /// Checks a route permission on an App and on its workspace, and returns the workspace. An
    /// unknown App gets the answer of a refusal.
    pub(super) async fn authorize(
        &self,
        caller: &Principal,
        permission: Permission<App>,
        id: AppId,
    ) -> Result<WorkspaceId, Error> {
        let check = permission.on(id);
        let Some(workspace) = service::workspace(&self.pool, id).await? else {
            return Err(AccessError::denied(caller, &check).into());
        };
        self.gate.authorize(caller, check.within(workspace)).await?;
        Ok(workspace)
    }
}
