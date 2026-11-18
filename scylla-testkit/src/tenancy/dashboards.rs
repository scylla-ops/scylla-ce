//! The `TestApp` helpers of `DashboardService`: the saved layouts and the open items.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::dashboards::v1::dashboard_service_client::DashboardServiceClient;
use scylla_proto::dashboards::v1::{
    CreateDashboardRequest, Dashboard, DashboardVisibility, DashboardWidgets,
    DeleteDashboardRequest, GetDashboardRequest, GetDashboardResponse, ListAttentionItemsRequest,
    ListAttentionItemsResponse, ListDashboardsRequest, ListDashboardsResponse,
    SetTeamDefaultDashboardRequest, UpdateDashboardRequest,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::rpc::page;
use crate::{id, TestApp};

/// Makes the widgets of a request: version 1 of the format and a JSON text.
pub fn dashboard_widgets(json: &str) -> DashboardWidgets {
    DashboardWidgets {
        schema_version: 1,
        widgets_json: json.to_owned(),
    }
}

impl TestApp {
    /// A client of `DashboardService`.
    fn dashboards(&self) -> DashboardServiceClient<Channel> {
        DashboardServiceClient::new(self.channel.clone())
    }

    /// Calls `CreateDashboard` as `actor` with a name, a visibility and no widget.
    pub async fn create_dashboard(
        &self,
        actor: ActorId,
        workspace: &str,
        name: &str,
        visibility: DashboardVisibility,
    ) -> Result<Dashboard, Status> {
        let request = CreateDashboardRequest {
            workspace_id: Some(id(workspace)),
            name: name.to_owned(),
            visibility: visibility.into(),
            widgets: Some(dashboard_widgets("[]")),
        };
        self.create_dashboard_with(actor, request).await
    }

    /// Calls `CreateDashboard` as `actor` with a request, so a test can send a bad value.
    pub async fn create_dashboard_with(
        &self,
        actor: ActorId,
        request: CreateDashboardRequest,
    ) -> Result<Dashboard, Status> {
        let request = self.authed(actor, request).await;
        let response = self.dashboards().create_dashboard(request).await?;
        Ok(response
            .into_inner()
            .dashboard
            .expect("dashboard in response"))
    }

    /// Calls `GetDashboard` as `actor`. The id is text, so a test can send a bad id.
    pub async fn get_dashboard(
        &self,
        actor: ActorId,
        dashboard: &str,
    ) -> Result<GetDashboardResponse, Status> {
        let request = GetDashboardRequest {
            dashboard_id: Some(id(dashboard)),
        };
        let request = self.authed(actor, request).await;
        Ok(self.dashboards().get_dashboard(request).await?.into_inner())
    }

    /// Calls `ListDashboards` as `actor`, for one page of `size` dashboards after `token`.
    pub async fn list_dashboards_page(
        &self,
        actor: ActorId,
        workspace: &str,
        size: i32,
        token: &str,
    ) -> Result<ListDashboardsResponse, Status> {
        let request = ListDashboardsRequest {
            workspace_id: Some(id(workspace)),
            page: Some(page(size, token)),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .dashboards()
            .list_dashboards(request)
            .await?
            .into_inner())
    }

    /// Calls `ListDashboards` as `actor`, and returns the names of the first page.
    pub async fn dashboard_names(
        &self,
        actor: ActorId,
        workspace: &str,
    ) -> Result<Vec<String>, Status> {
        let response = self.list_dashboards_page(actor, workspace, 100, "").await?;
        Ok(response
            .dashboards
            .into_iter()
            .map(|dashboard| dashboard.name)
            .collect())
    }

    /// Calls `UpdateDashboard` as `actor` with a request.
    pub async fn update_dashboard(
        &self,
        actor: ActorId,
        request: UpdateDashboardRequest,
    ) -> Result<Dashboard, Status> {
        let request = self.authed(actor, request).await;
        let response = self.dashboards().update_dashboard(request).await?;
        Ok(response
            .into_inner()
            .dashboard
            .expect("dashboard in response"))
    }

    /// Calls `UpdateDashboard` as `actor` with a new name only, at a version.
    pub async fn rename_dashboard(
        &self,
        actor: ActorId,
        dashboard: &str,
        version: i64,
        name: &str,
    ) -> Result<Dashboard, Status> {
        let request = UpdateDashboardRequest {
            dashboard_id: Some(id(dashboard)),
            version,
            name: Some(name.to_owned()),
            visibility: DashboardVisibility::Unspecified.into(),
            widgets: None,
        };
        self.update_dashboard(actor, request).await
    }

    /// Calls `DeleteDashboard` as `actor`.
    pub async fn delete_dashboard(&self, actor: ActorId, dashboard: &str) -> Result<(), Status> {
        let request = DeleteDashboardRequest {
            dashboard_id: Some(id(dashboard)),
        };
        let request = self.authed(actor, request).await;
        self.dashboards().delete_dashboard(request).await?;
        Ok(())
    }

    /// Calls `SetTeamDefaultDashboard` as `actor`. `None` removes the team default.
    pub async fn set_team_default_dashboard(
        &self,
        actor: ActorId,
        workspace: &str,
        dashboard: Option<&str>,
    ) -> Result<Option<Dashboard>, Status> {
        let request = SetTeamDefaultDashboardRequest {
            workspace_id: Some(id(workspace)),
            dashboard_id: dashboard.map(id),
        };
        let request = self.authed(actor, request).await;
        let response = self
            .dashboards()
            .set_team_default_dashboard(request)
            .await?;
        Ok(response.into_inner().dashboard)
    }

    /// Calls `ListAttentionItems` as `actor`.
    pub async fn list_attention_items(
        &self,
        actor: ActorId,
        request: ListAttentionItemsRequest,
    ) -> Result<ListAttentionItemsResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self.dashboards().list_attention_items(request).await?;
        Ok(response.into_inner())
    }
}
