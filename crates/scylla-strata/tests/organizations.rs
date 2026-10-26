//! Tests the Organizations module of Scylla, written with strata: the CE application tests of the
//! slice on stub ports, and the gRPC adapter over a real tonic server.

use async_trait::async_trait;
use scylla_auth::authz::{Grant, PermissionService};
use scylla_core::application::organization::OrganizationRepository;
use scylla_core::application::pagination::{PaginatedResult, PaginationParams};
use scylla_core::application::{AppRepository, PrincipalAccess, UserRepository};
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::jobs::JobBuilder;
use scylla_core::test_support::pipelines::PipelineBuilder;
use scylla_core::test_support::stubs::{
    NoUsers, ScopesByAgent, StubJobs, StubRegistry, StubTails, alice, dispatcher, empty_page,
};
use scylla_domain::domain::agent::Agent;
use scylla_domain::domain::app::{App, AppCredential, AppName};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppId, OrganizationId, ProjectId, UserId};
use scylla_domain::domain::organization::{
    Organization, OrganizationDescription, OrganizationName,
};
use scylla_domain::domain::permission::Permission;
use scylla_proto::organization::v1::CreateOrganizationRequest;
use scylla_strata::grpc::{OrganizationServiceClient, OrganizationServiceServer};
use scylla_strata::{AccessModel, JobDispatch, OrganizationsRpc, ScyllaCtx};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use strata::{Contextual, Edition};
use strata_tonic::Grpc;
use tonic::Code;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

#[strata::edition(modules(scylla_strata::Access, scylla_strata::Organizations))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = scylla_strata::CedarPermissions;
}

#[derive(Default)]
struct StubOrganizations {
    rows: Mutex<HashMap<OrganizationId, Organization>>,
    grants: Mutex<Vec<Grant>>,
}

#[async_trait]
impl OrganizationRepository for StubOrganizations {
    async fn create(&self, organization: &Organization) -> DomainResult<Organization> {
        self.rows
            .lock()
            .unwrap()
            .insert(organization.id().clone(), organization.clone());
        Ok(organization.clone())
    }
    async fn provision_with_owner(
        &self,
        organization: &Organization,
        grant: &Grant,
    ) -> DomainResult<()> {
        self.create(organization).await?;
        self.grants.lock().unwrap().push(grant.clone());
        Ok(())
    }
    async fn list_principals(
        &self,
        _: &OrganizationId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<UserId>> {
        empty_page()
    }
    async fn count_members(&self, _: &OrganizationId) -> DomainResult<u64> {
        Ok(0)
    }
    async fn has_member(&self, _: &OrganizationId, _: &UserId) -> DomainResult<bool> {
        Ok(false)
    }
    async fn list_for_user(
        &self,
        _: &UserId,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>> {
        empty_page()
    }
    async fn find_by_id(&self, id: &OrganizationId) -> DomainResult<Organization> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("Organization", id))
    }
    async fn update(&self, organization: &Organization) -> DomainResult<Organization> {
        self.create(organization).await
    }
    async fn delete(&self, organization: &Organization) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(organization.id());
        Ok(())
    }
    async fn list_all(
        &self,
        _: Option<&PaginationParams>,
    ) -> DomainResult<PaginatedResult<Organization>> {
        empty_page()
    }
}

#[derive(Default)]
struct StubApps(Mutex<Vec<App>>);

#[async_trait]
impl AppRepository for StubApps {
    async fn create_app(&self, _: &App, _: &AppCredential) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn provision_agent(
        &self,
        _: &App,
        _: &AppCredential,
        _: &Agent,
        _: &Grant,
    ) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn provision(&self, _: &App, _: &Grant) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn find_by_id(&self, _: &AppId) -> DomainResult<App> {
        unreachable!("an organization action lists its apps")
    }
    async fn find_trigger_runner(&self, _: &OrganizationId) -> DomainResult<Option<AppId>> {
        unreachable!("an organization action lists its apps")
    }
    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Vec<App>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|a| a.organization_id() == organization_id)
            .cloned()
            .collect())
    }
    async fn set_active(&self, _: &AppId, _: bool) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn delete(&self, _: &AppId) -> DomainResult<()> {
        unreachable!("an organization action writes no app")
    }
    async fn list_access(&self, _: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        unreachable!("an organization action reads no app access")
    }
}

