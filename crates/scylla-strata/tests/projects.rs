//! Tests the Projects use case of Scylla, written with strata: CE in memory, and the gRPC
//! adapter over a real tonic server.

#![allow(clippy::unused_async_trait_impl)]

use scylla_auth::authz::{Grant, PermissionService, Visibility};
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::project::{
    CreateProject, DeleteProject, GetProject, ListOrganizationProjects, ListProjectMembers,
    ListUserProjects, SetProjectActive, UpdateProject,
};
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::jobs::JobBuilder;
use scylla_core::test_support::pipelines::PipelineBuilder;
use scylla_core::test_support::stubs::{ScopesByAgent, alice};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{JobId, OrganizationId, ProjectId, UserId};
use scylla_domain::domain::job::Job;
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::project::{Project, ProjectName};
use scylla_domain::domain::user::User;
use scylla_proto::common::v1::OrganizationId as PbOrganizationId;
use scylla_proto::project::v1::CreateProjectRequest;
use scylla_strata::grpc::{ProjectServiceClient, ProjectServiceServer};
use scylla_strata::{
    AccessModel, Directory, Jobs, Permissions, ProjectStore, Projects, ProjectsRpc, Scopes,
    ScyllaCtx,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use strata::{BuildCx, BuildError, Component, Contextual, Edition, Resource};
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

/// The rows of the store, the owner grants it wrote and the visibility of each list.
#[derive(Default)]
pub struct RecordingStore {
    rows: Mutex<Vec<Project>>,
    grants: Mutex<Vec<Grant>>,
    listed_by_organization: Mutex<Vec<Visibility>>,
    listed_for_user: Mutex<Vec<String>>,
}

impl RecordingStore {
    fn row(&self, id: &ProjectId) -> Option<Project> {
        self.rows
            .lock()
            .unwrap()
            .iter()
            .find(|p| p.id() == id)
            .cloned()
    }

    fn is_empty(&self) -> bool {
        self.rows.lock().unwrap().is_empty()
    }
}

fn empty<T>() -> PaginatedResult<T> {
    PaginatedResult::new(Vec::new(), &PaginationParams::default(), 0)
}

impl ProjectStore for RecordingStore {
    async fn create(&self, project: &Project) -> DomainResult<Project> {
        self.rows.lock().unwrap().push(project.clone());
        Ok(project.clone())
    }
    async fn provision_with_owner(&self, project: &Project, grant: &Grant) -> DomainResult<()> {
        self.create(project).await?;
        self.grants.lock().unwrap().push(grant.clone());
        Ok(())
    }
    async fn find_by_id(&self, id: &ProjectId) -> DomainResult<Project> {
        self.row(id)
            .ok_or_else(|| DomainError::not_found("Project", id))
    }
    async fn update(&self, project: &Project) -> DomainResult<Project> {
        let mut rows = self.rows.lock().unwrap();
        let row = rows
            .iter_mut()
            .find(|p| p.id() == project.id())
            .ok_or_else(|| DomainError::not_found("Project", project.id()))?;
        *row = project.clone();
        Ok(project.clone())
    }
    async fn delete(&self, project: &Project) -> DomainResult<()> {
        self.rows.lock().unwrap().retain(|p| p.id() != project.id());
        Ok(())
    }
    async fn list_all(
        &self,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        Ok(empty())
    }
    async fn list_by_organization(
        &self,
        _: &OrganizationId,
        _: Option<&PaginationParams>,
        visible: &Visibility,
    ) -> DomainResult<PaginatedResult<Project>> {
        self.listed_by_organization
            .lock()
            .unwrap()
            .push(visible.clone());
        Ok(empty())
    }
    async fn list_principals(
        &self,
        _: &ProjectId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>> {
        Ok(empty())
    }
    async fn list_for_user(
        &self,
        _: &UserId,
        permission: &str,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Project>> {
        self.listed_for_user
            .lock()
            .unwrap()
            .push(permission.to_string());
        Ok(empty())
    }
}

/// The visibility of each permission key; `All` for a key it does not hold.
#[derive(Clone, Default)]
pub struct Seen(Arc<HashMap<&'static str, Visibility>>);

impl Resource for Seen {
    const SECTION: &'static str = "seen";
    async fn build(_: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(Seen::default())
    }
}

pub struct SeenScopes(Seen);

impl Component for SeenScopes {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        Ok(SeenScopes(cx.resource::<Seen>().await?))
    }
}

impl Scopes for SeenScopes {
    async fn visible(&self, _: &CallerContext, permission: &str) -> DomainResult<Visibility> {
        Ok(self.0.0.get(permission).cloned().unwrap_or(Visibility::All))
    }
}

#[derive(Default)]
pub struct NoUsers;

impl Directory for NoUsers {
    async fn users(&self, _: PaginatedResult<UserId>) -> DomainResult<PaginatedResult<User>> {
        Ok(empty())
    }
}

/// The live jobs of the project, and the jobs it stopped.
#[derive(Default)]
pub struct LiveJobs {
    live: Mutex<Vec<Job>>,
    stopped: Mutex<Vec<JobId>>,
}

impl Jobs for LiveJobs {
    async fn live_in_project(&self, _: &ProjectId) -> DomainResult<Vec<Job>> {
        Ok(self.live.lock().unwrap().clone())
    }
    async fn stop(&self, jobs: &[Job]) {
        self.stopped
            .lock()
            .unwrap()
            .extend(jobs.iter().map(|j| j.id().clone()));
    }
}

#[strata::edition(
    extends = TestBase,
    overrides(scylla_strata::Access, scylla_strata::Projects)
)]
pub struct StubCe;

#[strata::overrides]
impl scylla_strata::Access for StubCe {
    type Permissions = scylla_strata::CedarPermissions;
}

#[strata::overrides]
impl scylla_strata::Projects for StubCe {
    type Store = RecordingStore;
    type Scopes = SeenScopes;
    type Directory = NoUsers;
    type Jobs = LiveJobs;
}

/// A veto of each write and each read of the projects, as a quota or a kill switch does.
#[strata::module(section = "veto")]
pub trait Veto: scylla_strata::Projects {
    #[before(scylla_strata::Projects::writes)]
    async fn veto_write(&self, usecase: &'static str) -> DomainResult<()> {
        Err(DomainError::quota_exceeded(format!("vetoed {usecase}")))
    }

    #[before(scylla_strata::Projects::reads)]
    async fn veto_read(&self, usecase: &'static str) -> DomainResult<()> {
        Err(DomainError::quota_exceeded(format!("vetoed {usecase}")))
    }
}

#[strata::edition(extends = StubCe, modules(Veto))]
pub struct VetoCe;

fn alice_ctx() -> ScyllaCtx {
    ScyllaCtx {
        caller: alice(),
        ..ScyllaCtx::default()
    }
}

fn access(permissions: Arc<dyn PermissionService>) -> AccessModel {
    AccessModel {
        permissions,
        visibility: Arc::new(ScopesByAgent::everything()),
    }
}

async fn stub_ce(permissions: Arc<dyn PermissionService>, seen: Seen) -> StubCe {
    StubCe::builder(&strata::Config::new())
        .provide(access(permissions))
        .provide(seen)
        .build()
        .await
        .unwrap()
}

async fn veto_ce(permissions: Arc<dyn PermissionService>) -> VetoCe {
    VetoCe::builder(&strata::Config::new())
        .provide(access(permissions))
        .build()
        .await
        .unwrap()
}

fn acme(name: &str) -> CreateProject {
    create("acme", name)
}

#[tokio::test]
async fn a_create_by_a_user_checks_the_permission_then_writes_the_owner_grant() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let app = stub_ce(permissions.clone(), Seen::default()).await;

    let project = ProjectsRpc::create_project(&app.scoped(&alice_ctx()), acme("rocket"))
        .await
        .unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreateProject(OrganizationId::new("acme"))]
    );
    assert!(Projects::store(&app).row(project.id()).is_some());
    assert_eq!(Projects::store(&app).grants.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_denied_caller_writes_nothing() {
    let app = stub_ce(Arc::new(DenyingPermissionService::new()), Seen::default()).await;

    let err = ProjectsRpc::create_project(&app.scoped(&alice_ctx()), acme("rocket"))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(Projects::store(&app).is_empty());
}

#[tokio::test]
async fn a_hook_before_the_writes_vetoes_after_the_permission_check() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let app = veto_ce(permissions.clone()).await;

    let err = ProjectsRpc::create_project(&app.scoped(&alice_ctx()), acme("rocket"))
        .await
        .unwrap_err();

    assert!(matches!(&err, DomainError::QuotaExceeded(m) if m == "vetoed create_project"));
    assert_eq!(permissions.permissions().len(), 1);
    assert!(Projects::store(&app).is_empty());
}

