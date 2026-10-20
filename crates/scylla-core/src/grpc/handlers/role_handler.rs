use crate::application::RoleUseCases;
use crate::grpc::adapter::run;
use crate::grpc::mappers::{authz_action_to_proto, effective_scope_to_proto, role_to_proto};
use derive_more::Constructor;
use scylla_extension::Actions;
use scylla_proto::authz::v1::{
    CreateRoleRequest, CreateRoleResponse, DeleteRoleRequest, DeleteRoleResponse,
    GetEffectivePermissionsRequest, GetEffectivePermissionsResponse, GetMyPermissionsRequest,
    GetMyPermissionsResponse, GetRoleRequest, GetRoleResponse, ListAuthzVocabularyRequest,
    ListAuthzVocabularyResponse, ListRolesRequest, ListRolesResponse, UpdateRoleRequest,
    UpdateRoleResponse, role_service_server::RoleService,
};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Constructor)]
pub struct RoleHandler {
    actions: Arc<Actions>,
    roles: Arc<RoleUseCases>,
}

#[async_trait::async_trait]
impl RoleService for RoleHandler {
    async fn create_role(
        &self,
        request: Request<CreateRoleRequest>,
    ) -> Result<Response<CreateRoleResponse>, Status> {
        let role = run(&self.actions, &*self.roles, request).await?;
        Ok(Response::new(CreateRoleResponse {
            role: Some(role_to_proto(&role)),
        }))
    }

    async fn update_role(
        &self,
        request: Request<UpdateRoleRequest>,
    ) -> Result<Response<UpdateRoleResponse>, Status> {
        let role = run(&self.actions, &*self.roles, request).await?;
        Ok(Response::new(UpdateRoleResponse {
            role: Some(role_to_proto(&role)),
        }))
    }

    async fn delete_role(
        &self,
        request: Request<DeleteRoleRequest>,
    ) -> Result<Response<DeleteRoleResponse>, Status> {
        run(&self.actions, &*self.roles, request).await?;
        Ok(Response::new(DeleteRoleResponse {}))
    }

    async fn list_roles(
        &self,
        request: Request<ListRolesRequest>,
    ) -> Result<Response<ListRolesResponse>, Status> {
        let roles = run(&self.actions, &*self.roles, request).await?;
        Ok(Response::new(ListRolesResponse {
            roles: roles.iter().map(role_to_proto).collect(),
        }))
    }

    async fn get_role(
        &self,
        request: Request<GetRoleRequest>,
    ) -> Result<Response<GetRoleResponse>, Status> {
        let role = run(&self.actions, &*self.roles, request).await?;
        Ok(Response::new(GetRoleResponse {
            role: Some(role_to_proto(&role)),
        }))
    }

    async fn get_effective_permissions(
        &self,
        request: Request<GetEffectivePermissionsRequest>,
    ) -> Result<Response<GetEffectivePermissionsResponse>, Status> {
        let scopes = run(&self.actions, &*self.roles, request).await?;
        Ok(Response::new(GetEffectivePermissionsResponse {
            scopes: scopes.iter().map(effective_scope_to_proto).collect(),
        }))
    }

    async fn get_my_permissions(
        &self,
        request: Request<GetMyPermissionsRequest>,
    ) -> Result<Response<GetMyPermissionsResponse>, Status> {
        let scopes = run(&self.actions, &*self.roles, request).await?;
        Ok(Response::new(GetMyPermissionsResponse {
            scopes: scopes.iter().map(effective_scope_to_proto).collect(),
        }))
    }

    async fn list_authz_vocabulary(
        &self,
        request: Request<ListAuthzVocabularyRequest>,
    ) -> Result<Response<ListAuthzVocabularyResponse>, Status> {
        let actions = run(&self.actions, &*self.roles, request).await?;
        Ok(Response::new(ListAuthzVocabularyResponse {
            actions: actions.iter().map(authz_action_to_proto).collect(),
        }))
    }
}
