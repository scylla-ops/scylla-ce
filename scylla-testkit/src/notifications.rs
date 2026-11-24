//! The `TestApp` helpers of `NotificationService`, and one pass of the task of the notifications.

use scylla_core::features::notifications;
use scylla_domain::identity::actors::ActorId;
use scylla_proto::notifications::v1::notification_service_client::NotificationServiceClient;
use scylla_proto::notifications::v1::{
    GetNotificationSummaryRequest, GetNotificationSummaryResponse, ListNotificationsRequest,
    ListNotificationsResponse, MarkAllNotificationsReadRequest, MarkNotificationsReadRequest,
    Notification, SetNotificationsDoneRequest,
};
use tonic::transport::Channel;
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Runs one pass of the task of the notifications, and returns the number of facts.
    pub async fn dispatch_notifications(&self) -> usize {
        notifications::dispatch_pending(&self.pool, self.access.as_ref(), &self.notification_kinds)
            .await
            .expect("a pass of the notifications")
    }

    /// Runs one pass, then calls `ListNotifications` as `actor` for the first page of the inbox.
    pub async fn inbox(&self, actor: ActorId) -> Vec<Notification> {
        self.dispatch_notifications().await;
        let request = ListNotificationsRequest::default();
        let page = self.list_notifications(actor, request).await;
        page.expect("a list of the notifications").notifications
    }

    /// Calls `GetNotificationSummary` as `actor`.
    pub async fn notification_summary(
        &self,
        actor: ActorId,
    ) -> Result<GetNotificationSummaryResponse, Status> {
        let request = self.authed(actor, GetNotificationSummaryRequest {}).await;
        let response = self.notifications().get_notification_summary(request).await;
        Ok(response?.into_inner())
    }

    /// Calls `ListNotifications` as `actor`.
    pub async fn list_notifications(
        &self,
        actor: ActorId,
        request: ListNotificationsRequest,
    ) -> Result<ListNotificationsResponse, Status> {
        let request = self.authed(actor, request).await;
        let response = self.notifications().list_notifications(request).await;
        Ok(response?.into_inner())
    }

    /// Calls `MarkNotificationsRead` as `actor`.
    pub async fn mark_notifications_read(
        &self,
        actor: ActorId,
        ids: &[&str],
        read: bool,
    ) -> Result<(), Status> {
        let request = MarkNotificationsReadRequest {
            notification_ids: ids.iter().map(|value| id(*value)).collect(),
            read,
        };
        let request = self.authed(actor, request).await;
        self.notifications()
            .mark_notifications_read(request)
            .await?;
        Ok(())
    }

    /// Calls `MarkAllNotificationsRead` as `actor`.
    pub async fn mark_all_notifications_read(&self, actor: ActorId) -> Result<(), Status> {
        let request = self.authed(actor, MarkAllNotificationsReadRequest {}).await;
        self.notifications()
            .mark_all_notifications_read(request)
            .await?;
        Ok(())
    }

    /// Calls `SetNotificationsDone` as `actor`.
    pub async fn set_notifications_done(
        &self,
        actor: ActorId,
        ids: &[&str],
        done: bool,
    ) -> Result<(), Status> {
        let request = SetNotificationsDoneRequest {
            notification_ids: ids.iter().map(|value| id(*value)).collect(),
            done,
        };
        let request = self.authed(actor, request).await;
        self.notifications().set_notifications_done(request).await?;
        Ok(())
    }

    /// Returns a client of `NotificationService`.
    pub fn notifications(&self) -> NotificationServiceClient<Channel> {
        NotificationServiceClient::new(self.channel.clone())
    }
}
