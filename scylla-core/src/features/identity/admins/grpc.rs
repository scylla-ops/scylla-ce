//! The gRPC handler of `UserAdminService`: each route checks its permission on the platform,
//! where only the instance admins pass.

use scylla_domain::authz::Permission;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::identity::admins::{ListUsers, UserAdminHolder};
use scylla_domain::identity::users::{NewUser, UserStatus};
use scylla_domain::platform::Platform;
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::user_admin_service_server::{
    UserAdminService, UserAdminServiceServer,
};
use scylla_proto::common::v1 as common;
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::features::identity::accounts::{self, AccountContext};
use crate::Error;

/// Makes the service.
pub(crate) fn server(ctx: AccountContext) -> UserAdminServiceServer<UserAdminGrpc> {
    UserAdminServiceServer::new(UserAdminGrpc { ctx })
}

/// The handler.
pub(crate) struct UserAdminGrpc {
    /// What the account routes share.
    ctx: AccountContext,
}

/// The answer of a route.
type Answer<T> = Result<Response<T>, Status>;

#[tonic::async_trait]
impl UserAdminService for UserAdminGrpc {
    async fn list_users(
        &self,
        request: Request<pb::ListUsersRequest>,
    ) -> Answer<pb::ListUsersResponse> {
        self.admin(&request, UserAdminHolder::LIST_USERS).await?;
        let request = request.into_inner();
        let query = ListUsers {
            query: request.query,
            page: parse::optional(request.page, "page")?,
        };
        let page = accounts::list_users(&self.ctx.pool, query).await?;
        Ok(Response::new(pb::ListUsersResponse {
            users: page.items.into_iter().map(ToProto::to_proto).collect(),
            page: Some(page.next_after.to_proto()),
        }))
    }

    async fn get_user(&self, request: Request<pb::GetUserRequest>) -> Answer<pb::GetUserResponse> {
        self.admin(&request, UserAdminHolder::GET_USER).await?;
        let user = user_id(request.into_inner().user_id)?;
        let user = accounts::get_user(&self.ctx.pool, user).await?;
        Ok(Response::new(pb::GetUserResponse {
            user: Some(user.to_proto()),
        }))
    }

    async fn create_user(
        &self,
        request: Request<pb::CreateUserRequest>,
    ) -> Answer<pb::CreateUserResponse> {
        // Check the caller, and read the new user.
        self.admin(&request, UserAdminHolder::CREATE_USER).await?;
        let request = request.into_inner();
        let handle = match request.handle.trim() {
            "" => None,
            _ => Some(parse::field(request.handle, "handle")?),
        };
        let new = NewUser {
            email: parse::field(request.email, "email")?,
            email_verified: true,
            display_name: parse::field(request.display_name, "display_name")?,
            handle,
            password: Some(parse::field(request.password, "password")?),
        };

        // Create the user, and answer with it.
        let user = service::create(&self.ctx.pool, new).await?;
        Ok(Response::new(pb::CreateUserResponse {
            user: Some(user.to_proto()),
        }))
    }

    async fn update_user(
        &self,
        request: Request<pb::UpdateUserRequest>,
    ) -> Answer<pb::UpdateUserResponse> {
        // Check the caller, and read the changes.
        self.admin(&request, UserAdminHolder::UPDATE_USER).await?;
        let request = request.into_inner();
        let user = user_id(request.user_id)?;
        let change = accounts::profile_change(request.display_name, request.handle)?;
        let email = request
            .email
            .map(|email| parse::field(email, "email"))
            .transpose()?;

        // Change the user, and answer with it.
        let user = accounts::change_profile(&self.ctx.pool, user, &change, email.as_ref()).await?;
        Ok(Response::new(pb::UpdateUserResponse {
            user: Some(user.to_proto()),
        }))
    }

