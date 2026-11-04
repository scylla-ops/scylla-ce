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

        // Check the route permission on the workspace, then create the App.
        self.gate
            .authorize(&caller, AppHolder::CREATE_APP.on(new.workspace_id))
            .await?;
        let (app, code) = service::create(&self.pool, new).await?;
        Ok(pb::CreateAppResponse {
            app: Some(convert::app(app, false)),
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
            app: Some(convert::app(app, self.registry.is_connected(id))),
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
        let apps = page.items.into_iter().map(|app| {
            let connected = self.registry.is_connected(app.id);
            convert::app(app, connected)
        });
        Ok(pb::ListAppsResponse {
            apps: apps.collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `DeleteApp`: the App goes with its keys, its roles and its stream.
    async fn delete(
        &self,
        request: Request<pb::DeleteAppRequest>,
    ) -> Result<pb::DeleteAppResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::DELETE_APP, id).await?;
        let gate = self.gate.as_ref();
        service::delete(&self.pool, gate, &self.registry, id).await?;
        Ok(pb::DeleteAppResponse {})
    }

    /// Handles `DisableApp`: its tokens, its stream and its jobs stop.
    async fn disable(
        &self,
        request: Request<pb::DisableAppRequest>,
    ) -> Result<pb::DisableAppResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::DISABLE_APP, id).await?;
        let app = service::disable(&self.pool, &self.registry, id).await?;
        Ok(pb::DisableAppResponse {
            app: Some(convert::app(app, false)),
        })
    }

    /// Handles `EnableApp`: the App gets tokens again.
    async fn enable(
        &self,
        request: Request<pb::EnableAppRequest>,
    ) -> Result<pb::EnableAppResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::ENABLE_APP, id).await?;
        let app = service::enable(&self.pool, id).await?;
        Ok(pb::EnableAppResponse {
            app: Some(convert::app(app, self.registry.is_connected(id))),
        })
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
