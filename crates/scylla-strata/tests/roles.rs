//! Tests the Roles use cases of Scylla, written with strata: the CE application tests of the
//! role slice on the stub ports, and the gRPC adapter over a real tonic server.

use async_trait::async_trait;
use scylla_auth::authz::{
    Grant, GrantRepository, PermissionService, Principal, Role, RoleKind, RoleRepository, Scope,
    ScopeKind,
};
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::stubs::{ScopesByAgent, StubGrants, StubRoles, alice};
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{OrganizationId, ProjectId, UserId};
use scylla_domain::domain::permission::{PERMISSION_CATALOG, Permission};
use scylla_domain::domain::role::{RoleDescription, RoleDisplayName, RoleName};
use scylla_proto::authz::v1::ListRolesRequest;
use scylla_strata::grpc::{RoleServiceClient, RoleServiceServer};
use scylla_strata::{AccessModel, CedarPermissions, RolesRpc, ScyllaCtx};
use std::sync::Arc;
use strata::{Contextual, Edition};
use strata_tonic::Grpc;
use tonic::Code;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

#[strata::edition(modules(scylla_strata::Access, scylla_strata::Roles))]
pub struct RolesBase;

#[strata::edition(extends = RolesBase, overrides(scylla_strata::Access))]
pub struct TestRoles;

#[strata::overrides]
impl scylla_strata::Access for TestRoles {
    type Permissions = CedarPermissions;
}

struct Lab {
    app: TestRoles,
    roles: Arc<StubRoles>,
}

fn on(caller: &CallerContext) -> ScyllaCtx {
    ScyllaCtx {
        caller: caller.clone(),
        ..ScyllaCtx::default()
    }
}

async fn build(
    permissions: Arc<dyn PermissionService>,
    roles: Arc<StubRoles>,
    grants: Vec<Grant>,
) -> TestRoles {
    let config = strata::Config::new();
    let role_repo: Arc<dyn RoleRepository> = roles;
    let grant_repo: Arc<dyn GrantRepository> = Arc::new(StubGrants::new(grants));
    TestRoles::builder(&config)
        .provide(AccessModel {
            permissions,
            visibility: Arc::new(ScopesByAgent::everything()),
        })
        .provide_arc(role_repo)
        .provide_arc(grant_repo)
        .build()
        .await
        .unwrap()
}

async fn lab(permissions: Arc<dyn PermissionService>, roles: Vec<Role>, grants: Vec<Grant>) -> Lab {
    lab_using(permissions, roles, grants, &[]).await
}

async fn lab_using(
    permissions: Arc<dyn PermissionService>,
    mut roles: Vec<Role>,
    mut grants: Vec<Grant>,
    used: &[&str],
) -> Lab {
    roles.push(role("system-admin", ScopeKind::System, true, &["*"]));
    grants.push(Grant::new(
        Principal::User(UserId::new("root")),
        RoleName::new("system-admin").unwrap(),
        Scope::System,
    ));
    let roles = Arc::new(
        used.iter()
            .fold(StubRoles::new(roles), |stub, id| stub.used(id)),
    );
    Lab {
        app: build(permissions, roles.clone(), grants).await,
        roles,
    }
}

fn root() -> CallerContext {
    CallerContext::User(UserId::new("root"))
}

fn id(id: &str) -> RoleName {
    RoleName::new(id).unwrap()
}

fn custom(lab: &Lab) -> Vec<Role> {
    lab.roles
        .rows()
        .into_iter()
        .filter(|r| !r.builtin)
        .collect()
}

fn role(id: &str, scope: ScopeKind, builtin: bool, permissions: &[&str]) -> Role {
    Role {
        id: id.to_string(),
        key: builtin.then(|| id.to_string()),
        name: RoleDisplayName::new(id).unwrap(),
        description: RoleDescription::new("").unwrap(),
        scope,
        kind: RoleKind::Member,
        owner_org: None,
        builtin,
        permissions: permissions.iter().map(ToString::to_string).collect(),
        version: 0,
    }
}

fn owned(id: &str, organization: &str, permissions: &[&str]) -> Role {
    Role {
        owner_org: Some(OrganizationId::new(organization)),
        ..role(id, ScopeKind::Project, false, permissions)
    }
}

struct NewRole {
    organization_id: Option<OrganizationId>,
    name: RoleDisplayName,
    description: RoleDescription,
    scope: ScopeKind,
    kind: RoleKind,
    permissions: Vec<String>,
}

