//! The gRPC handler of `ProfileService`: the account of the caller. Each route checks its
//! permission on the caller itself, and needs a user.

use scylla_domain::actors::ActorId;
use scylla_domain::authz::Check;
use scylla_domain::users::{AccountCaller, Password, SessionId, KIND};
use scylla_extension::{AccessError, Principal};
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::profile_service_server::{ProfileService, ProfileServiceServer};
use tonic::{Request, Response, Status};

use super::service::refused;
use super::{password, store, AccountContext};
use crate::features::{providers, sessions};
use crate::parse::{self, ToProto};
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

#[tonic::async_trait]
impl ProfileService for ProfileGrpc {
    async fn get_me(
        &self,
        request: Request<pb::GetMeRequest>,
    ) -> Result<Response<pb::GetMeResponse>, Status> {
        let (_, user) = self.user(&request, AccountCaller::GET_ME).await?;
        let mut tx = self.ctx.pool.begin().await.map_err(Error::from)?;
        let found = store::find_user(&mut tx, user).await?;
        tx.commit().await.map_err(Error::from)?;
        let user = found.ok_or(Error::NotFound("user"))?;
        Ok(Response::new(pb::GetMeResponse {
            user: Some(pb::User {
                id: Some(user.id.to_proto()),
                display_name: user.display_name,
                email: user.email.unwrap_or_default(),
                status: user.status.as_str().to_owned(),
                created_at: Some(parse::timestamp(user.created_at)),
            }),
        }))
    }

    async fn change_password(
        &self,
        request: Request<pb::ChangePasswordRequest>,
    ) -> Result<Response<pb::ChangePasswordResponse>, Status> {
        // Check the caller, the new password, and the current password if the user has one.
        let (caller, user) = self.user(&request, AccountCaller::CHANGE_PASSWORD).await?;
        let request = request.into_inner();
        let new: Password = parse::field(request.new_password, "new_password")?;
        let mut tx = self.ctx.pool.begin().await.map_err(Error::from)?;
        if let Some(hash) = store::password_hash(&mut tx, user).await? {
            if !password::verify(request.current_password, Some(hash)).await {
                return Err(Error::invalid("current_password", "does not match").into());
            }
        }

        // Set the password, and close the other sessions of the user.
        let hash = password::hash(new.as_str().to_owned()).await?;
        store::set_password(&mut tx, user, &hash).await?;
        sessions::revoke_all(&mut tx, user, caller.session()).await?;
        tx.commit().await.map_err(Error::from)?;
        Ok(Response::new(pb::ChangePasswordResponse {}))
    }

    async fn list_identities(
        &self,
        request: Request<pb::ListIdentitiesRequest>,
    ) -> Result<Response<pb::ListIdentitiesResponse>, Status> {
        let (_, user) = self.user(&request, AccountCaller::LIST_IDENTITIES).await?;
        let mut tx = self.ctx.pool.begin().await.map_err(Error::from)?;
        let identities = providers::identities_of(&mut tx, user).await?;
        let has_password = store::password_hash(&mut tx, user).await?.is_some();
        tx.commit().await.map_err(Error::from)?;
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
    ) -> Result<Response<pb::UnlinkIdentityResponse>, Status> {
        // Refuse to remove the last way to sign in.
        let (_, user) = self.user(&request, AccountCaller::UNLINK_IDENTITY).await?;
        let request = request.into_inner();
        let mut tx = self.ctx.pool.begin().await.map_err(Error::from)?;
        let identities = providers::identities_of(&mut tx, user).await?;
        let has_password = store::password_hash(&mut tx, user).await?.is_some();
        if identities.len() <= 1 && !has_password {
            let message = "the last way to sign in cannot be removed";
            return Err(refused("LAST_SIGN_IN_METHOD", message).into());
        }

        // Remove the identity.
        if !providers::unlink(&mut tx, user, &request.provider, &request.subject).await? {
            return Err(Error::NotFound("identity").into());
        }
        tx.commit().await.map_err(Error::from)?;
        Ok(Response::new(pb::UnlinkIdentityResponse {}))
    }

    async fn list_sessions(
        &self,
        request: Request<pb::ListSessionsRequest>,
    ) -> Result<Response<pb::ListSessionsResponse>, Status> {
        let (caller, user) = self.user(&request, AccountCaller::LIST_SESSIONS).await?;
        let mut tx = self.ctx.pool.begin().await.map_err(Error::from)?;
        let open = sessions::list(&mut tx, user).await?;
        tx.commit().await.map_err(Error::from)?;
        Ok(Response::new(pb::ListSessionsResponse {
            sessions: open
                .into_iter()
                .map(|session| pb::Session {
                    current: caller.session() == Some(session.id),
                    id: Some(session.id.to_proto()),
                    methods: session.methods,
                    user_agent: session.user_agent.unwrap_or_default(),
                    created_at: Some(parse::timestamp(session.created_at)),
                    last_used_at: Some(parse::timestamp(session.last_used_at)),
                })
                .collect(),
        }))
    }

    async fn revoke_session(
        &self,
        request: Request<pb::RevokeSessionRequest>,
    ) -> Result<Response<pb::RevokeSessionResponse>, Status> {
        let (_, user) = self.user(&request, AccountCaller::REVOKE_SESSION).await?;
        let id: SessionId = parse::required(request.into_inner().session_id, "session_id")?;
        let mut tx = self.ctx.pool.begin().await.map_err(Error::from)?;
        if !sessions::revoke(&mut tx, user, id).await? {
            return Err(Error::NotFound("session").into());
        }
        tx.commit().await.map_err(Error::from)?;
        Ok(Response::new(pb::RevokeSessionResponse {}))
    }
}

impl ProfileGrpc {
    /// Authenticates the caller, checks the route permission on the caller, and requires a user.
    async fn user<T: Sync>(
        &self,
        request: &Request<T>,
        permission: scylla_domain::authz::Permission<scylla_domain::actors::Actor>,
    ) -> Result<(Principal, ActorId), Error> {
        let caller = self.ctx.gate.caller(request).await?;
        let check: Check = permission.on(caller.actor.id);
        self.ctx.gate.authorize(&caller, check).await?;
        if caller.actor.kind != KIND {
            let reason = "only a user has an account".to_owned();
            return Err(AccessError::Denied(reason).into());
        }
        let user = caller.actor.id;
        Ok((caller, user))
    }
}
