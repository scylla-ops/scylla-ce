use super::{authed, parse};
use crate::{Organizations, OrganizationsRpc};
use scylla_core::grpc::{domain_error_to_status, organization_to_proto};
use scylla_proto::organization::v1 as pb;
use strata_tonic::Grpc;

pub use super::service::organization::organization_service_client::OrganizationServiceClient;
pub use super::service::organization::organization_service_server::{
    OrganizationService, OrganizationServiceServer,
};

strata_tonic::service! {
    impl<A: Organizations> OrganizationService for Grpc<A> {
        rpc create_organization(pb::CreateOrganizationRequest) -> pb::CreateOrganizationResponse = |call, req| {
            authed(&call).await?;
            let organization = OrganizationsRpc::create_organization(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateOrganizationResponse { organization: Some(organization_to_proto(&organization)) })
        }
        rpc get_organization(pb::GetOrganizationRequest) -> pb::GetOrganizationResponse = |call, req| {
            authed(&call).await?;
            let organization = OrganizationsRpc::get_organization(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::GetOrganizationResponse { organization: Some(organization_to_proto(&organization)) })
        }
        rpc update_organization(pb::UpdateOrganizationRequest) -> pb::UpdateOrganizationResponse = |call, req| {
            authed(&call).await?;
            let organization = OrganizationsRpc::update_organization(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::UpdateOrganizationResponse { organization: Some(organization_to_proto(&organization)) })
        }
        rpc set_organization_active(pb::SetOrganizationActiveRequest) -> pb::SetOrganizationActiveResponse = |call, req| {
            authed(&call).await?;
            let organization = OrganizationsRpc::set_organization_active(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::SetOrganizationActiveResponse { organization: Some(organization_to_proto(&organization)) })
        }
        rpc delete_organization(pb::DeleteOrganizationRequest) -> pb::DeleteOrganizationResponse = |call, req| {
            authed(&call).await?;
            OrganizationsRpc::delete_organization(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteOrganizationResponse {})
        }
        rpc list_organizations(pb::ListOrganizationsRequest) -> pb::ListOrganizationsResponse = |call, req| {
            authed(&call).await?;
            Ok(OrganizationsRpc::list_organizations(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?.into())
        }
        rpc list_organization_members(pb::ListOrganizationMembersRequest) -> pb::ListOrganizationMembersResponse = |call, req| {
            authed(&call).await?;
            Ok(OrganizationsRpc::list_organization_members(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?.into())
        }
        rpc list_user_organizations(pb::ListUserOrganizationsRequest) -> pb::ListUserOrganizationsResponse = |call, req| {
            authed(&call).await?;
            Ok(OrganizationsRpc::list_user_organizations(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?.into())
        }
    }
}
