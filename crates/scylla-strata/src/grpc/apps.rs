use super::{authed, parse};
use crate::{Apps, AppsRpc};
use scylla_core::grpc::convert::ts;
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::{app_access_to_proto, app_credential_to_proto, app_to_proto};
use scylla_proto::app::v1 as pb;
use strata_tonic::Grpc;

pub use super::service::app::app_auth_service_client::AppAuthServiceClient;
pub use super::service::app::app_auth_service_server::{AppAuthService, AppAuthServiceServer};
pub use super::service::app::app_service_client::AppServiceClient;
pub use super::service::app::app_service_server::{AppService, AppServiceServer};

strata_tonic::service! {
    impl<A: Apps> AppService for Grpc<A> {
        rpc create_app(pb::CreateAppRequest) -> pb::CreateAppResponse = |call, req| {
            authed(&call).await?;
            let created = AppsRpc::create_app(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateAppResponse { app: Some(app_to_proto(&created.app)), secret: created.secret.as_str().to_string() })
        }
        rpc get_app(pb::GetAppRequest) -> pb::GetAppResponse = |call, req| {
            authed(&call).await?;
            let app = AppsRpc::get_app(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::GetAppResponse { app: Some(app_to_proto(&app)) })
        }
        rpc list_apps(pb::ListAppsRequest) -> pb::ListAppsResponse = |call, req| {
            authed(&call).await?;
            let apps = AppsRpc::list_apps(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ListAppsResponse { apps: apps.iter().map(app_to_proto).collect() })
        }
        rpc delete_app(pb::DeleteAppRequest) -> pb::DeleteAppResponse = |call, req| {
            authed(&call).await?;
            AppsRpc::delete_app(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteAppResponse {})
        }
        rpc set_app_active(pb::SetAppActiveRequest) -> pb::SetAppActiveResponse = |call, req| {
            authed(&call).await?;
            let app = AppsRpc::set_app_active(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::SetAppActiveResponse { app: Some(app_to_proto(&app)) })
        }
        rpc create_app_secret(pb::CreateAppSecretRequest) -> pb::CreateAppSecretResponse = |call, req| {
            authed(&call).await?;
            let created = AppsRpc::create_app_secret(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateAppSecretResponse {
                app_secret: Some(app_credential_to_proto(&created.credential)),
                secret: created.secret.as_str().to_string(),
            })
        }
        rpc list_app_secrets(pb::ListAppSecretsRequest) -> pb::ListAppSecretsResponse = |call, req| {
            authed(&call).await?;
            let secrets = AppsRpc::list_app_secrets(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ListAppSecretsResponse { app_secrets: secrets.iter().map(app_credential_to_proto).collect() })
        }
        rpc revoke_app_secret(pb::RevokeAppSecretRequest) -> pb::RevokeAppSecretResponse = |call, req| {
            authed(&call).await?;
            AppsRpc::revoke_app_secret(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::RevokeAppSecretResponse {})
        }
        rpc set_app_secret_enabled(pb::SetAppSecretEnabledRequest) -> pb::SetAppSecretEnabledResponse = |call, req| {
            authed(&call).await?;
            let credential = AppsRpc::set_app_secret_enabled(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::SetAppSecretEnabledResponse { app_secret: Some(app_credential_to_proto(&credential)) })
        }
        rpc set_app_secret_label(pb::SetAppSecretLabelRequest) -> pb::SetAppSecretLabelResponse = |call, req| {
            authed(&call).await?;
            let credential = AppsRpc::set_app_secret_label(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::SetAppSecretLabelResponse { app_secret: Some(app_credential_to_proto(&credential)) })
        }
        rpc list_app_access(pb::ListAppAccessRequest) -> pb::ListAppAccessResponse = |call, req| {
            authed(&call).await?;
            let access = AppsRpc::list_app_access(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ListAppAccessResponse { access: access.iter().map(app_access_to_proto).collect() })
        }
    }
}

strata_tonic::service! {
    impl<A: Apps> AppAuthService for Grpc<A> {
        rpc issue_token(pb::IssueTokenRequest) -> pb::IssueTokenResponse = |call, req| {
            let token = AppsRpc::issue_app_token(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::IssueTokenResponse { token: token.token().to_string(), expires_at: ts(token.expires_at()) })
        }
    }
}
