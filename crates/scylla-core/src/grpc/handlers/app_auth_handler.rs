use crate::application::AppTokenUseCases;
use crate::grpc::adapter::run_public;
use crate::grpc::convert::ts;
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::app::v1::{
    IssueTokenRequest, IssueTokenResponse, app_auth_service_server::AppAuthService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct AppAuthHandler {
    actions: Arc<Actions>,
    tokens: Arc<AppTokenUseCases>,
}

#[async_trait::async_trait]
impl AppAuthService for AppAuthHandler {
    async fn issue_token(
        &self,
        request: Request<IssueTokenRequest>,
    ) -> Result<Response<IssueTokenResponse>, Status> {
        let token = run_public(&self.actions, &*self.tokens, request).await?;
        Ok(Response::new(IssueTokenResponse {
            token: token.token().to_string(),
            expires_at: ts(token.expires_at()),
        }))
    }
}