async fn create_role(app: &impl RolesRpc<TestRoles>, r: NewRole) -> DomainResult<Role> {
    RolesRpc::create_role(
        app,
        r.organization_id,
        r.name,
        r.description,
        r.scope,
        r.kind,
        r.permissions,
    )
    .await
}

struct RoleChange {
    id: RoleName,
    name: RoleDisplayName,
    description: RoleDescription,
    permissions: Vec<String>,
}

async fn update_role(app: &impl RolesRpc<TestRoles>, r: RoleChange) -> DomainResult<Role> {
    RolesRpc::update_role(app, r.id, r.name, r.description, r.permissions).await
}

fn create(permissions: &[&str]) -> NewRole {
    NewRole {
        organization_id: None,
        name: RoleDisplayName::new("CI Runner").unwrap(),
        description: RoleDescription::new("").unwrap(),
        scope: ScopeKind::Project,
        kind: RoleKind::Member,
        permissions: permissions.iter().map(ToString::to_string).collect(),
    }
}

fn create_in(organization: &str, permissions: &[&str]) -> NewRole {
    NewRole {
        organization_id: Some(OrganizationId::new(organization)),
        ..create(permissions)
    }
}

fn update(role: &str, name: &str, description: &str, permissions: &[&str]) -> RoleChange {
    RoleChange {
        id: id(role),
        name: RoleDisplayName::new(name).unwrap(),
        description: RoleDescription::new(description).unwrap(),
        permissions: permissions.iter().map(ToString::to_string).collect(),
    }
}

#[tokio::test]
async fn a_create_checks_manage_roles_then_stores() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone(), vec![], vec![]).await;

    let created = create_role(&lab.app.scoped(&on(&root())), create(&["readPipeline"]))
        .await
        .unwrap();

    assert_eq!(permissions.permissions(), vec![Permission::ManageRoles]);
    assert!(!created.builtin);
    assert!(created.owner_org.is_none());
    assert_eq!(custom(&lab), [created]);
}

#[tokio::test]
async fn a_denied_create_never_stores() {
    let lab = lab(Arc::new(DenyingPermissionService::new()), vec![], vec![]).await;

    let err = create_role(&lab.app.scoped(&on(&alice())), create(&["readPipeline"]))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(custom(&lab).is_empty());
}

#[tokio::test]
async fn a_permission_out_of_the_role_scope_is_refused_before_anything_is_stored() {
    let lab = lab(Arc::new(RecordingPermissionService::new()), vec![], vec![]).await;

    let err = create_role(
        &lab.app.scoped(&on(&alice())),
        create(&["createOrganization"]),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, DomainError::Validation(_)));
    assert!(custom(&lab).is_empty());
}

#[tokio::test]
async fn an_update_validates_against_the_stored_scope_and_rewrites_the_role() {
    let lab = lab(
        Arc::new(RecordingPermissionService::new()),
        vec![role("ci", ScopeKind::Project, false, &["readPipeline"])],
        vec![],
    )
    .await;
    let ctx = on(&root());
    let root = lab.app.scoped(&ctx);

    let err = update_role(
        &root,
        update("ci", "CI", "runs the builds", &["createProject"]),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)));

    let updated = update_role(
        &root,
        update("ci", "CI", "runs the builds", &["runPipeline"]),
    )
    .await
    .unwrap();
    assert_eq!(updated.name.as_str(), "CI");
    assert_eq!(updated.permissions, vec!["runPipeline".to_string()]);
    assert_eq!(custom(&lab), [updated]);
}

#[tokio::test]
async fn a_missing_role_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()), vec![], vec![]).await;

    let err = RolesRpc::get_role(&lab.app.scoped(&on(&alice())), id("ghost"))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::NotFound(_)));
}

#[tokio::test]
async fn a_builtin_role_or_a_role_in_use_cannot_be_deleted() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab_using(
        permissions.clone(),
        vec![
            role("organization-admin", ScopeKind::Organization, true, &["*"]),
            role("ci", ScopeKind::Project, false, &["readPipeline"]),
            role("unused", ScopeKind::Project, false, &["readPipeline"]),
        ],
        vec![],
        &["ci"],
    )
    .await;
    let ctx = on(&root());
    let root = lab.app.scoped(&ctx);

    for role in ["organization-admin", "ci"] {
        let err = RolesRpc::delete_role(&root, id(role)).await.unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{role}");
    }

    RolesRpc::delete_role(&root, id("unused")).await.unwrap();
    assert!(!lab.roles.rows().iter().any(|r| r.id == "unused"));
    assert_eq!(
        permissions.permissions().last(),
        Some(&Permission::ManageRole(id("unused"))),
        "a command on one role is checked on that role"
    );
}

