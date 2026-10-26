use super::{authed, parse};
use crate::{Users, UsersRpc};
use scylla_core::grpc::convert::ParseInSession;
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::{
    delivery_to_proto, user_access_response, user_sessions_response, user_to_proto,
};
use scylla_proto::user::v1 as pb;
use strata_tonic::Grpc;
use tonic::Status;

pub use super::service::user::user_service_client::UserServiceClient;
pub use super::service::user::user_service_server::{UserService, UserServiceServer};

fn in_session<R: ParseInSession>(req: R) -> Result<R::Into, Status> {
    req.parse_in_session(None)
}

strata_tonic::service! {
    impl<A: Users> UserService for Grpc<A> {
        rpc create_user(pb::CreateUserRequest) -> pb::CreateUserResponse = |call, req| {
            authed(&call).await?;
            let user = UsersRpc::create_user(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateUserResponse { user: Some(user_to_proto(&user)) })
        }
        rpc get_user(pb::GetUserRequest) -> pb::GetUserResponse = |call, req| {
            authed(&call).await?;
            let user = UsersRpc::get_user(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::GetUserResponse { user: Some(user_to_proto(&user)) })
        }
        rpc get_me(pb::GetMeRequest) -> pb::GetMeResponse = |call, req| {
            authed(&call).await?;
            let user = UsersRpc::get_me(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::GetMeResponse { user: Some(user_to_proto(&user)) })
        }
        rpc update_user(pb::UpdateUserRequest) -> pb::UpdateUserResponse = |call, req| {
            authed(&call).await?;
            let user = UsersRpc::update_user(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::UpdateUserResponse { user: Some(user_to_proto(&user)) })
        }
        rpc delete_user(pb::DeleteUserRequest) -> pb::DeleteUserResponse = |call, req| {
            authed(&call).await?;
            UsersRpc::delete_user(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteUserResponse {})
        }
        rpc list_users(pb::ListUsersRequest) -> pb::ListUsersResponse = |call, req| {
            authed(&call).await?;
            Ok(UsersRpc::list_users(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?.into())
        }
        rpc change_password(pb::ChangePasswordRequest) -> pb::ChangePasswordResponse = |call, req| {
            authed(&call).await?;
            UsersRpc::change_password(&call.scoped(), in_session(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ChangePasswordResponse {})
        }
        rpc send_password_reset(pb::SendPasswordResetRequest) -> pb::SendPasswordResetResponse = |call, req| {
            authed(&call).await?;
            let delivery = UsersRpc::send_password_reset(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::SendPasswordResetResponse { delivery: delivery_to_proto(delivery) })
        }
        rpc revoke_user_sessions(pb::RevokeUserSessionsRequest) -> pb::RevokeUserSessionsResponse = |call, req| {
            authed(&call).await?;
            let revoked = UsersRpc::revoke_user_sessions(&call.scoped(), in_session(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::RevokeUserSessionsResponse { revoked: u32::try_from(revoked).unwrap_or(u32::MAX) })
        }
        rpc set_user_active(pb::SetUserActiveRequest) -> pb::SetUserActiveResponse = |call, req| {
            authed(&call).await?;
            let user = UsersRpc::set_user_active(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::SetUserActiveResponse { user: Some(user_to_proto(&user)) })
        }
        rpc delete_account(pb::DeleteAccountRequest) -> pb::DeleteAccountResponse = |call, req| {
            authed(&call).await?;
            UsersRpc::delete_account(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteAccountResponse {})
        }
        rpc list_user_access(pb::ListUserAccessRequest) -> pb::ListUserAccessResponse = |call, req| {
            authed(&call).await?;
            let access = UsersRpc::list_user_access(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(user_access_response(&access))
        }
        rpc list_user_sessions(pb::ListUserSessionsRequest) -> pb::ListUserSessionsResponse = |call, req| {
            authed(&call).await?;
            let sessions = UsersRpc::list_user_sessions(&call.scoped(), in_session(req)?).await.map_err(domain_error_to_status)?;
            Ok(user_sessions_response(&sessions))
        }
        rpc revoke_user_session(pb::RevokeUserSessionRequest) -> pb::RevokeUserSessionResponse = |call, req| {
            authed(&call).await?;
            UsersRpc::revoke_user_session(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::RevokeUserSessionResponse {})
        }
    }
}
