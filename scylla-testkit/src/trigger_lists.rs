//! The `TestApp` helpers of the lists of `TriggerService` for the dashboard: the triggers of a
//! workspace, their fires, and the counts of their fires.

use scylla_domain::identity::actors::ActorId;
use scylla_proto::pipelines::v1::{
    GetTriggerFireStatsRequest, GetTriggerFireStatsResponse, ListTriggerFiresRequest,
    ListTriggerFiresResponse, ListWorkspaceTriggersRequest, ListWorkspaceTriggersResponse,
};
use tonic::Status;

use crate::TestApp;

impl TestApp {
    /// Calls `ListWorkspaceTriggers` as `actor`.
    pub async fn list_workspace_triggers(
        &self,
        actor: ActorId,
        request: ListWorkspaceTriggersRequest,
    ) -> Result<ListWorkspaceTriggersResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self.triggers().list_workspace_triggers(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `ListTriggerFires` as `actor`.
    pub async fn list_trigger_fires(
        &self,
        actor: ActorId,
        request: ListTriggerFiresRequest,
    ) -> Result<ListTriggerFiresResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self.triggers().list_trigger_fires(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `GetTriggerFireStats` as `actor`.
    pub async fn get_trigger_fire_stats(
        &self,
        actor: ActorId,
        request: GetTriggerFireStatsRequest,
    ) -> Result<GetTriggerFireStatsResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self.triggers().get_trigger_fire_stats(request).await?;
        Ok(response.into_inner())
    }
}