#[tokio::test]
async fn a_role_of_an_organization_is_created_there_with_its_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone(), vec![], vec![]).await;

    let created = create_role(
        &lab.app.scoped(&on(&root())),
        create_in("o1", &["readPipeline"]),
    )
    .await
    .unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageOrgRoles(OrganizationId::new("o1"))]
    );
    assert_eq!(created.owner_org, Some(OrganizationId::new("o1")));
    assert_eq!(custom(&lab), [created]);
}

#[tokio::test]
async fn a_role_of_an_organization_is_never_system_scoped_nor_an_admin_role() {
    let lab = lab(Arc::new(RecordingPermissionService::new()), vec![], vec![]).await;
    let system = NewRole {
        scope: ScopeKind::System,
        ..create_in("o1", &["readOrganization"])
    };
    let admin = NewRole {
        kind: RoleKind::Admin,
        ..create(&["readPipeline"])
    };
    let ctx = on(&root());

    for command in [system, admin] {
        let err = create_role(&lab.app.scoped(&ctx), command)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation(_)));
    }
    assert!(custom(&lab).is_empty());
}

#[tokio::test]
async fn a_role_holds_only_permissions_its_author_holds() {
    let lab = lab(
        Arc::new(RecordingPermissionService::new()),
        vec![role(
            "role-manager",
            ScopeKind::Organization,
            false,
            &["manageOrgRoles", "readProject"],
        )],
        vec![Grant::new(
            Principal::User(UserId::new("dave")),
            id("role-manager"),
            Scope::Organization(OrganizationId::new("o1")),
        )],
    )
    .await;
    let ctx = on(&CallerContext::User(UserId::new("dave")));
    let dave = lab.app.scoped(&ctx);

    let err = create_role(&dave, create_in("o1", &["deleteProject"]))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::BusinessRule(_)));
    let err = create_role(&dave, create_in("o2", &["readProject"]))
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::BusinessRule(_)),
        "a grant on o1 confers nothing on o2"
    );

    let created = create_role(&dave, create_in("o1", &["readProject"]))
        .await
        .unwrap();
    assert_eq!(created.owner_org, Some(OrganizationId::new("o1")));
}

#[tokio::test]
async fn an_update_checks_only_the_permissions_it_adds() {
    let lab = lab(
        Arc::new(RecordingPermissionService::new()),
        vec![
            role(
                "role-manager",
                ScopeKind::Organization,
                false,
                &["manageOrgRoles", "readProject"],
            ),
            owned("deployer", "o1", &["readProject", "runPipeline"]),
            owned("owner", "o1", &["*"]),
        ],
        vec![Grant::new(
            Principal::User(UserId::new("dave")),
            id("role-manager"),
            Scope::Organization(OrganizationId::new("o1")),
        )],
    )
    .await;
    let ctx = on(&CallerContext::User(UserId::new("dave")));
    let dave = lab.app.scoped(&ctx);

    update_role(
        &dave,
        update("deployer", "Renamed", "", &["readProject", "runPipeline"]),
    )
    .await
    .expect("keeping a permission the author lacks is no escalation");
    update_role(
        &dave,
        update("owner", "Renamed", "", &["readProject", "deleteProject"]),
    )
    .await
    .expect("narrowing full control adds nothing");
    let err = update_role(
        &dave,
        update("deployer", "Renamed", "", &["runPipeline", "deleteProject"]),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DomainError::BusinessRule(_)));
}

#[tokio::test]
async fn an_organization_lists_the_platform_roles_and_its_own() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(
        permissions.clone(),
        vec![
            owned("deployer-o1", "o1", &["runPipeline"]),
            owned("deployer-o2", "o2", &["runPipeline"]),
        ],
        vec![],
    )
    .await;

    let roles = RolesRpc::list_roles(
        &lab.app.scoped(&on(&root())),
        Some(OrganizationId::new("o1")),
    )
    .await
    .unwrap();

    let ids: Vec<&str> = roles.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(ids, ["deployer-o1", "system-admin"]);
    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageOrgRoles(OrganizationId::new("o1"))]
    );
}

