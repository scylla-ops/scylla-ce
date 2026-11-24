//! The gRPC handler of `UserStateService`: a user reads and writes its own state.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::user_state::{UserStateCaller, UserStateEntry, MAX_VALUE_BYTES};
use scylla_domain::identity::users::KIND;
use scylla_extension::{AccessError, Gate, Principal};
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::user_state_service_server::{
    UserStateService, UserStateServiceServer,
};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

/// Makes the service.
pub(super) fn server(pool: PgPool, gate: Arc<dyn Gate>) -> UserStateServiceServer<UserStateGrpc> {
    UserStateServiceServer::new(UserStateGrpc { pool, gate })
}

/// The handler.
pub(super) struct UserStateGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl UserStateService for UserStateGrpc {
    async fn get_user_state(
        &self,
        request: Request<pb::GetUserStateRequest>,
    ) -> Result<Response<pb::GetUserStateResponse>, Status> {
        let caller = self.user(&request, UserStateCaller::GET_USER_STATE).await?;
        let entries = service::list(&self.pool, caller.actor.id).await?;
        Ok(Response::new(pb::GetUserStateResponse {
            entries: entries.into_iter().map(ToProto::to_proto).collect(),
        }))
    }

    async fn put_user_state_entry(
        &self,
        request: Request<pb::PutUserStateEntryRequest>,
    ) -> Result<Response<pb::PutUserStateEntryResponse>, Status> {
        let permission = UserStateCaller::PUT_USER_STATE_ENTRY;
        let caller = self.user(&request, permission).await?;
        let request = request.into_inner();
        let key = parse::field(request.key, "key")?;
        let value = parse::json(&request.value_json, "value_json", MAX_VALUE_BYTES)?;
        let expected = request.expected_version;
        let entry = service::put(&self.pool, caller.actor.id, &key, &value, expected).await?;
        Ok(Response::new(pb::PutUserStateEntryResponse {
            entry: Some(entry.to_proto()),
        }))
    }
}

impl UserStateGrpc {
    /// Authenticates the caller and checks the route. Only a user with a session has a state.
    async fn user<T: Sync>(
        &self,
        request: &Request<T>,
        permission: Permission<Actor>,
    ) -> Result<Principal, Error> {
        let caller = self.gate.caller(request).await?;
        let check = permission.on(caller.actor.id);
        self.gate.authorize(&caller, check.clone()).await?;
        if caller.actor.kind != KIND || caller.api_token().is_some() {
            let reason = "only a user with a session has a state".to_owned();
            let object = check.object();
            let refusal = AccessError::refused(&caller, check.permission(), object, reason);
            return Err(refusal.into());
        }
        Ok(caller)
    }
}

impl ToProto<pb::UserStateEntry> for UserStateEntry {
    fn to_proto(self) -> pb::UserStateEntry {
        pb::UserStateEntry {
            key: self.key,
            value_json: self.value_json,
            version: self.version,
            updated_at: Some(parse::timestamp(self.updated_at)),
        }
    }
}
