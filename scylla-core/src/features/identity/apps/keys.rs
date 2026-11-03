//! The handlers of `AppService` for the keys and the join codes of an App.

use scylla_domain::identity::apps::{App, AppId, AppKeyId};
use scylla_proto::apps::v1 as pb;
use tonic::Request;

use super::grpc::AppsGrpc;
use super::service;
use crate::api::parse::{self, ToProto};
use crate::Error;

impl AppsGrpc {
    /// Handles `CreateJoinCode`: a new code, and the previous open codes stop.
    pub(super) async fn join_code(
        &self,
        request: Request<pb::CreateJoinCodeRequest>,
    ) -> Result<pb::CreateJoinCodeResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let id: AppId = parse::required(request.into_inner().app_id, "app_id")?;
        self.authorize(&caller, App::CREATE_JOIN_CODE, id).await?;
        let code = service::join_code(&self.pool, id, caller.actor.id).await?;
        Ok(pb::CreateJoinCodeResponse {
            join_code: code.code,
            expires_at: Some(parse::timestamp(code.expires_at)),
        })
    }

    /// Handles `AddAppKey`: a public key that the App made.
    pub(super) async fn add_key(
        &self,
        request: Request<pb::AddAppKeyRequest>,
    ) -> Result<pb::AddAppKeyResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.app_id, "app_id")?;
        self.authorize(&caller, App::ADD_APP_KEY, id).await?;
        let key = service::add_key(&self.pool, id, &request.public_key).await?;
        Ok(pb::AddAppKeyResponse {
            key: Some(key.to_proto()),
        })
    }

    /// Handles `RevokeAppKey`: the tokens of the key stop, and the stream of the App closes.
    pub(super) async fn revoke_key(
        &self,
        request: Request<pb::RevokeAppKeyRequest>,
    ) -> Result<pb::RevokeAppKeyResponse, Error> {
        let caller = self.gate.caller(&request).await?;
        let request = request.into_inner();
        let id: AppId = parse::required(request.app_id, "app_id")?;
        let key: AppKeyId = parse::required(request.key_id, "key_id")?;
        self.authorize(&caller, App::REVOKE_APP_KEY, id).await?;
        service::revoke_key(&self.pool, &self.registry, id, key).await?;
        Ok(pb::RevokeAppKeyResponse {})
    }
}
