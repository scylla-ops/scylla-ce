//! The gRPC handler of `NotificationService`: a user reads and changes its own inbox.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::users::KIND;
use scylla_domain::notifications::{
    Filter, InvitationState, Notification, NotificationCaller, NotificationId, POLL_AFTER_SECONDS,
};
use scylla_extension::{AccessError, Gate, NotificationKinds, Principal};
use scylla_proto::common::v1::Id as ProtoId;
use scylla_proto::notifications::v1 as pb;
use scylla_proto::notifications::v1::notification_invitation as invitation_pb;
use scylla_proto::notifications::v1::notification_service_server::{
    NotificationService, NotificationServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(
    pool: PgPool,
    gate: Arc<dyn Gate>,
    kinds: Arc<NotificationKinds>,
) -> NotificationServiceServer<NotificationsGrpc> {
    NotificationServiceServer::new(NotificationsGrpc { pool, gate, kinds })
}

/// The handler.
pub(super) struct NotificationsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
    /// The kinds of the edition.
    kinds: Arc<NotificationKinds>,
}

#[tonic::async_trait]
impl NotificationService for NotificationsGrpc {
    async fn get_notification_summary(
        &self,
        request: Request<pb::GetNotificationSummaryRequest>,
    ) -> Result<Response<pb::GetNotificationSummaryResponse>, Status> {
        Ok(Response::new(self.summary(request).await?))
    }

    async fn list_notifications(
        &self,
        request: Request<pb::ListNotificationsRequest>,
    ) -> Result<Response<pb::ListNotificationsResponse>, Status> {
        Ok(Response::new(self.list(request).await?))
    }

    async fn mark_notifications_read(
        &self,
        request: Request<pb::MarkNotificationsReadRequest>,
    ) -> Result<Response<pb::MarkNotificationsReadResponse>, Status> {
        Ok(Response::new(self.mark_read(request).await?))
    }

    async fn mark_all_notifications_read(
        &self,
        request: Request<pb::MarkAllNotificationsReadRequest>,
    ) -> Result<Response<pb::MarkAllNotificationsReadResponse>, Status> {
        Ok(Response::new(self.mark_all_read(request).await?))
    }

    async fn set_notifications_done(
        &self,
        request: Request<pb::SetNotificationsDoneRequest>,
    ) -> Result<Response<pb::SetNotificationsDoneResponse>, Status> {
        Ok(Response::new(self.set_done(request).await?))
    }
}

impl NotificationsGrpc {
    /// Authenticates the caller and checks the route permission on the caller. Only a user with no
    /// API token has an inbox.
    async fn user<T: Sync>(
        &self,
        request: &Request<T>,
        permission: Permission<Actor>,
    ) -> Result<Principal, Error> {
        let caller = self.gate.caller(request).await?;
        let check = permission.on(caller.actor.id);
        self.gate.authorize(&caller, check.clone()).await?;
        if caller.actor.kind != KIND || caller.api_token().is_some() {
            let reason = "only a user with a session has an inbox".to_owned();
            let object = check.object();
            let refusal = AccessError::refused(&caller, check.permission(), object, reason);
            return Err(refusal.into());
        }
        Ok(caller)
    }

    /// Handles `GetNotificationSummary`.
    async fn summary(
        &self,
        request: Request<pb::GetNotificationSummaryRequest>,
    ) -> Result<pb::GetNotificationSummaryResponse, Error> {
        let permission = NotificationCaller::GET_NOTIFICATION_SUMMARY;
        let caller = self.user(&request, permission).await?;
        let summary = service::summary(&self.pool, caller.actor.id).await?;
        Ok(pb::GetNotificationSummaryResponse {
            unread_count: summary.unread,
            needs_action_count: summary.needs_action,
            poll_after_seconds: POLL_AFTER_SECONDS,
        })
    }

