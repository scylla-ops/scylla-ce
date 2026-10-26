use super::{authed, parse};
use crate::{Roles, RolesRpc};
use scylla_core::grpc::domain_error_to_status;
use scylla_core::grpc::mappers::{authz_action_to_proto, effective_scope_to_proto, role_to_proto};
use scylla_proto::authz::v1 as pb;
use strata_tonic::Grpc;

pub use super::service::authz::role_service_client::RoleServiceClient;
pub use super::service::authz::role_service_server::{RoleService, RoleServiceServer};

strata_tonic::service! {
    impl<A: Roles> RoleService for Grpc<A> {
        rpc create_role(pb::CreateRoleRequest) -> pb::CreateRoleResponse = |call, req| {
            authed(&call).await?;
            let role = RolesRpc::create_role(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::CreateRoleResponse { role: Some(role_to_proto(&role)) })
        }
        rpc update_role(pb::UpdateRoleRequest) -> pb::UpdateRoleResponse = |call, req| {
            authed(&call).await?;
            let role = RolesRpc::update_role(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::UpdateRoleResponse { role: Some(role_to_proto(&role)) })
        }
        rpc delete_role(pb::DeleteRoleRequest) -> pb::DeleteRoleResponse = |call, req| {
            authed(&call).await?;
            RolesRpc::delete_role(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::DeleteRoleResponse {})
        }
        rpc list_roles(pb::ListRolesRequest) -> pb::ListRolesResponse = |call, req| {
            authed(&call).await?;
            let roles = RolesRpc::list_roles(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ListRolesResponse { roles: roles.iter().map(role_to_proto).collect() })
        }
        rpc get_role(pb::GetRoleRequest) -> pb::GetRoleResponse = |call, req| {
            authed(&call).await?;
            let role = RolesRpc::get_role(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::GetRoleResponse { role: Some(role_to_proto(&role)) })
        }
        rpc get_effective_permissions(pb::GetEffectivePermissionsRequest) -> pb::GetEffectivePermissionsResponse = |call, req| {
            authed(&call).await?;
            let scopes = RolesRpc::get_effective_permissions(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::GetEffectivePermissionsResponse { scopes: scopes.iter().map(effective_scope_to_proto).collect() })
        }
        rpc get_my_permissions(pb::GetMyPermissionsRequest) -> pb::GetMyPermissionsResponse = |call, req| {
            authed(&call).await?;
            let scopes = RolesRpc::get_my_permissions(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::GetMyPermissionsResponse { scopes: scopes.iter().map(effective_scope_to_proto).collect() })
        }
        rpc list_authz_vocabulary(pb::ListAuthzVocabularyRequest) -> pb::ListAuthzVocabularyResponse = |call, req| {
            authed(&call).await?;
            let actions = RolesRpc::list_authz_vocabulary(&call.scoped(), parse(req)?).await.map_err(domain_error_to_status)?;
            Ok(pb::ListAuthzVocabularyResponse { actions: actions.iter().map(authz_action_to_proto).collect() })
        }
    }
}