    async fn set_user_status(
        &self,
        request: Request<pb::SetUserStatusRequest>,
    ) -> Answer<pb::SetUserStatusResponse> {
        // Check the caller, and read the user and its new status.
        let caller = self
            .admin(&request, UserAdminHolder::SET_USER_STATUS)
            .await?;
        let request = request.into_inner();
        let user = user_id(request.user_id)?;
        let status = match request.status.as_str() {
            "active" => UserStatus::Active,
            "disabled" => UserStatus::Disabled,
            _ => return Err(Error::invalid("status", "must be active or disabled").into()),
        };

        // Change the status, and answer with the user.
        let user = service::set_status(&self.ctx.pool, caller, user, status).await?;
        Ok(Response::new(pb::SetUserStatusResponse {
            user: Some(user.to_proto()),
        }))
    }

    async fn delete_user(
        &self,
        request: Request<pb::DeleteUserRequest>,
    ) -> Answer<pb::DeleteUserResponse> {
        let caller = self.admin(&request, UserAdminHolder::DELETE_USER).await?;
        let user = user_id(request.into_inner().user_id)?;
        service::delete(&self.ctx, caller, user).await?;
        Ok(Response::new(pb::DeleteUserResponse {}))
    }

    async fn send_password_reset(
        &self,
        request: Request<pb::SendPasswordResetRequest>,
    ) -> Answer<pb::SendPasswordResetResponse> {
        self.admin(&request, UserAdminHolder::SEND_PASSWORD_RESET)
            .await?;
        let user = user_id(request.into_inner().user_id)?;
        accounts::send_reset(&self.ctx, user).await?;
        Ok(Response::new(pb::SendPasswordResetResponse {}))
    }

    async fn list_user_sessions(
        &self,
        request: Request<pb::ListUserSessionsRequest>,
    ) -> Answer<pb::ListUserSessionsResponse> {
        self.admin(&request, UserAdminHolder::LIST_USER_SESSIONS)
            .await?;
        let user = user_id(request.into_inner().user_id)?;
        let open = service::sessions_of(&self.ctx.pool, user).await?;
        Ok(Response::new(pb::ListUserSessionsResponse {
            sessions: open
                .into_iter()
                .map(|info| accounts::session(info, None))
                .collect(),
        }))
    }

    async fn revoke_user_sessions(
        &self,
        request: Request<pb::RevokeUserSessionsRequest>,
    ) -> Answer<pb::RevokeUserSessionsResponse> {
        self.admin(&request, UserAdminHolder::REVOKE_USER_SESSIONS)
            .await?;
        let user = user_id(request.into_inner().user_id)?;
        accounts::revoke_sessions(&self.ctx.pool, user, None).await?;
        Ok(Response::new(pb::RevokeUserSessionsResponse {}))
    }

    async fn grant_instance_admin(
        &self,
        request: Request<pb::GrantInstanceAdminRequest>,
    ) -> Answer<pb::GrantInstanceAdminResponse> {
        self.admin(&request, UserAdminHolder::GRANT_INSTANCE_ADMIN)
            .await?;
        let user = user_id(request.into_inner().user_id)?;
        service::grant(&self.ctx.pool, user).await?;
        Ok(Response::new(pb::GrantInstanceAdminResponse {}))
    }

    async fn revoke_instance_admin(
        &self,
        request: Request<pb::RevokeInstanceAdminRequest>,
    ) -> Answer<pb::RevokeInstanceAdminResponse> {
        self.admin(&request, UserAdminHolder::REVOKE_INSTANCE_ADMIN)
            .await?;
        let user = user_id(request.into_inner().user_id)?;
        service::revoke(&self.ctx.pool, user).await?;
        Ok(Response::new(pb::RevokeInstanceAdminResponse {}))
    }
}

impl UserAdminGrpc {
    /// Authenticates the caller, and checks the route permission on the platform. Returns the
    /// actor of the caller.
    async fn admin<T: Sync>(
        &self,
        request: &Request<T>,
        permission: Permission<Platform>,
    ) -> Result<ActorId, Error> {
        let caller = self.ctx.gate.caller(request).await?;
        self.ctx
            .gate
            .authorize(&caller, permission.on(Platform::ID))
            .await?;
        Ok(caller.actor.id)
    }
}

/// Reads the required user of a request.
fn user_id(value: Option<common::Id>) -> Result<ActorId, Error> {
    parse::required(value, "user_id")
}
