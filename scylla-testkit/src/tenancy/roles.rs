//! The `TestApp` helpers of `RoleService`, and of the roles of the platform level.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::Platform;
use scylla_proto::access::v1::role_service_client::RoleServiceClient;
use scylla_proto::access::v1::{
    CatalogArea, CreateRoleRequest, DeleteRoleRequest, GetRoleRequest,
    ListPermissionCatalogRequest, ListRoleHoldersRequest, ListRoleHoldersResponse,
    ListRolesRequest, Role as RoleMessage, UpdateRoleRequest,
};
use tonic::transport::Channel;
use tonic::Status;

use super::access::Role;
use crate::{id, TestApp};

/// The fields of a new role, as text, so a test can send a bad value.
#[derive(Clone, Copy, Debug)]
pub struct RoleInput<'a> {
    /// The name.
    pub name: &'a str,
    /// The level.
    pub level: &'a str,
    /// Who can hold the role.
    pub holder: &'a str,
    /// The ids of the permissions.
    pub permissions: &'a [&'a str],
    /// The workspace that owns the role.
    pub workspace: Option<&'a str>,
}

impl<'a> RoleInput<'a> {
    /// Names a role of the instance for people, on the platform level.
    pub fn on_platform(name: &'a str, permissions: &'a [&'a str]) -> Self {
        Self {
            name,
            level: "platform",
            holder: "people",
            permissions,
            workspace: None,
        }
    }
}

/// Makes a list of texts.
fn texts(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

impl TestApp {
    /// A client of `RoleService`.
    fn roles(&self) -> RoleServiceClient<Channel> {
        RoleServiceClient::new(self.channel.clone())
    }

    /// Gives a user a role that only shows the workspace of `owner`, so it can get a role on a
    /// project of that workspace. It reads no project.
    pub async fn join_workspace(&self, owner: ActorId, user: ActorId) {
        let workspace = self.workspace_of(owner).await.to_string();
        let name = format!("Joined {user}");
        let input = RoleInput {
            name: &name,
            level: "workspace",
            holder: "people",
            permissions: &["workspace.view"],
            workspace: Some(&workspace),
        };
        let role = self.create_role(owner, input).await.expect("a new role");
        let grant = Role::on("workspace", &workspace, &role.grant_name);
        self.grant_role(owner, grant, &user.to_string())
            .await
            .expect("a grant");
    }

    /// Calls `ListPermissionCatalog` as `actor`.
    pub async fn permission_catalog(&self, actor: ActorId) -> Result<Vec<CatalogArea>, Status> {
        let request = self.authed(actor, ListPermissionCatalogRequest {}).await;
        let response = self.roles().list_permission_catalog(request).await?;
        Ok(response.into_inner().areas)
    }

    /// Calls `ListRoles` as `actor`.
    pub async fn list_roles(
        &self,
        actor: ActorId,
        workspace: Option<&str>,
    ) -> Result<Vec<RoleMessage>, Status> {
        let request = ListRolesRequest {
            workspace_id: workspace.map(id),
        };
        let request = self.authed(actor, request).await;
        Ok(self.roles().list_roles(request).await?.into_inner().roles)
    }

    /// Calls `GetRole` as `actor`.
    pub async fn get_role(
        &self,
        actor: ActorId,
        key: &str,
        workspace: Option<&str>,
    ) -> Result<RoleMessage, Status> {
        let request = GetRoleRequest {
            key: key.to_owned(),
            workspace_id: workspace.map(id),
        };
        let request = self.authed(actor, request).await;
        let response = self.roles().get_role(request).await?.into_inner();
        Ok(response.role.expect("role in response"))
    }

    /// Calls `CreateRole` as `actor`.
    pub async fn create_role(
        &self,
        actor: ActorId,
        input: RoleInput<'_>,
    ) -> Result<RoleMessage, Status> {
        let request = CreateRoleRequest {
            name: input.name.to_owned(),
            description: String::new(),
            level: input.level.to_owned(),
            holder: input.holder.to_owned(),
            permissions: texts(input.permissions),
            workspace_id: input.workspace.map(id),
        };
        let request = self.authed(actor, request).await;
        let response = self.roles().create_role(request).await?.into_inner();
        Ok(response.role.expect("role in response"))
    }

    /// Calls `UpdateRole` as `actor`.
    pub async fn update_role(
        &self,
        actor: ActorId,
        role: &str,
        name: &str,
        permissions: &[&str],
    ) -> Result<RoleMessage, Status> {
        let request = UpdateRoleRequest {
            role_id: Some(id(role)),
            name: name.to_owned(),
            description: String::new(),
            permissions: texts(permissions),
        };
        let request = self.authed(actor, request).await;
        let response = self.roles().update_role(request).await?.into_inner();
        Ok(response.role.expect("role in response"))
    }

    /// Calls `DeleteRole` as `actor`.
    pub async fn delete_role(&self, actor: ActorId, role: &str) -> Result<(), Status> {
        self.delete_role_moving(actor, role, "").await
    }

    /// Calls `DeleteRole` as `actor`, with the role that the holders get.
    pub async fn delete_role_moving(
        &self,
        actor: ActorId,
        role: &str,
        move_to: &str,
    ) -> Result<(), Status> {
        let request = DeleteRoleRequest {
            role_id: Some(id(role)),
            move_to: move_to.to_owned(),
        };
        let request = self.authed(actor, request).await;
        self.roles().delete_role(request).await?;
        Ok(())
    }

    /// Calls `ListRoleHolders` as `actor`.
    pub async fn role_holders(
        &self,
        actor: ActorId,
        key: &str,
        workspace: Option<&str>,
    ) -> Result<ListRoleHoldersResponse, Status> {
        let request = ListRoleHoldersRequest {
            key: key.to_owned(),
            workspace_id: workspace.map(id),
        };
        let request = self.authed(actor, request).await;
        Ok(self.roles().list_role_holders(request).await?.into_inner())
    }

    /// Calls `Grant` on the platform as `actor`: gives a role of the platform level.
    pub async fn grant_on_platform(
        &self,
        actor: ActorId,
        role: &str,
        to: ActorId,
    ) -> Result<(), Status> {
        let platform = Platform::ID.to_string();
        let role = Role::on("platform", &platform, role);
        self.grant_role(actor, role, &to.to_string()).await
    }

    /// Calls `Revoke` on the platform as `actor`: takes back a role of the platform level.
    pub async fn revoke_on_platform(
        &self,
        actor: ActorId,
        role: &str,
        from: ActorId,
    ) -> Result<(), Status> {
        let platform = Platform::ID.to_string();
        let role = Role::on("platform", &platform, role);
        self.revoke_role(actor, role, &from.to_string()).await
    }
}
