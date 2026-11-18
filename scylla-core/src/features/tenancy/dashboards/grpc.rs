//! The gRPC handler of `DashboardService`: authenticate, parse, authorize, call the service,
//! answer.

use std::sync::Arc;

use scylla_domain::tenancy::dashboards::{
    DashboardChange, DashboardHolder, DashboardId, ListDashboards, NewDashboard, Visibility,
};
use scylla_domain::tenancy::workspaces::WorkspaceId;
use scylla_extension::{Gate, Hooks};
use scylla_proto::dashboards::v1 as pb;
use scylla_proto::dashboards::v1::dashboard_service_server::{
    DashboardService, DashboardServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{access, convert, service};
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    hooks: Arc<Hooks>,
    gate: Arc<dyn Gate>,
) -> DashboardServiceServer<DashboardsGrpc> {
    DashboardServiceServer::new(DashboardsGrpc { pool, hooks, gate })
}

/// The handler.
pub(super) struct DashboardsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The hooks of the edition.
    hooks: Arc<Hooks>,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl DashboardService for DashboardsGrpc {
    async fn list_dashboards(
        &self,
        request: Request<pb::ListDashboardsRequest>,
    ) -> Result<Response<pb::ListDashboardsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn get_dashboard(
        &self,
        request: Request<pb::GetDashboardRequest>,
    ) -> Result<Response<pb::GetDashboardResponse>, Status> {
        Ok(Response::new(self.get(request).await?))
    }

    async fn create_dashboard(
        &self,
        request: Request<pb::CreateDashboardRequest>,
    ) -> Result<Response<pb::CreateDashboardResponse>, Status> {
        Ok(Response::new(self.create(request).await?))
    }

    async fn update_dashboard(
        &self,
        request: Request<pb::UpdateDashboardRequest>,
    ) -> Result<Response<pb::UpdateDashboardResponse>, Status> {
        Ok(Response::new(self.update(request).await?))
    }

    async fn delete_dashboard(
        &self,
        request: Request<pb::DeleteDashboardRequest>,
    ) -> Result<Response<pb::DeleteDashboardResponse>, Status> {
        Ok(Response::new(self.delete(request).await?))
    }

    async fn set_team_default_dashboard(
        &self,
        request: Request<pb::SetTeamDefaultDashboardRequest>,
    ) -> Result<Response<pb::SetTeamDefaultDashboardResponse>, Status> {
        Ok(Response::new(self.set_team_default(request).await?))
    }
}

impl DashboardsGrpc {
    /// Handles `ListDashboards`: the shared dashboards of a workspace, and the private
    /// dashboards of the caller.
    async fn list(
        &self,
        request: Request<pb::ListDashboardsRequest>,
    ) -> Result<pb::ListDashboardsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let query = ListDashboards {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            caller: caller.actor.id,
            page: parse::optional(request.page, "page")?,
        };

