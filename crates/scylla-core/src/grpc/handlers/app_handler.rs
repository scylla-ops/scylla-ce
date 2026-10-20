use crate::application::AppUseCases;
use crate::grpc::adapter::run;
use crate::grpc::mappers::{app_access_to_proto, app_credential_to_proto, app_to_proto};
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::app::v1::{
    CreateAppRequest, CreateAppResponse, CreateAppSecretRequest, CreateAppSecretResponse,
    DeleteAppRequest, DeleteAppResponse, GetAppRequest, GetAppResponse, ListAppAccessRequest,
    ListAppAccessResponse, ListAppSecretsRequest, ListAppSecretsResponse, ListAppsRequest,
    ListAppsResponse, RevokeAppSecretRequest, RevokeAppSecretResponse, SetAppActiveRequest,
    SetAppActiveResponse, SetAppSecretEnabledRequest, SetAppSecretEnabledResponse,
    SetAppSecretLabelRequest, SetAppSecretLabelResponse, app_service_server::AppService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct AppHandler {
    actions: Arc<Actions>,
    apps: Arc<AppUseCases>,
}

#[async_trait::async_trait]
impl AppService for AppHandler {
    async fn create_app(
        &self,
        request: Request<CreateAppRequest>,
    ) -> Result<Response<CreateAppResponse>, Status> {
        let created = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(CreateAppResponse {
            app: Some(app_to_proto(&created.app)),
            secret: created.secret.as_str().to_string(),
        }))
    }

    async fn get_app(
        &self,
        request: Request<GetAppRequest>,
    ) -> Result<Response<GetAppResponse>, Status> {
        let app = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(GetAppResponse {
            app: Some(app_to_proto(&app)),
        }))
    }

    async fn list_apps(
        &self,
        request: Request<ListAppsRequest>,
    ) -> Result<Response<ListAppsResponse>, Status> {
        let apps = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(ListAppsResponse {
            apps: apps.iter().map(app_to_proto).collect(),
        }))
    }

    async fn delete_app(
        &self,
        request: Request<DeleteAppRequest>,
    ) -> Result<Response<DeleteAppResponse>, Status> {
        run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(DeleteAppResponse {}))
    }

    async fn set_app_active(
        &self,
        request: Request<SetAppActiveRequest>,
    ) -> Result<Response<SetAppActiveResponse>, Status> {
        let app = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(SetAppActiveResponse {
            app: Some(app_to_proto(&app)),
        }))
    }

    async fn create_app_secret(
        &self,
        request: Request<CreateAppSecretRequest>,
    ) -> Result<Response<CreateAppSecretResponse>, Status> {
        let created = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(CreateAppSecretResponse {
            app_secret: Some(app_credential_to_proto(&created.credential)),
            secret: created.secret.as_str().to_string(),
        }))
    }

    async fn list_app_secrets(
        &self,
        request: Request<ListAppSecretsRequest>,
    ) -> Result<Response<ListAppSecretsResponse>, Status> {
        let secrets = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(ListAppSecretsResponse {
            app_secrets: secrets.iter().map(app_credential_to_proto).collect(),
        }))
    }

    async fn revoke_app_secret(
        &self,
        request: Request<RevokeAppSecretRequest>,
    ) -> Result<Response<RevokeAppSecretResponse>, Status> {
        run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(RevokeAppSecretResponse {}))
    }

    async fn set_app_secret_enabled(
        &self,
        request: Request<SetAppSecretEnabledRequest>,
    ) -> Result<Response<SetAppSecretEnabledResponse>, Status> {
        let credential = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(SetAppSecretEnabledResponse {
            app_secret: Some(app_credential_to_proto(&credential)),
        }))
    }

    async fn set_app_secret_label(
        &self,
        request: Request<SetAppSecretLabelRequest>,
    ) -> Result<Response<SetAppSecretLabelResponse>, Status> {
        let credential = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(SetAppSecretLabelResponse {
            app_secret: Some(app_credential_to_proto(&credential)),
        }))
    }

    async fn list_app_access(
        &self,
        request: Request<ListAppAccessRequest>,
    ) -> Result<Response<ListAppAccessResponse>, Status> {
        let access = run(&self.actions, &*self.apps, request).await?;
        Ok(Response::new(ListAppAccessResponse {
            access: access.iter().map(app_access_to_proto).collect(),
        }))
    }
}
