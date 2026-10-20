use crate::application::GrantUseCases;
use crate::grpc::adapter::run;
use crate::grpc::mappers::{grant_to_proto, grantable_role_to_proto};
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::authz::v1::{
    CreateGrantRequest, CreateGrantResponse, ListGrantableRolesRequest, ListGrantableRolesResponse,
    ListGrantsRequest, ListGrantsResponse, RevokeAllAccessRequest, RevokeAllAccessResponse,
    RevokeGrantRequest, RevokeGrantResponse, grant_service_server::GrantService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct GrantHandler {
    actions: Arc<Actions>,
    grants: Arc<GrantUseCases>,
}

#[async_trait::async_trait]
impl GrantService for GrantHandler {
    async fn create_grant(
        &self,
        request: Request<CreateGrantRequest>,
    ) -> Result<Response<CreateGrantResponse>, Status> {
        let grant = run(&self.actions, &*self.grants, request).await?;
        Ok(Response::new(CreateGrantResponse {
            grant: Some(grant_to_proto(&grant)),
        }))
    }

    async fn revoke_grant(
        &self,
        request: Request<RevokeGrantRequest>,
    ) -> Result<Response<RevokeGrantResponse>, Status> {
        run(&self.actions, &*self.grants, request).await?;
        Ok(Response::new(RevokeGrantResponse {}))
    }

    async fn revoke_all_access(
        &self,
        request: Request<RevokeAllAccessRequest>,
    ) -> Result<Response<RevokeAllAccessResponse>, Status> {
        let revoked = run(&self.actions, &*self.grants, request).await?;
        Ok(Response::new(RevokeAllAccessResponse { revoked }))
    }

    async fn list_grants(
        &self,
        request: Request<ListGrantsRequest>,
    ) -> Result<Response<ListGrantsResponse>, Status> {
        let grants = run(&self.actions, &*self.grants, request).await?;
        Ok(Response::new(ListGrantsResponse {
            grants: grants.iter().map(grant_to_proto).collect(),
        }))
    }

    async fn list_grantable_roles(
        &self,
        request: Request<ListGrantableRolesRequest>,
    ) -> Result<Response<ListGrantableRolesResponse>, Status> {
        let roles = run(&self.actions, &*self.grants, request).await?;
        Ok(Response::new(ListGrantableRolesResponse {
            roles: roles.iter().map(grantable_role_to_proto).collect(),
        }))
    }
}
