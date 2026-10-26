use super::{authed, parse};
use crate::{Grants, GrantsRpc};
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::{grant_to_proto, grantable_role_to_proto};
use scylla_proto::authz::v1 as pb;
use strata_tonic::Grpc;

pub use super::service::authz::grant_service_client::GrantServiceClient;
pub use super::service::authz::grant_service_server::{GrantService, GrantServiceServer};

strata_tonic::service! {
    impl<A: Grants> GrantService for Grpc<A> {
        rpc create_grant(pb::CreateGrantRequest) -> pb::CreateGrantResponse = |call, req| {
            authed(&call).await?;
            let grant = GrantsRpc::create_grant(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateGrantResponse { grant: Some(grant_to_proto(&grant)) })
        }
        rpc revoke_grant(pb::RevokeGrantRequest) -> pb::RevokeGrantResponse = |call, req| {
            authed(&call).await?;
            GrantsRpc::revoke_grant(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::RevokeGrantResponse {})
        }
        rpc revoke_all_access(pb::RevokeAllAccessRequest) -> pb::RevokeAllAccessResponse = |call, req| {
            authed(&call).await?;
            let revoked = GrantsRpc::revoke_all_access(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::RevokeAllAccessResponse { revoked })
        }
        rpc list_grants(pb::ListGrantsRequest) -> pb::ListGrantsResponse = |call, req| {
            authed(&call).await?;
            let grants = GrantsRpc::list_grants(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ListGrantsResponse { grants: grants.iter().map(grant_to_proto).collect() })
        }
        rpc list_grantable_roles(pb::ListGrantableRolesRequest) -> pb::ListGrantableRolesResponse = |call, req| {
            authed(&call).await?;
            let roles = GrantsRpc::list_grantable_roles(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ListGrantableRolesResponse { roles: roles.iter().map(grantable_role_to_proto).collect() })
        }
    }
}