        // Check the route permission on the workspace, then read one page.
        let check = DashboardHolder::LIST_DASHBOARDS.on(query.workspace_id);
        self.gate.authorize(&caller, check).await?;
        let page = service::list(&self.pool, query).await?;
        Ok(pb::ListDashboardsResponse {
            dashboards: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `GetDashboard`: one dashboard with its widgets.
    async fn get(
        &self,
        request: Request<pb::GetDashboardRequest>,
    ) -> Result<pb::GetDashboardResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let id: DashboardId = parse::required(request.into_inner().dashboard_id, "dashboard_id")?;

        // Check the route on the dashboard. An unknown dashboard gets the answer of a refusal.
        let found = service::get(&self.pool, id).await?;
        let dashboard = found.as_ref().map(|(dashboard, _)| dashboard);
        access::authorize(self.gate.as_ref(), &caller, access::GET, id, dashboard).await?;
        let (dashboard, widgets) = found.ok_or(Error::NotFound("dashboard"))?;
        Ok(pb::GetDashboardResponse {
            dashboard: Some(dashboard.to_proto()),
            widgets: Some(widgets.to_proto()),
        })
    }

    /// Handles `CreateDashboard`, in a workspace, with the caller as the creator.
    async fn create(
        &self,
        request: Request<pb::CreateDashboardRequest>,
    ) -> Result<pb::CreateDashboardResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let widgets = request
            .widgets
            .ok_or_else(|| Error::invalid("widgets", "is required"))?;
        let new = NewDashboard {
            workspace_id: parse::required(request.workspace_id, "workspace_id")?,
            created_by: caller.actor.id,
            name: parse::field(request.name, "name")?,
            visibility: convert::visibility(request.visibility)?.unwrap_or(Visibility::Private),
            widgets: convert::widgets(widgets)?,
        };

        // Check the route permission on the workspace, then create the dashboard.
        let check = DashboardHolder::CREATE_DASHBOARD.on(new.workspace_id);
        self.gate.authorize(&caller, check).await?;
        let dashboard = service::create(&self.pool, &self.hooks, caller.actor, new).await?;
        Ok(pb::CreateDashboardResponse {
            dashboard: Some(dashboard.to_proto()),
        })
    }

    /// Handles `UpdateDashboard`: a new name, visibility or widgets, at the version that the
    /// client read.
    async fn update(
        &self,
        request: Request<pb::UpdateDashboardRequest>,
    ) -> Result<pb::UpdateDashboardResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: DashboardId = parse::required(request.dashboard_id, "dashboard_id")?;
        let change = DashboardChange {
            version: request.version,
            name: request
                .name
                .map(|name| parse::field(name, "name"))
                .transpose()?,
            visibility: convert::visibility(request.visibility)?,
            widgets: request.widgets.map(convert::widgets).transpose()?,
        };

        // Check the route on the dashboard. An unknown dashboard gets the answer of a refusal.
        let found = service::find(&self.pool, id).await?;
        access::authorize(
            self.gate.as_ref(),
            &caller,
            access::UPDATE,
            id,
            found.as_ref(),
        )
        .await?;

        // Change the dashboard.
        let dashboard = service::update(&self.pool, caller.actor.id, id, change).await?;
        Ok(pb::UpdateDashboardResponse {
            dashboard: Some(dashboard.to_proto()),
        })
    }

    /// Handles `DeleteDashboard`.
    async fn delete(
        &self,
        request: Request<pb::DeleteDashboardRequest>,
    ) -> Result<pb::DeleteDashboardResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let id: DashboardId = parse::required(request.into_inner().dashboard_id, "dashboard_id")?;

        // Check the route on the dashboard. An unknown dashboard gets the answer of a refusal.
        let found = service::find(&self.pool, id).await?;
        access::authorize(
            self.gate.as_ref(),
            &caller,
            access::DELETE,
            id,
            found.as_ref(),
        )
        .await?;

        // Delete the dashboard.
        service::delete(&self.pool, &self.hooks, caller.actor, id).await?;
        Ok(pb::DeleteDashboardResponse {})
    }

    /// Handles `SetTeamDefaultDashboard`: a shared dashboard becomes the team default of its
    /// workspace, or the workspace has no team default.
    async fn set_team_default(
        &self,
        request: Request<pb::SetTeamDefaultDashboardRequest>,
    ) -> Result<pb::SetTeamDefaultDashboardResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let workspace: WorkspaceId = parse::required(request.workspace_id, "workspace_id")?;
        let id: Option<DashboardId> = request
            .dashboard_id
            .map(|id| parse::field(id, "dashboard_id"))
            .transpose()?;

        // Check the route permission on the workspace, then set the team default.
        let check = DashboardHolder::SET_TEAM_DEFAULT_DASHBOARD.on(workspace);
        self.gate.authorize(&caller, check).await?;
        let dashboard = service::set_team_default(&self.pool, workspace, id).await?;
        Ok(pb::SetTeamDefaultDashboardResponse {
            dashboard: dashboard.map(ToProto::to_proto),
        })
    }
}
