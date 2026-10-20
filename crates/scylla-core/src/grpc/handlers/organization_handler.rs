//! The adapter: each RPC is one `run` and its response. Parsing lives in the
//! organization mapper, behind `Parse`; no RPC checks a permission or touches a port.

use crate::application::OrganizationUseCases;
use crate::grpc::adapter::run;
use crate::grpc::mappers::organization_to_proto;
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::organization::v1::{
    CreateOrganizationRequest, CreateOrganizationResponse, DeleteOrganizationRequest,
    DeleteOrganizationResponse, GetOrganizationRequest, GetOrganizationResponse,
    ListOrganizationMembersRequest, ListOrganizationMembersResponse, ListOrganizationsRequest,
    ListOrganizationsResponse, ListUserOrganizationsRequest, ListUserOrganizationsResponse,
    SetOrganizationActiveRequest, SetOrganizationActiveResponse, UpdateOrganizationRequest,
    UpdateOrganizationResponse, organization_service_server::OrganizationService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct OrganizationHandler {
    actions: Arc<Actions>,
    organizations: Arc<OrganizationUseCases>,
}

#[async_trait::async_trait]
impl OrganizationService for OrganizationHandler {
    async fn create_organization(
        &self,
        request: Request<CreateOrganizationRequest>,
    ) -> Result<Response<CreateOrganizationResponse>, Status> {
        let organization = run(&self.actions, &*self.organizations, request).await?;
        Ok(Response::new(CreateOrganizationResponse {
            organization: Some(organization_to_proto(&organization)),
        }))
    }

    async fn get_organization(
        &self,
        request: Request<GetOrganizationRequest>,
    ) -> Result<Response<GetOrganizationResponse>, Status> {
        let organization = run(&self.actions, &*self.organizations, request).await?;
        Ok(Response::new(GetOrganizationResponse {
            organization: Some(organization_to_proto(&organization)),
        }))
    }

    async fn update_organization(
        &self,
        request: Request<UpdateOrganizationRequest>,
    ) -> Result<Response<UpdateOrganizationResponse>, Status> {
        let organization = run(&self.actions, &*self.organizations, request).await?;
        Ok(Response::new(UpdateOrganizationResponse {
            organization: Some(organization_to_proto(&organization)),
        }))
    }

    async fn set_organization_active(
        &self,
        request: Request<SetOrganizationActiveRequest>,
    ) -> Result<Response<SetOrganizationActiveResponse>, Status> {
        let organization = run(&self.actions, &*self.organizations, request).await?;
        Ok(Response::new(SetOrganizationActiveResponse {
            organization: Some(organization_to_proto(&organization)),
        }))
    }

    async fn delete_organization(
        &self,
        request: Request<DeleteOrganizationRequest>,
    ) -> Result<Response<DeleteOrganizationResponse>, Status> {
        run(&self.actions, &*self.organizations, request).await?;
        Ok(Response::new(DeleteOrganizationResponse {}))
    }

    async fn list_organizations(
        &self,
        request: Request<ListOrganizationsRequest>,
    ) -> Result<Response<ListOrganizationsResponse>, Status> {
        let page = run(&self.actions, &*self.organizations, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_organization_members(
        &self,
        request: Request<ListOrganizationMembersRequest>,
    ) -> Result<Response<ListOrganizationMembersResponse>, Status> {
        let page = run(&self.actions, &*self.organizations, request).await?;
        Ok(Response::new(page.into()))
    }

    async fn list_user_organizations(
        &self,
        request: Request<ListUserOrganizationsRequest>,
    ) -> Result<Response<ListUserOrganizationsResponse>, Status> {
        let page = run(&self.actions, &*self.organizations, request).await?;
        Ok(Response::new(page.into()))
    }
}
