//! One `TestApp` helper per core RPC, and the helpers that make requests.

use scylla_domain::actors::ActorId;
use scylla_domain::workspaces::WorkspaceId;
use scylla_proto::access::v1::access_service_client::AccessServiceClient;
use scylla_proto::access::v1::{GrantRequest, ObjectRef, RevokeRequest};
use scylla_proto::capabilities::v1::capability_service_client::CapabilityServiceClient;
use scylla_proto::capabilities::v1::{GetCapabilitiesRequest, GetCapabilitiesResponse};
use scylla_proto::common::v1 as common;
use scylla_proto::projects::v1::{
    CreateProjectRequest, DeleteProjectRequest, GetProjectRequest, ListProjectsRequest,
    ListProjectsResponse, Project,
};
use tonic::metadata::MetadataValue;
use tonic::{Request, Status};
use tonic_health::pb::health_check_response::ServingStatus;
use tonic_health::pb::health_client::HealthClient;
use tonic_health::pb::HealthCheckRequest;

use crate::TestApp;

/// A role on an object, as text, so a test can send a bad value: `Role::on("project", id,
/// "viewer")`.
#[derive(Clone, Copy, Debug)]
pub struct Role<'a> {
    /// The kind of the object.
    pub kind: &'a str,
    /// The id of the object.
    pub id: &'a str,
    /// The name of the role.
    pub name: &'a str,
}

impl<'a> Role<'a> {
    /// Names a role on an object.
    pub fn on(kind: &'a str, id: &'a str, name: &'a str) -> Self {
        Self { kind, id, name }
    }

    /// Makes the object message.
    fn object(&self) -> Option<ObjectRef> {
        Some(ObjectRef {
            kind: self.kind.to_owned(),
            id: Some(id(self.id)),
        })
    }
}

impl TestApp {
    /// Calls `CreateProject` as `actor`, in the personal workspace of the actor.
    pub async fn create_project(&self, actor: ActorId, name: &str) -> Result<Project, Status> {
        let workspace = self.workspace_of(actor).await;
        self.create_project_in(actor, workspace, name).await
    }

    /// Calls `CreateProject` as `actor`, in `workspace`.
    pub async fn create_project_in(
        &self,
        actor: ActorId,
        workspace: WorkspaceId,
        name: &str,
    ) -> Result<Project, Status> {
        let request = CreateProjectRequest {
            workspace_id: Some(id(workspace.to_string())),
            name: name.to_owned(),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().create_project(request).await?;
        Ok(response.into_inner().project.expect("project in response"))
    }

    /// Calls `GetProject` as `actor`. The id is text, so a test can send a bad id.
    pub async fn get_project(&self, actor: ActorId, project_id: &str) -> Result<Project, Status> {
        let request = GetProjectRequest {
            project_id: Some(id(project_id)),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().get_project(request).await?;
        Ok(response.into_inner().project.expect("project in response"))
    }

    /// Calls `ListProjects` as `actor`, for all the projects that the actor can see.
    pub async fn list_projects(
        &self,
        actor: ActorId,
        size: i32,
        token: &str,
    ) -> Result<ListProjectsResponse, Status> {
        let request = ListProjectsRequest {
            workspace_id: None,
            page: Some(common::PageRequest {
                size,
                token: token.to_owned(),
            }),
        };
        let request = self.authed(actor, request).await;
        Ok(self
            .projects
            .clone()
            .list_projects(request)
            .await?
            .into_inner())
    }

    /// Calls `ListProjects` as `actor` for one page of 100, of one workspace or of all, and
    /// returns the names of the projects in list order.
    pub async fn visible_projects(
        &self,
        actor: ActorId,
        workspace: Option<WorkspaceId>,
    ) -> Result<Vec<String>, Status> {
        let request = ListProjectsRequest {
            workspace_id: workspace.map(|workspace| id(workspace.to_string())),
            page: Some(common::PageRequest {
                size: 100,
                token: String::new(),
            }),
        };
        let request = self.authed(actor, request).await;
        let response = self.projects.clone().list_projects(request).await?;
        let projects = response.into_inner().projects;
        Ok(projects.into_iter().map(|project| project.name).collect())
    }

    /// Calls `DeleteProject` as `actor`. The id is text, so a test can send a bad id.
    pub async fn delete_project(&self, actor: ActorId, project_id: &str) -> Result<(), Status> {
        let request = DeleteProjectRequest {
            project_id: Some(id(project_id)),
        };
        let request = self.authed(actor, request).await;
        self.projects.clone().delete_project(request).await?;
        Ok(())
    }

    /// Calls `Grant` as `actor`: gives the role to the actor `to`, as text.
    pub async fn grant_role(&self, actor: ActorId, role: Role<'_>, to: &str) -> Result<(), Status> {
        let request = GrantRequest {
            object: role.object(),
            role: role.name.to_owned(),
            actor_id: Some(id(to)),
        };
        let request = self.authed(actor, request).await;
        AccessServiceClient::new(self.channel.clone())
            .grant(request)
            .await?;
        Ok(())
    }

    /// Calls `Revoke` as `actor`: takes back the role from the actor `from`, as text.
    pub async fn revoke_role(
        &self,
        actor: ActorId,
        role: Role<'_>,
        from: &str,
    ) -> Result<(), Status> {
        let request = RevokeRequest {
            object: role.object(),
            role: role.name.to_owned(),
            actor_id: Some(id(from)),
        };
        let request = self.authed(actor, request).await;
        AccessServiceClient::new(self.channel.clone())
            .revoke(request)
            .await?;
        Ok(())
    }

    /// Calls `GetCapabilities`, with no token.
    pub async fn get_capabilities(&self) -> Result<GetCapabilitiesResponse, Status> {
        let mut client = CapabilityServiceClient::new(self.channel.clone());
        let response = client.get_capabilities(GetCapabilitiesRequest {}).await?;
        Ok(response.into_inner())
    }

    /// Calls the standard health check for the whole server.
    pub async fn health_check(&self) -> Result<ServingStatus, Status> {
        let mut client = HealthClient::new(self.channel.clone());
        let request = HealthCheckRequest {
            service: String::new(),
        };
        Ok(client.check(request).await?.into_inner().status())
    }
}

/// Makes a request with a token in `authorization: Bearer`.
pub fn with_token<M>(token: &str, message: M) -> Request<M> {
    let mut request = Request::new(message);
    let value = MetadataValue::try_from(format!("Bearer {token}")).expect("ascii token");
    request.metadata_mut().insert("authorization", value);
    request
}

/// Makes an id message from text.
pub fn id(value: impl Into<String>) -> common::Id {
    common::Id {
        value: value.into(),
    }
}

/// Returns the text of an id of a response.
pub fn text(id: &Option<common::Id>) -> &str {
    &id.as_ref().expect("id in response").value
}
