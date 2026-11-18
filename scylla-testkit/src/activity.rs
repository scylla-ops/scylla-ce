//! The `TestApp` helpers of `ActivityService`.

use std::time::Duration;

use scylla_domain::identity::actors::ActorId;
use scylla_proto::activity::v1::activity_service_client::ActivityServiceClient;
use scylla_proto::activity::v1::{Activity, ListActivityRequest, ListActivityResponse};
use tonic::transport::Channel;
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Calls `ListActivity` as `actor`.
    pub async fn list_activity(
        &self,
        actor: ActorId,
        request: ListActivityRequest,
    ) -> Result<ListActivityResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self.activity().list_activity(request).await?;
        Ok(response.into_inner())
    }

    /// Calls `ListActivity` as `actor` for the first page of a workspace, and returns its rows,
    /// newest first.
    pub async fn activity_of(&self, actor: ActorId, workspace: &str) -> Vec<Activity> {
        let request = ListActivityRequest {
            workspace_id: Some(id(workspace)),
            ..ListActivityRequest::default()
        };
        let response = self.list_activity(actor, request).await;
        response.expect("a list of the activity").activities
    }

    /// Reads the first page of a workspace as `actor` until it has `count` rows or more, for 5
    /// seconds at most, and returns its rows.
    pub async fn wait_for_activity(
        &self,
        actor: ActorId,
        workspace: &str,
        count: usize,
    ) -> Vec<Activity> {
        let mut rows = self.activity_of(actor, workspace).await;
        for _ in 0..50 {
            if rows.len() >= count {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
            rows = self.activity_of(actor, workspace).await;
        }
        rows
    }

    /// Returns the kinds of the rows of the first page of a workspace that `actor` reads, newest
    /// first.
    pub async fn activity_kinds(&self, actor: ActorId, workspace: &str) -> Vec<String> {
        let rows = self.activity_of(actor, workspace).await;
        rows.into_iter().map(|row| row.kind).collect()
    }

    /// Returns a client of `ActivityService`.
    fn activity(&self) -> ActivityServiceClient<Channel> {
        ActivityServiceClient::new(self.channel.clone())
    }
}