    /// Handles `ListNotifications`.
    async fn list(
        &self,
        request: Request<pb::ListNotificationsRequest>,
    ) -> Result<pb::ListNotificationsResponse, Error> {
        // Authenticate the caller, then read the request.
        let caller = self
            .user(&request, NotificationCaller::LIST_NOTIFICATIONS)
            .await?;
        let request = request.into_inner();
        let filter = Filter {
            done: request.done,
            unread_only: request.unread_only,
            needs_action_only: request.needs_action_only,
            workspace: request
                .workspace_id
                .map(|id| parse::field(id, "workspace_id"))
                .transpose()?,
            kinds: (!request.category.is_empty()).then(|| self.kinds.keys_of(&request.category)),
        };
        let page = parse::optional(request.page, "page")?;

        // Read one page, and name the category of each row.
        let (gate, kinds) = (self.gate.as_ref(), self.kinds.as_ref());
        let page = service::list(&self.pool, gate, kinds, &caller, &filter, page).await?;
        let notifications = page
            .items
            .into_iter()
            .map(|row| {
                let category = kinds.get(&row.kind).map(|kind| kind.category);
                let category = category.unwrap_or_default().to_owned();
                pb::Notification {
                    category,
                    ..row.to_proto()
                }
            })
            .collect();
        Ok(pb::ListNotificationsResponse {
            notifications,
            page: Some(page.next_after.to_proto()),
        })
    }

    /// Handles `MarkNotificationsRead`.
    async fn mark_read(
        &self,
        request: Request<pb::MarkNotificationsReadRequest>,
    ) -> Result<pb::MarkNotificationsReadResponse, Error> {
        let permission = NotificationCaller::MARK_NOTIFICATIONS_READ;
        let caller = self.user(&request, permission).await?;
        let request = request.into_inner();
        let ids = read_ids(request.notification_ids)?;
        service::set_read(&self.pool, caller.actor.id, &ids, request.read).await?;
        Ok(pb::MarkNotificationsReadResponse {})
    }

    /// Handles `MarkAllNotificationsRead`.
    async fn mark_all_read(
        &self,
        request: Request<pb::MarkAllNotificationsReadRequest>,
    ) -> Result<pb::MarkAllNotificationsReadResponse, Error> {
        let permission = NotificationCaller::MARK_ALL_NOTIFICATIONS_READ;
        let caller = self.user(&request, permission).await?;
        service::set_all_read(&self.pool, caller.actor.id).await?;
        Ok(pb::MarkAllNotificationsReadResponse {})
    }

    /// Handles `SetNotificationsDone`.
    async fn set_done(
        &self,
        request: Request<pb::SetNotificationsDoneRequest>,
    ) -> Result<pb::SetNotificationsDoneResponse, Error> {
        let permission = NotificationCaller::SET_NOTIFICATIONS_DONE;
        let caller = self.user(&request, permission).await?;
        let request = request.into_inner();
        let ids = read_ids(request.notification_ids)?;
        service::set_done(&self.pool, caller.actor.id, &ids, request.done).await?;
        Ok(pb::SetNotificationsDoneResponse {})
    }
}

/// Reads the ids of a request.
fn read_ids(ids: Vec<ProtoId>) -> Result<Vec<NotificationId>, Error> {
    ids.into_iter()
        .map(|id| parse::field(id, "notification_ids"))
        .collect()
}

impl ToProto<pb::Notification> for Notification {
    fn to_proto(self) -> pb::Notification {
        pb::Notification {
            id: Some(self.id.to_proto()),
            kind: self.kind,
            category: String::new(),
            reason: self.reason,
            actor_name: self.actor_name.unwrap_or_default(),
            workspace_id: self.workspace_id.map(ToProto::to_proto),
            workspace_slug: self.workspace_slug.unwrap_or_default(),
            workspace_name: self.workspace_name.unwrap_or_default(),
            project_id: self.project_id.map(ToProto::to_proto),
            project_name: self.project_name.unwrap_or_default(),
            params: self.params.into_iter().collect(),
            occurrences: self.occurrences,
            needs_action: self.needs_action,
            first_at: Some(parse::timestamp(self.first_at)),
            last_at: Some(parse::timestamp(self.last_at)),
            read_at: self.read_at.map(parse::timestamp),
            done_at: self.done_at.map(parse::timestamp),
            invitation: self
                .invitation
                .map(|invitation| pb::NotificationInvitation {
                    id: Some(invitation.id.to_proto()),
                    role: invitation.role,
                    state: Some(match invitation.state {
                        InvitationState::Open(at) => {
                            invitation_pb::State::Open(invitation_pb::Open {
                                expires_at: Some(parse::timestamp(at)),
                            })
                        }
                        InvitationState::Accepted(at) => {
                            invitation_pb::State::Accepted(invitation_pb::Accepted {
                                accepted_at: Some(parse::timestamp(at)),
                            })
                        }
                        InvitationState::Expired(at) => {
                            invitation_pb::State::Expired(invitation_pb::Expired {
                                expired_at: Some(parse::timestamp(at)),
                            })
                        }
                    }),
                }),
        }
    }
}
