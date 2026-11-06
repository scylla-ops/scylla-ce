//! The gRPC handler of `ProfileService`: the account of the caller. Each route checks its
//! permission on the caller itself, and needs a user.

use scylla_domain::authz::Permission;
use scylla_domain::identity::actors::{Actor, ActorId};
use scylla_domain::identity::users::{AccountCaller, SessionId, KIND};
use scylla_extension::{AccessError, Principal};
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::profile_service_server::{ProfileService, ProfileServiceServer};
use tonic::{Request, Response, Status};

use super::{convert, manage, AccountContext};
use crate::api::parse::{self, ToProto};
use crate::features::identity::sessions;
use crate::Error;

/// Makes the service.
pub(crate) fn server(ctx: AccountContext) -> ProfileServiceServer<ProfileGrpc> {
    ProfileServiceServer::new(ProfileGrpc { ctx })
}

/// The handler.
pub(crate) struct ProfileGrpc {
    /// What the account routes share.
    ctx: AccountContext,
}

/// The answer of a route.
type Answer<T> = Result<Response<T>, Status>;

#[tonic::async_trait]
impl ProfileService for ProfileGrpc {
    async fn get_me(&self, request: Request<pb::GetMeRequest>) -> Answer<pb::GetMeResponse> {
        let (_, user) = self.user(&request, AccountCaller::GET_ME).await?;
        let user = manage::get_user(&self.ctx.pool, user).await?;
        Ok(Response::new(pb::GetMeResponse {
            user: Some(user.to_proto()),
        }))
    }

    async fn update_profile(
        &self,
        request: Request<pb::UpdateProfileRequest>,
    ) -> Answer<pb::UpdateProfileResponse> {
        let (_, user) = self.user(&request, AccountCaller::UPDATE_PROFILE).await?;
        let request = request.into_inner();
        let change = convert::profile_change(request.display_name, request.handle)?;
        let user = manage::change_profile(&self.ctx.pool, user, &change, None).await?;
        Ok(Response::new(pb::UpdateProfileResponse {
            user: Some(user.to_proto()),
        }))
    }

    async fn delete_account(
        &self,
        request: Request<pb::DeleteAccountRequest>,
    ) -> Answer<pb::DeleteAccountResponse> {
        let (_, user) = self.user(&request, AccountCaller::DELETE_ACCOUNT).await?;
        let password = request.into_inner().password;
        manage::delete_account(&self.ctx, user, password).await?;
        Ok(Response::new(pb::DeleteAccountResponse {}))
    }

    async fn request_email_change(
        &self,
        request: Request<pb::RequestEmailChangeRequest>,
    ) -> Answer<pb::RequestEmailChangeResponse> {
        let (_, user) = self
            .user(&request, AccountCaller::REQUEST_EMAIL_CHANGE)
            .await?;
        let request = request.into_inner();
        let email = parse::field(request.new_email, "new_email")?;
        manage::request_email_change(&self.ctx, user, email, request.password).await?;
        Ok(Response::new(pb::RequestEmailChangeResponse {}))
    }

    async fn confirm_email_change(
        &self,
        request: Request<pb::ConfirmEmailChangeRequest>,
    ) -> Answer<pb::ConfirmEmailChangeResponse> {
        let (_, user) = self
            .user(&request, AccountCaller::CONFIRM_EMAIL_CHANGE)
            .await?;
        let token = request.into_inner().token;
        let user = manage::confirm_email_change(&self.ctx, user, &token).await?;
        Ok(Response::new(pb::ConfirmEmailChangeResponse {
            user: Some(user.to_proto()),
        }))
    }

    async fn change_password(
        &self,
        request: Request<pb::ChangePasswordRequest>,
    ) -> Answer<pb::ChangePasswordResponse> {
        let (caller, user) = self.user(&request, AccountCaller::CHANGE_PASSWORD).await?;
        let request = request.into_inner();
        let new = parse::field(request.new_password, "new_password")?;
        let keep = caller.session();
        manage::change_password(&self.ctx, user, keep, request.current_password, new).await?;
        Ok(Response::new(pb::ChangePasswordResponse {}))
    }

    async fn list_identities(
        &self,
        request: Request<pb::ListIdentitiesRequest>,
    ) -> Answer<pb::ListIdentitiesResponse> {
        let (_, user) = self.user(&request, AccountCaller::LIST_IDENTITIES).await?;
        let (identities, has_password) = manage::sign_in_ways(&self.ctx.pool, user).await?;
        Ok(Response::new(pb::ListIdentitiesResponse {
            identities: identities
                .into_iter()
                .map(|identity| pb::Identity {
                    provider: identity.provider,
                    subject: identity.subject,
                    email: identity.email.unwrap_or_default(),
                    linked_at: Some(parse::timestamp(identity.linked_at)),
                })
                .collect(),
            has_password,
        }))
    }

    async fn unlink_identity(
        &self,
        request: Request<pb::UnlinkIdentityRequest>,
    ) -> Answer<pb::UnlinkIdentityResponse> {
        let (_, user) = self.user(&request, AccountCaller::UNLINK_IDENTITY).await?;
        let request = request.into_inner();
        let ctx = &self.ctx;
        manage::unlink_identity(ctx, user, &request.provider, &request.subject).await?;
        Ok(Response::new(pb::UnlinkIdentityResponse {}))
    }

    async fn list_sessions(
        &self,
        request: Request<pb::ListSessionsRequest>,
    ) -> Answer<pb::ListSessionsResponse> {
        let (caller, user) = self.user(&request, AccountCaller::LIST_SESSIONS).await?;
        let open = sessions::of_user(&self.ctx.pool, user).await?;
        Ok(Response::new(pb::ListSessionsResponse {
            sessions: open
                .into_iter()
                .map(|session| convert::session(session, caller.session()))
                .collect(),
        }))
    }

    async fn revoke_session(
        &self,
        request: Request<pb::RevokeSessionRequest>,
    ) -> Answer<pb::RevokeSessionResponse> {
        let (_, user) = self.user(&request, AccountCaller::REVOKE_SESSION).await?;
        let id: SessionId = parse::required(request.into_inner().session_id, "session_id")?;
        sessions::close_one(&self.ctx.pool, user, id).await?;
        Ok(Response::new(pb::RevokeSessionResponse {}))
    }

    async fn revoke_other_sessions(
        &self,
        request: Request<pb::RevokeOtherSessionsRequest>,
    ) -> Answer<pb::RevokeOtherSessionsResponse> {
        let (caller, user) = self
            .user(&request, AccountCaller::REVOKE_OTHER_SESSIONS)
            .await?;
        manage::revoke_sessions(&self.ctx.pool, user, caller.session()).await?;
        Ok(Response::new(pb::RevokeOtherSessionsResponse {}))
    }
}

impl ProfileGrpc {
    /// Authenticates the caller, checks the route permission on the caller, and requires a user.
    async fn user<T: Sync>(
        &self,
        request: &Request<T>,
        permission: Permission<Actor>,
    ) -> Result<(Principal, ActorId), Error> {
        let caller = self.ctx.gate.caller(request).await?;
        let check = permission.on(caller.actor.id);
        self.ctx.gate.authorize(&caller, check.clone()).await?;
        if caller.actor.kind != KIND {
            let reason = "only a user has an account".to_owned();
            let object = check.object();
            return Err(AccessError::refused(&caller, check.permission(), object, reason).into());
        }
        let user = caller.actor.id;
        Ok((caller, user))
    }
}
