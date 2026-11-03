//! The `TestApp` helpers of the instance admins and of `UserAdminService`, sent as an actor.
//! A user id is text, so a test can send a bad id.

use scylla_core::features::identity::accounts;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::users::{Email, NewUser};
use scylla_domain::Name;
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::user_admin_service_client::UserAdminServiceClient;
use scylla_proto::common::v1 as common;
use tonic::transport::Channel;
use tonic::Status;

use crate::{id, TestApp};

impl TestApp {
    /// Creates an active user with its personal workspace and the instance admin right, in one
    /// transaction, as `user create --admin` does, and returns its id.
    pub async fn admin(&self) -> ActorId {
        let new = NewUser {
            email: Email::parse(format!("{}@test.example", ActorId::generate())).expect("email"),
            email_verified: true,
            display_name: Name::parse("Test Admin".to_owned()).expect("name"),
            handle: None,
            password: None,
        };
        accounts::create_admin(&self.pool, new)
            .await
            .expect("create admin")
            .id
    }

    /// Calls `ListUsers` as `actor`, with a filter and a page.
    pub async fn list_users(
        &self,
        actor: ActorId,
        query: &str,
        page: common::PageRequest,
    ) -> Result<pb::ListUsersResponse, Status> {
        let request = pb::ListUsersRequest {
            query: query.to_owned(),
            page: Some(page),
        };
        let request = self.authed(actor, request).await;
        Ok(self.user_admin().list_users(request).await?.into_inner())
    }

    /// Calls `GetUser` as `actor`.
    pub async fn get_user(&self, actor: ActorId, user: &str) -> Result<pb::User, Status> {
        let request = pb::GetUserRequest {
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        let response = self.user_admin().get_user(request).await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `CreateUser` as `actor`. An empty handle means none.
    pub async fn create_user(
        &self,
        actor: ActorId,
        email: &str,
        handle: &str,
        password: &str,
    ) -> Result<pb::User, Status> {
        let request = pb::CreateUserRequest {
            email: email.to_owned(),
            display_name: "Created User".to_owned(),
            handle: handle.to_owned(),
            password: password.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.user_admin().create_user(request).await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `UpdateUser` as `actor`.
    pub async fn update_user(
        &self,
        actor: ActorId,
        request: pb::UpdateUserRequest,
    ) -> Result<pb::User, Status> {
        let request = self.authed(actor, request).await;
        let response = self.user_admin().update_user(request).await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `SetUserStatus` as `actor`.
    pub async fn set_user_status(
        &self,
        actor: ActorId,
        user: &str,
        status: &str,
    ) -> Result<pb::User, Status> {
        let request = pb::SetUserStatusRequest {
            user_id: Some(id(user)),
            status: status.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.user_admin().set_user_status(request).await?;
        Ok(response.into_inner().user.expect("user in response"))
    }

    /// Calls `DeleteUser` as `actor`.
    pub async fn delete_user(&self, actor: ActorId, user: &str) -> Result<(), Status> {
        let request = pb::DeleteUserRequest {
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        self.user_admin().delete_user(request).await?;
        Ok(())
    }

    /// Calls `SendPasswordReset` as `actor`. The server sends the mail after its answer.
    pub async fn send_password_reset(&self, actor: ActorId, user: &str) -> Result<(), Status> {
        let request = pb::SendPasswordResetRequest {
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        self.user_admin().send_password_reset(request).await?;
        Ok(())
    }

    /// Calls `ListUserSessions` as `actor`.
    pub async fn list_user_sessions(
        &self,
        actor: ActorId,
        user: &str,
    ) -> Result<Vec<pb::Session>, Status> {
        let request = pb::ListUserSessionsRequest {
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        let response = self.user_admin().list_user_sessions(request).await?;
        Ok(response.into_inner().sessions)
    }

    /// Calls `RevokeUserSessions` as `actor`.
    pub async fn revoke_user_sessions(&self, actor: ActorId, user: &str) -> Result<(), Status> {
        let request = pb::RevokeUserSessionsRequest {
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        self.user_admin().revoke_user_sessions(request).await?;
        Ok(())
    }

    /// Calls `GrantInstanceAdmin` as `actor`.
    pub async fn grant_instance_admin(&self, actor: ActorId, user: &str) -> Result<(), Status> {
        let request = pb::GrantInstanceAdminRequest {
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        self.user_admin().grant_instance_admin(request).await?;
        Ok(())
    }

    /// Calls `RevokeInstanceAdmin` as `actor`.
    pub async fn revoke_instance_admin(&self, actor: ActorId, user: &str) -> Result<(), Status> {
        let request = pb::RevokeInstanceAdminRequest {
            user_id: Some(id(user)),
        };
        let request = self.authed(actor, request).await;
        self.user_admin().revoke_instance_admin(request).await?;
        Ok(())
    }

    /// Returns a client of `UserAdminService`.
    fn user_admin(&self) -> UserAdminServiceClient<Channel> {
        UserAdminServiceClient::new(self.channel.clone())
    }
}