#[tokio::test]
async fn effective_permissions_group_the_grants_by_scope_behind_manage_system_grants() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let alice_principal = Principal::User(UserId::new("alice"));
    let lab = lab(
        permissions.clone(),
        vec![
            role(
                "ci",
                ScopeKind::Project,
                false,
                &["readPipeline", "runPipeline"],
            ),
            role("janitor", ScopeKind::Organization, false, &["deleteJob"]),
            role("organization-admin", ScopeKind::Organization, true, &["*"]),
        ],
        vec![
            Grant::new(
                alice_principal.clone(),
                id("ci"),
                Scope::Project(ProjectId::new("p1")),
            ),
            Grant::new(
                alice_principal.clone(),
                id("janitor"),
                Scope::Organization(OrganizationId::new("o1")),
            ),
            Grant::new(
                alice_principal.clone(),
                id("organization-admin"),
                Scope::Organization(OrganizationId::new("o2")),
            ),
        ],
    )
    .await;

    let scopes = RolesRpc::get_effective_permissions(
        &lab.app
            .scoped(&on(&CallerContext::Service(ServiceIdentity::recorder()))),
        alice_principal,
    )
    .await
    .unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageSystemGrants]
    );
    assert_eq!(scopes.len(), 3);
    assert_eq!(
        scopes[0].permissions,
        vec!["readPipeline".to_string(), "runPipeline".to_string()]
    );
    assert_eq!(scopes[1].permissions, vec!["deleteJob".to_string()]);
    assert!(scopes[2].full_control);
    assert!(scopes[2].permissions.is_empty());
}

#[tokio::test]
async fn my_permissions_asks_for_no_permission_and_refuses_a_non_principal() {
    let lab = lab(
        Arc::new(DenyingPermissionService::new()),
        vec![role(
            "organization-admin",
            ScopeKind::Organization,
            true,
            &["*"],
        )],
        vec![Grant::new(
            Principal::User(UserId::new("alice")),
            id("organization-admin"),
            Scope::Organization(OrganizationId::new("o1")),
        )],
    )
    .await;
    let ctx = on(&alice());

    let scopes = RolesRpc::get_my_permissions(&lab.app.scoped(&ctx))
        .await
        .unwrap();
    assert_eq!(scopes.len(), 1);
    assert!(scopes[0].full_control);

    let err = RolesRpc::get_effective_permissions(
        &lab.app.scoped(&ctx),
        Principal::User(UserId::new("bob")),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    for caller in [
        CallerContext::Service(ServiceIdentity::recorder()),
        CallerContext::Anonymous,
    ] {
        let err = RolesRpc::get_my_permissions(&lab.app.scoped(&on(&caller)))
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)), "{caller}");
    }
}

#[tokio::test]
async fn the_vocabulary_is_the_permission_catalog_and_asks_no_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone(), vec![], vec![]).await;

    let vocabulary = RolesRpc::list_authz_vocabulary(&lab.app.scoped(&on(&alice())))
        .await
        .unwrap();

    assert_eq!(vocabulary.len(), PERMISSION_CATALOG.len());
    assert!(permissions.permissions().is_empty());
}

struct DenyBob;

#[async_trait]
impl PermissionService for DenyBob {
    async fn check(&self, caller: &CallerContext, permission: Permission) -> DomainResult<()> {
        match caller {
            CallerContext::User(id) if id.as_str() == "bob" => Err(DomainError::forbidden(
                format!("{} is not granted", permission.key()),
            )),
            _ => Ok(()),
        }
    }
}

async fn serve(app: TestRoles) -> RoleServiceClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Server::builder().add_service(RoleServiceServer::new(Grpc::new(app)));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    RoleServiceClient::new(channel)
}

fn list_roles(bearer: Option<&str>) -> tonic::Request<ListRolesRequest> {
    let mut request = tonic::Request::new(ListRolesRequest {
        organization_id: None,
    });
    if let Some(bearer) = bearer {
        request
            .metadata_mut()
            .insert("authorization", bearer.parse().unwrap());
    }
    request
}

#[tokio::test]
async fn grpc_serves_the_role_contract() {
    let roles = Arc::new(StubRoles::new(vec![role(
        "ci",
        ScopeKind::Project,
        false,
        &["readPipeline"],
    )]));
    let mut client = serve(build(Arc::new(DenyBob), roles, vec![]).await).await;

    let listed = client
        .list_roles(list_roles(Some("Bearer user:ann")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(listed.roles.len(), 1);
    let denied = client
        .list_roles(list_roles(Some("Bearer user:bob")))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied:?}");
    let anonymous = client.list_roles(list_roles(None)).await.unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
}
