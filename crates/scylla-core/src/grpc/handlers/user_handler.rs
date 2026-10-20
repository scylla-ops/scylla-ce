//! The adapter: each RPC is one `run` and its response. Parsing lives in the
//! user mapper, behind `Parse`; no RPC checks a permission or touches a port.

use crate::application::{PasswordResetUseCases, UserUseCases};
use crate::grpc::adapter::{run, run_in_session};
use crate::grpc::mappers::{
    delivery_to_proto, user_access_response, user_sessions_response, user_to_proto,
};
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::user::v1::{
    ChangePasswordRequest, ChangePasswordResponse, CreateUserRequest, CreateUserResponse,
    DeleteAccountRequest, DeleteAccountResponse, DeleteUserRequest, DeleteUserResponse,
    GetMeRequest, GetMeResponse, GetUserRequest, GetUserResponse, ListUserAccessRequest,
    ListUserAccessResponse, ListUserSessionsRequest, ListUserSessionsResponse, ListUsersRequest,
    ListUsersResponse, RevokeUserSessionRequest, RevokeUserSessionResponse,
    RevokeUserSessionsRequest, RevokeUserSessionsResponse, SendPasswordResetRequest,
    SendPasswordResetResponse, SetUserActiveRequest, SetUserActiveResponse, UpdateUserRequest,
    UpdateUserResponse, user_service_server::UserService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct UserHandler {
    actions: Arc<Actions>,
    users: Arc<UserUseCases>,
    resets: Arc<PasswordResetUseCases>,
}

#[async_trait::async_trait]
impl UserService for UserHandler {
    async fn create_user(
        &self,
        request: Request<CreateUserRequest>,
    ) -> Result<Response<CreateUserResponse>, Status> {
        let user = run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(CreateUserResponse {
            user: Some(user_to_proto(&user)),
        }))
    }

    async fn get_user(
        &self,
        request: Request<GetUserRequest>,
    ) -> Result<Response<GetUserResponse>, Status> {
        let user = run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(GetUserResponse {
            user: Some(user_to_proto(&user)),
        }))
    }

    async fn get_me(
        &self,
        request: Request<GetMeRequest>,
    ) -> Result<Response<GetMeResponse>, Status> {
        let user = run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(GetMeResponse {
            user: Some(user_to_proto(&user)),
        }))
    }

    async fn update_user(
        &self,
        request: Request<UpdateUserRequest>,
    ) -> Result<Response<UpdateUserResponse>, Status> {
        let user = run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(UpdateUserResponse {
            user: Some(user_to_proto(&user)),
        }))
    }

    async fn delete_user(
        &self,
        request: Request<DeleteUserRequest>,
    ) -> Result<Response<DeleteUserResponse>, Status> {
        run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(DeleteUserResponse {}))
    }

    async fn list_users(
        &self,
        request: Request<ListUsersRequest>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let page = run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn change_password(
        &self,
        request: Request<ChangePasswordRequest>,
    ) -> Result<Response<ChangePasswordResponse>, Status> {
        run_in_session(&self.actions, &*self.users, request).await?;
        Ok(Response::new(ChangePasswordResponse {}))
    }

    async fn send_password_reset(
        &self,
        request: Request<SendPasswordResetRequest>,
    ) -> Result<Response<SendPasswordResetResponse>, Status> {
        let delivery = run(&self.actions, &*self.resets, request).await?;
        Ok(Response::new(SendPasswordResetResponse {
            delivery: delivery_to_proto(delivery),
        }))
    }

    async fn revoke_user_sessions(
        &self,
        request: Request<RevokeUserSessionsRequest>,
    ) -> Result<Response<RevokeUserSessionsResponse>, Status> {
        let revoked = run_in_session(&self.actions, &*self.users, request).await?;
        Ok(Response::new(RevokeUserSessionsResponse {
            revoked: u32::try_from(revoked).unwrap_or(u32::MAX),
        }))
    }

    async fn set_user_active(
        &self,
        request: Request<SetUserActiveRequest>,
    ) -> Result<Response<SetUserActiveResponse>, Status> {
        let user = run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(SetUserActiveResponse {
            user: Some(user_to_proto(&user)),
        }))
    }

    async fn delete_account(
        &self,
        request: Request<DeleteAccountRequest>,
    ) -> Result<Response<DeleteAccountResponse>, Status> {
        run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(DeleteAccountResponse {}))
    }

    async fn list_user_access(
        &self,
        request: Request<ListUserAccessRequest>,
    ) -> Result<Response<ListUserAccessResponse>, Status> {
        let access = run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(user_access_response(&access)))
    }

    async fn list_user_sessions(
        &self,
        request: Request<ListUserSessionsRequest>,
    ) -> Result<Response<ListUserSessionsResponse>, Status> {
        let sessions = run_in_session(&self.actions, &*self.users, request).await?;
        Ok(Response::new(user_sessions_response(&sessions)))
    }

    async fn revoke_user_session(
        &self,
        request: Request<RevokeUserSessionRequest>,
    ) -> Result<Response<RevokeUserSessionResponse>, Status> {
        run(&self.actions, &*self.users, request).await?;
        Ok(Response::new(RevokeUserSessionResponse {}))
    }
}
