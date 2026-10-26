//! Tests the Projects use case of Scylla, written with strata: CE in memory, and the gRPC
//! adapter over a real tonic server.

#![allow(clippy::unused_async_trait_impl)]

use scylla_core::application::project::{
    CreateProject, DeleteProject, GetProject, ListOrganizationProjects, ListProjectMembers,
    ListUserProjects, SetProjectActive, UpdateProject,
};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{OrganizationId, UserId};
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::project::ProjectName;
use scylla_proto::common::v1::OrganizationId as PbOrganizationId;
use scylla_proto::project::v1::CreateProjectRequest;
use scylla_strata::grpc::{ProjectServiceClient, ProjectServiceServer};
use scylla_strata::{Permissions, Projects, ProjectsRpc};
use strata::Contextual;
use strata_tonic::Grpc;
use tonic::Code;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

/// Grants each permission to a user, and none to the other callers.
#[derive(Default)]
pub struct UsersMayDoAll;

impl Permissions for UsersMayDoAll {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        match caller {
            CallerContext::User(_) => Ok(()),
            _ => Err(DomainError::forbidden(format!(
                "{} is not granted",
                permission.key()
            ))),
        }
    }
}

#[strata::edition(modules(scylla_strata::Access, scylla_strata::Projects))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = UsersMayDoAll;
}

const ANN: &str = "Bearer user:ann";

fn create(org: &str, name: &str) -> CreateProject {
    CreateProject {
        organization_id: OrganizationId::new(org),
        name: ProjectName::new(name).unwrap(),
        description: None,
    }
}

#[tokio::test]
async fn ce_runs_the_project_lifecycle_for_a_user() {
    let app = TestCe::default();
    let ann = app
        .request_context(&[("authorization", ANN)])
        .await
        .unwrap();
    let ann = app.scoped(&ann);

    let project = ProjectsRpc::create_project(&ann, create("acme", "web"))
        .await
        .unwrap();
    assert!(project.is_active());
    let mine = ProjectsRpc::list_user_projects(
        &ann,
        ListUserProjects {
            user_id: UserId::new("ann"),
            pagination: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        mine.items().len(),
        1,
        "the creator is the admin of the project"
    );
    let members = ProjectsRpc::list_project_members(
        &ann,
        ListProjectMembers {
            project_id: project.id().clone(),
            pagination: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(members.metadata().total_count(), 1);

    let renamed = ProjectsRpc::update_project(
        &ann,
        UpdateProject {
            id: project.id().clone(),
            name: Some(ProjectName::new("site").unwrap()),
            description: None,
        },
    )
    .await
    .unwrap();
    assert_eq!((renamed.name().as_str(), renamed.version()), ("site", 1));
    let off = ProjectsRpc::set_project_active(
        &ann,
        SetProjectActive {
            id: project.id().clone(),
            is_active: false,
        },
    )
    .await
    .unwrap();
    assert!(!off.is_active());
    let listed = ProjectsRpc::list_organization_projects(
        &ann,
        ListOrganizationProjects {
            organization_id: OrganizationId::new("acme"),
            pagination: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(listed.items().len(), 1);

    ProjectsRpc::delete_project(
        &ann,
        DeleteProject {
            id: project.id().clone(),
        },
    )
    .await
    .unwrap();
    let gone = ProjectsRpc::get_project(
        &ann,
        GetProject {
            id: project.id().clone(),
        },
    )
    .await
    .unwrap_err();
    assert!(gone.is_not_found(), "{gone}");
}

#[tokio::test]
async fn ce_refuses_a_caller_without_the_permission() {
    let app = TestCe::default();
    let refused = app.create_project(create("acme", "web")).await.unwrap_err();
    assert!(matches!(refused, DomainError::Forbidden(_)), "{refused}");
    let bad = app
        .request_context(&[("authorization", "Bearer nobody")])
        .await
        .unwrap();
    let refused = ProjectsRpc::create_project(&app.scoped(&bad), create("acme", "web"))
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, DomainError::Unauthorized(m) if m == "Invalid or expired token"),
        "{refused}"
    );
}

async fn serve<A: Projects>(app: A) -> ProjectServiceClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Server::builder().add_service(ProjectServiceServer::new(Grpc::new(app)));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    ProjectServiceClient::new(channel)
}

fn request(org: &str, name: &str, bearer: Option<&str>) -> tonic::Request<CreateProjectRequest> {
    let mut request = tonic::Request::new(CreateProjectRequest {
        organization_id: Some(PbOrganizationId {
            value: org.to_owned(),
        }),
        name: name.to_owned(),
        description: None,
    });
    if let Some(bearer) = bearer {
        request
            .metadata_mut()
            .insert("authorization", bearer.parse().unwrap());
    }
    request
}

#[tokio::test]
async fn grpc_serves_the_scylla_contract() {
    let mut client = serve(TestCe::default()).await;
    let created = client
        .create_project(request("acme", "web", Some(ANN)))
        .await
        .unwrap()
        .into_inner();
    let project = created.project.unwrap();
    assert_eq!((project.name.as_str(), project.is_active), ("web", true));
    let anonymous = client
        .create_project(request("beta", "web", None))
        .await
        .unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    assert_eq!(
        anonymous.message(),
        "Missing or invalid authorization token"
    );
    let bad_token = client
        .create_project(request("beta", "web", Some("Bearer x")))
        .await
        .unwrap_err();
    assert_eq!(bad_token.code(), Code::Unauthenticated, "{bad_token:?}");
    let empty_name = client
        .create_project(request("beta", " ", Some(ANN)))
        .await
        .unwrap_err();
    assert_eq!(empty_name.code(), Code::InvalidArgument, "{empty_name:?}");
}