struct Lab {
    app: TestCe,
    organizations: Arc<StubOrganizations>,
    apps: Arc<StubApps>,
    jobs: Arc<StubJobs>,
    registry: Arc<StubRegistry>,
}

impl Lab {
    async fn create(&self, name: &str) -> DomainResult<Organization> {
        OrganizationsRpc::create_organization(&self.app.scoped(&as_(alice())), org_name(name), None)
            .await
    }
}

fn as_(caller: CallerContext) -> ScyllaCtx {
    ScyllaCtx {
        caller,
        ..ScyllaCtx::default()
    }
}

async fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    let organizations = Arc::new(StubOrganizations::default());
    let apps = Arc::new(StubApps::default());
    let jobs = Arc::new(StubJobs::default());
    let registry = Arc::new(StubRegistry::default());
    let config = strata::Config::new();
    let app = TestCe::builder(&config)
        .provide(AccessModel {
            permissions,
            visibility: Arc::new(ScopesByAgent::everything()),
        })
        .provide(JobDispatch(dispatcher(
            registry.clone(),
            jobs.clone(),
            Arc::new(StubTails::default()),
        )))
        .provide_arc::<dyn OrganizationRepository>(organizations.clone())
        .provide_arc::<dyn UserRepository>(Arc::new(NoUsers))
        .provide_arc::<dyn AppRepository>(apps.clone())
        .build()
        .await
        .unwrap();
    Lab {
        app,
        organizations,
        apps,
        jobs,
        registry,
    }
}

fn org_name(name: &str) -> OrganizationName {
    OrganizationName::new(name).unwrap()
}

#[tokio::test]
async fn a_create_by_a_user_checks_the_permission_then_writes_the_owner_grant() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;

    let organization = lab.create("acme").await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreateOrganization]
    );
    assert!(
        lab.organizations
            .rows
            .lock()
            .unwrap()
            .contains_key(organization.id())
    );
    assert_eq!(lab.organizations.grants.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_denied_caller_writes_nothing() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;

    let err = lab.create("acme").await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.organizations.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_name_is_a_label_so_two_organizations_can_share_it() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let first = lab.create("acme").await.unwrap();

    let second = lab.create("acme").await.unwrap();

    assert_ne!(first.id(), second.id());
    assert_eq!(lab.organizations.rows.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn an_update_stages_the_change_and_persists_it() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("old").await.unwrap();

    let updated = OrganizationsRpc::update_organization(
        &lab.app.scoped(&as_(alice())),
        created.id().clone(),
        Some(OrganizationName::new("new").unwrap()),
        None,
    )
    .await
    .unwrap();

    assert_eq!(updated.name().as_str(), "new");
    assert_eq!(
        lab.organizations.rows.lock().unwrap()[created.id()]
            .name()
            .as_str(),
        "new"
    );
    assert_eq!(
        permissions.permissions()[1],
        Permission::UpdateOrganization(created.id().clone())
    );
}

#[tokio::test]
async fn an_update_keeps_clears_or_sets_the_description() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let ctx = as_(alice());
    let alice = lab.app.scoped(&ctx);
    let described = |d: &str| Some(OrganizationDescription::new(d).unwrap());
    let created =
        OrganizationsRpc::create_organization(&alice, org_name("acme"), described("widgets"))
            .await
            .unwrap();
    let id = created.id().clone();
    let description = |o: &Organization| o.description().map(|d| d.as_str().to_owned());

    let kept = OrganizationsRpc::update_organization(&alice, id.clone(), None, None)
        .await
        .unwrap();
    let cleared = OrganizationsRpc::update_organization(&alice, id.clone(), None, Some(None))
        .await
        .unwrap();
    let set = OrganizationsRpc::update_organization(&alice, id, None, Some(described("gadgets")))
        .await
        .unwrap();

    assert_eq!(description(&kept).as_deref(), Some("widgets"));
    assert_eq!(description(&cleared), None);
    assert_eq!(description(&set).as_deref(), Some("gadgets"));
}

