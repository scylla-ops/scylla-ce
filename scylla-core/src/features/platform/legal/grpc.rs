//! The gRPC handler of `LegalService`: anyone reads the legal settings, and the instance admins
//! change them.

use std::sync::Arc;

use scylla_domain::platform::legal::LegalHolder;
use scylla_domain::platform::Platform;
use scylla_extension::Gate;
use scylla_proto::platform::v1 as pb;
use scylla_proto::platform::v1::legal_service_server::{LegalService, LegalServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::{convert, service};
use crate::api::parse::ToProto;
use crate::Error;

/// Makes the service.
pub(crate) fn server(pool: PgPool, gate: Arc<dyn Gate>) -> LegalServiceServer<LegalGrpc> {
    LegalServiceServer::new(LegalGrpc { pool, gate })
}

/// The handler.
pub(crate) struct LegalGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl LegalService for LegalGrpc {
    async fn get_legal_info(
        &self,
        _request: Request<pb::GetLegalInfoRequest>,
    ) -> Result<Response<pb::GetLegalInfoResponse>, Status> {
        let settings = service::get(&self.pool).await?;
        Ok(Response::new(pb::GetLegalInfoResponse {
            info: Some(settings.to_proto()),
        }))
    }

    async fn update_legal_settings(
        &self,
        request: Request<pb::UpdateLegalSettingsRequest>,
    ) -> Result<Response<pb::UpdateLegalSettingsResponse>, Status> {
        let caller = self.gate.caller(&request).await.map_err(Error::from)?;
        let check = LegalHolder::UPDATE_LEGAL_SETTINGS.on(Platform::ID);
        self.gate
            .authorize(&caller, check)
            .await
            .map_err(Error::from)?;
        let change = convert::change(request.into_inner())?;
        let settings = service::update(&self.pool, change).await?;
        Ok(Response::new(pb::UpdateLegalSettingsResponse {
            info: Some(settings.to_proto()),
        }))
    }
}
