//! The gRPC handler of `UserStateService`: a user reads and writes its own state.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::identity::actors::Actor;
use scylla_domain::identity::user_state::{UserStateCaller, UserStateEntry, MAX_VALUE_BYTES};
use scylla_domain::identity::users::KIND;
use scylla_domain::DomainError;
use scylla_extension::{AccessError, Gate, Principal};
use scylla_proto::accounts::v1 as pb;
use scylla_proto::accounts::v1::user_state_service_server::{
    UserStateService, UserStateServiceServer,
};
use serde_json::Value;
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
        let value = value_of(&request.value_json)?;
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

/// Reads a value: a JSON text of at most `MAX_VALUE_BYTES` bytes that `jsonb` can keep.
fn value_of(text: &str) -> Result<Value, Error> {
    let invalid = |reason: String| Error::invalid("value_json", reason);
    if text.len() > MAX_VALUE_BYTES {
        let max = MAX_VALUE_BYTES;
        return Err(invalid(DomainError::TooLarge { max }.to_string()));
    }
    let value = serde_json::from_str(text).map_err(|_| invalid("must be JSON".to_owned()))?;
    if holds_nul(&value) {
        return Err(invalid("must have no NUL character".to_owned()));
    }
    Ok(value)
}

/// Returns `true` if a text in a value holds a NUL character, which `jsonb` refuses.
fn holds_nul(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains('\0'),
        Value::Array(items) => items.iter().any(holds_nul),
        Value::Object(fields) => fields
            .iter()
            .any(|(key, value)| key.contains('\0') || holds_nul(value)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}