#[tokio::test]
async fn a_delete_removes_the_organization() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("gone").await.unwrap();

    OrganizationsRpc::delete_organization(&lab.app.scoped(&as_(alice())), created.id().clone())
        .await
        .unwrap();

    assert!(lab.organizations.rows.lock().unwrap().is_empty());
    assert_eq!(
        permissions.permissions()[1],
        Permission::DeleteOrganization(created.id().clone())
    );
}

#[tokio::test]
async fn a_delete_stops_the_live_jobs_of_the_organization_on_their_agents() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create("gone").await.unwrap();
    let agent = AppId::new("agent-1");
    lab.registry.connect(&agent);
    let pipeline = PipelineBuilder::for_project_id(ProjectId::new("p")).build();
    let running = lab.jobs.insert(
        &JobBuilder::new(&pipeline)
            .running(true)
            .agent(agent.clone())
            .build(),
    );

    OrganizationsRpc::delete_organization(&lab.app.scoped(&as_(alice())), created.id().clone())
        .await
        .unwrap();

    assert_eq!(lab.registry.cancelled(), [(agent, running.id().clone())]);
}

#[tokio::test]
async fn a_delete_closes_the_agent_streams_of_the_organization_apps() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create("gone").await.unwrap();
    let other = lab.create("kept").await.unwrap();
    let app = |organization: &Organization, name: &str| {
        let app = App::create(organization.id().clone(), AppName::new(name).unwrap()).unwrap();
        lab.apps.0.lock().unwrap().push(app.clone());
        lab.registry.connect(app.id());
        app
    };
    let ours = app(&created, "runner");
    app(&other, "runner");

    OrganizationsRpc::delete_organization(&lab.app.scoped(&as_(alice())), created.id().clone())
        .await
        .unwrap();

    assert_eq!(lab.registry.disconnected(), [ours.id().clone()]);
}

#[tokio::test]
async fn a_read_checks_its_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create("seen").await.unwrap();

    let read =
        OrganizationsRpc::get_organization(&lab.app.scoped(&as_(alice())), created.id().clone())
            .await
            .unwrap();

    assert_eq!(read.id(), created.id());
    assert_eq!(
        permissions.permissions()[1],
        Permission::ReadOrganization(created.id().clone())
    );
}

/// Grants each permission to each caller except `user:eve`.
struct AllButEve;

#[async_trait]
impl PermissionService for AllButEve {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        match caller {
            CallerContext::User(id) if id.as_str() == "eve" => Err(DomainError::forbidden(
                format!("{} is not granted", permission.key()),
            )),
            _ => Ok(()),
        }
    }
}

async fn serve(lab: Lab) -> OrganizationServiceClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Server::builder().add_service(OrganizationServiceServer::new(Grpc::new(lab.app)));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    OrganizationServiceClient::new(channel)
}

fn request(name: &str, bearer: Option<&str>) -> tonic::Request<CreateOrganizationRequest> {
    let mut request = tonic::Request::new(CreateOrganizationRequest {
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
    let mut client = serve(lab(Arc::new(AllButEve)).await).await;

    let created = client
        .create_organization(request("acme", Some("Bearer user:ann")))
        .await
        .unwrap()
        .into_inner();
    let organization = created.organization.unwrap();
    assert_eq!(
        (organization.name.as_str(), organization.is_active),
        ("acme", true)
    );

    let anonymous = client
        .create_organization(request("acme", None))
        .await
        .unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    let denied = client
        .create_organization(request("acme", Some("Bearer user:eve")))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied:?}");
    let empty_name = client
        .create_organization(request(" ", Some("Bearer user:ann")))
        .await
        .unwrap_err();
    assert_eq!(empty_name.code(), Code::InvalidArgument, "{empty_name:?}");
}