#[tokio::test]
async fn a_hook_before_the_reads_vetoes_after_the_permission_check() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let app = veto_ce(permissions.clone()).await;
    let id = ProjectId::new("seen");

    let err = ProjectsRpc::get_project(&app.scoped(&alice_ctx()), GetProject { id: id.clone() })
        .await
        .unwrap_err();

    assert!(matches!(&err, DomainError::QuotaExceeded(m) if m == "vetoed get_project"));
    assert_eq!(permissions.permissions(), vec![Permission::ReadProject(id)]);
}

#[tokio::test]
async fn an_update_checks_its_permission_and_persists_the_change() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let app = stub_ce(permissions.clone(), Seen::default()).await;
    let ctx = alice_ctx();
    let alice = app.scoped(&ctx);
    let created = ProjectsRpc::create_project(&alice, acme("old"))
        .await
        .unwrap();

    let updated = ProjectsRpc::update_project(
        &alice,
        UpdateProject {
            id: created.id().clone(),
            name: Some(ProjectName::new("new").unwrap()),
            description: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(updated.name().as_str(), "new");
    assert_eq!(
        Projects::store(&app)
            .row(created.id())
            .unwrap()
            .name()
            .as_str(),
        "new"
    );
    assert_eq!(
        permissions.permissions()[1],
        Permission::UpdateProject(created.id().clone())
    );
}

#[tokio::test]
async fn a_delete_stops_the_live_jobs_of_the_project_after_it_removes_the_row() {
    let app = stub_ce(Arc::new(RecordingPermissionService::new()), Seen::default()).await;
    let ctx = alice_ctx();
    let alice = app.scoped(&ctx);
    let created = ProjectsRpc::create_project(&alice, acme("gone"))
        .await
        .unwrap();
    let pipeline = PipelineBuilder::for_project_id(created.id().clone()).build();
    let live = JobBuilder::new(&pipeline).build();
    Projects::jobs(&app).live.lock().unwrap().push(live.clone());

    ProjectsRpc::delete_project(
        &alice,
        DeleteProject {
            id: created.id().clone(),
        },
    )
    .await
    .unwrap();

    assert!(Projects::store(&app).is_empty());
    assert_eq!(
        *Projects::jobs(&app).stopped.lock().unwrap(),
        [live.id().clone()]
    );
}

#[tokio::test]
async fn the_projects_of_a_user_are_the_ones_its_grants_let_it_read() {
    let app = stub_ce(Arc::new(RecordingPermissionService::new()), Seen::default()).await;

    ProjectsRpc::list_user_projects(
        &app.scoped(&alice_ctx()),
        ListUserProjects {
            user_id: UserId::new("alice"),
            pagination: None,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        *Projects::store(&app).listed_for_user.lock().unwrap(),
        vec!["readProject".to_string()]
    );
}

fn list_acme() -> ListOrganizationProjects {
    ListOrganizationProjects {
        organization_id: OrganizationId::new("acme"),
        pagination: None,
    }
}

#[tokio::test]
async fn an_organization_listing_asks_the_access_model_once_and_reads_the_rest_from_the_grants() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let own = Visibility::Scoped {
        orgs: Vec::new(),
        projects: vec![ProjectId::new("p1")],
    };
    let seen = Seen(Arc::new(HashMap::from([
        ("listProjectsByOrganization", Visibility::none()),
        ("readProject", own.clone()),
    ])));
    let app = stub_ce(permissions.clone(), seen).await;

    ProjectsRpc::list_organization_projects(&app.scoped(&alice_ctx()), list_acme())
        .await
        .unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::ReadOrganization(OrganizationId::new("acme"))]
    );
    assert_eq!(
        *Projects::store(&app).listed_by_organization.lock().unwrap(),
        vec![own]
    );
}

#[tokio::test]
async fn listing_the_projects_of_an_organization_shows_them_all() {
    let wide = Visibility::Scoped {
        orgs: vec![OrganizationId::new("acme")],
        projects: Vec::new(),
    };
    let seen = Seen(Arc::new(HashMap::from([
        ("listProjectsByOrganization", wide.clone()),
        ("readProject", Visibility::none()),
    ])));
    let app = stub_ce(Arc::new(RecordingPermissionService::new()), seen).await;

    ProjectsRpc::list_organization_projects(&app.scoped(&alice_ctx()), list_acme())
        .await
        .unwrap();

    assert_eq!(
        *Projects::store(&app).listed_by_organization.lock().unwrap(),
        vec![wide]
    );
}
