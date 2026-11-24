//! The gRPC handler of `SettingsService`: the instance admins read and change the settings.

use std::sync::Arc;

use scylla_domain::authz::Permission;
use scylla_domain::platform::{InstanceSettings, InstanceSettingsChange, Platform, SettingsHolder};
use scylla_extension::Gate;
use scylla_proto::platform::v1 as pb;
use scylla_proto::platform::v1::settings_service_server::{SettingsService, SettingsServiceServer};
use sqlx::PgPool;
use tonic::{Request, Response, Status};

use super::service;
use crate::Error;

/// Makes the service.
pub(crate) fn server(pool: PgPool, gate: Arc<dyn Gate>) -> SettingsServiceServer<SettingsGrpc> {
    SettingsServiceServer::new(SettingsGrpc { pool, gate })
}

/// The handler.
pub(crate) struct SettingsGrpc {
    /// The database pool.
    pool: PgPool,
    /// The access gate.
    gate: Arc<dyn Gate>,
}

#[tonic::async_trait]
impl SettingsService for SettingsGrpc {
    async fn get_instance_settings(
        &self,
        request: Request<pb::GetInstanceSettingsRequest>,
    ) -> Result<Response<pb::GetInstanceSettingsResponse>, Status> {
        self.admin(&request, SettingsHolder::GET_INSTANCE_SETTINGS)
            .await?;
        let (settings, active) = service::get(&self.pool).await?;
        Ok(Response::new(pb::GetInstanceSettingsResponse {
            settings: Some(message(settings, active)),
        }))
    }

    async fn update_instance_settings(
        &self,
        request: Request<pb::UpdateInstanceSettingsRequest>,
    ) -> Result<Response<pb::UpdateInstanceSettingsResponse>, Status> {
        self.admin(&request, SettingsHolder::UPDATE_INSTANCE_SETTINGS)
            .await?;
        let request = request.into_inner();
        if request
            .workspace_limit
            .is_some_and(|limit| i32::try_from(limit).is_err())
        {
            return Err(Error::invalid("workspace_limit", "is too large").into());
        }
        let change = InstanceSettingsChange {
            personal_tokens_enabled: request.personal_tokens_enabled,
            workspace_limit: request.workspace_limit,
        };
        let (settings, active) = service::update(&self.pool, change).await?;
        Ok(Response::new(pb::UpdateInstanceSettingsResponse {
            settings: Some(message(settings, active)),
        }))
    }
}

impl SettingsGrpc {
    /// Authenticates the caller, and checks the route permission on the platform.
    async fn admin<T: Sync>(
        &self,
        request: &Request<T>,
        permission: Permission<Platform>,
    ) -> Result<(), Error> {
        let caller = self.gate.caller(request).await?;
        self.gate
            .authorize(&caller, permission.on(Platform::ID))
            .await?;
        Ok(())
    }
}

/// Makes the message of the settings.
fn message(settings: InstanceSettings, active: u32) -> pb::InstanceSettings {
    pb::InstanceSettings {
        personal_tokens_enabled: settings.personal_tokens_enabled,
        active_personal_tokens: active,
        workspace_limit: settings.workspace_limit,
    }
}
