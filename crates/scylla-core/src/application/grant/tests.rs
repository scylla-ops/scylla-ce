//! The grant's actions through the engine, on stub ports.

use super::*;
use crate::domain::caller::CallerContext;
use crate::domain::caller::ServiceIdentity;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, GrantId, OrganizationId, ProjectId, UserId};
use crate::domain::permission::Permission;
use crate::domain::permission::ResourceRef;
use crate::domain::role::RoleName;
use crate::domain::role::{RoleDescription, RoleDisplayName};
use crate::test_support::authz::{DenyingPermissionService, RecordingPermissionService, actions};
use crate::test_support::stubs::{StubGrants, StubRegistry, StubRoles};
use async_trait::async_trait;
use scylla_auth::authz::*;
use scylla_auth::authz::{ResourceAncestors, Role};
use scylla_extension::{Actions, Deleted};
use std::collections::HashMap;

/// Every project is in `o1`; an app is in the organization its entry names, and unknown without one.
#[derive(Default)]
struct StubAncestry {
    apps: HashMap<AppId, OrganizationId>,
}

#[async_trait]
impl AuthzEntityProvider for StubAncestry {
    async fn resource_ancestors(&self, resource: &ResourceRef) -> DomainResult<ResourceAncestors> {
        let organization = match resource {
            ResourceRef::App(id) => self.apps.get(id).cloned(),
            _ => Some(OrganizationId::new("o1")),
        };
        Ok(ResourceAncestors {
            organization,
            ..Default::default()
        })
    }
    async fn app_is_active(&self, _app: &AppId) -> DomainResult<bool> {
        Ok(true)
    }
    async fn policy_version(&self) -> DomainResult<i64> {
        Ok(0)
    }
}

fn test_role(id: &str, scope: ScopeKind, permissions: &[&str]) -> Role {
    Role {
        id: id.to_string(),
        key: Some(id.to_string()),
        name: RoleDisplayName::new(id).unwrap(),
        description: RoleDescription::new("").unwrap(),
        scope,
        kind: if matches!(
            id,
            ORGANIZATION_AGENT_ROLE | PROJECT_AGENT_ROLE | ORGANIZATION_TRIGGER_RUNNER_ROLE
        ) {
            RoleKind::Agent
        } else {
            RoleKind::Member
        },
        owner_org: None,
        builtin: true,
        permissions: permissions.iter().map(ToString::to_string).collect(),
        version: 0,
    }
}

fn owned_role(id: &str, organization: &str, permissions: &[&str]) -> Role {
    Role {
        owner_org: Some(OrganizationId::new(organization)),
        builtin: false,
        key: None,
        ..test_role(id, ScopeKind::Project, permissions)
    }
}

struct Lab {
    actions: Actions,
    uc: GrantUseCases,
    grants: Arc<StubGrants>,
    registry: Arc<StubRegistry>,
}

impl Lab {
    async fn grant(&self, caller: &CallerContext, grant: &Grant) -> DomainResult<Grant> {
        let cmd = CreateGrant {
            principal: grant.principal.clone(),
            role: grant.role.clone(),
            scope: grant.scope.clone(),
        };
        self.actions.run(&self.uc, caller, cmd).await
    }

    async fn revoke_all_access(
        &self,
        caller: &CallerContext,
        principal: &Principal,
        scope: &Scope,
    ) -> DomainResult<u64> {
        let cmd = RevokeAllAccess {
            principal: principal.clone(),
            scope: scope.clone(),
        };
        self.actions.run(&self.uc, caller, cmd).await
    }

    async fn revoke(&self, id: &str) -> DomainResult<Option<Deleted<Grant>>> {
        let cmd = RevokeGrant {
            id: GrantId::new(id),
        };
        self.actions.run(&self.uc, &admin(), cmd).await
    }

    async fn list(&self, scope: Option<Scope>) -> DomainResult<Vec<Grant>> {
        self.actions
            .run(&self.uc, &admin(), ListGrants { scope })
            .await
    }
}

fn lab(grants: Vec<Grant>) -> Lab {
    lab_with(grants, vec![], Arc::new(RecordingPermissionService::new()))
}

fn lab_with(grants: Vec<Grant>, roles: Vec<Role>, permissions: Arc<dyn PermissionService>) -> Lab {
    lab_in(grants, roles, permissions, StubAncestry::default())
}

fn lab_in(
    grants: Vec<Grant>,
    roles: Vec<Role>,
    permissions: Arc<dyn PermissionService>,
    ancestry: StubAncestry,
) -> Lab {
    let grants = Arc::new(StubGrants::new(grants));
    let registry = Arc::new(StubRegistry::default());
    Lab {
        actions: actions(permissions),
        uc: GrantUseCases::new(
            grants.clone(),
            Arc::new(StubRoles::new(roles)),
            registry.clone(),
            Arc::new(ancestry),
        ),
        grants,
        registry,
    }
}

fn admin() -> CallerContext {
    CallerContext::User(UserId::new("admin"))
}

fn service() -> CallerContext {
    CallerContext::Service(ServiceIdentity::recorder())
}

fn org() -> Scope {
    Scope::Organization(OrganizationId::new("o1"))
}

fn owner_grant(principal: Principal, scope: Scope) -> Grant {
    Grant::new(
        principal,
        RoleName::new(ORGANIZATION_ADMIN_ROLE).unwrap(),
        scope,
    )
}

fn list_grantable(scope_kind: Option<ScopeKind>, organization: Option<&str>) -> ListGrantableRoles {
    ListGrantableRoles {
        scope_kind,
        organization_id: organization.map(OrganizationId::new),
    }
}

#[tokio::test]
async fn the_grantable_roles_are_the_platform_roles_and_those_of_the_organization() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab_with(
        vec![],
        vec![
            test_role(SYSTEM_ADMIN_ROLE, ScopeKind::System, &["*"]),
            test_role(PROJECT_VIEWER_ROLE, ScopeKind::Project, &["readProject"]),
            owned_role("deployer-o1", "o1", &["runPipeline"]),
            owned_role("deployer-o2", "o2", &["runPipeline"]),
        ],
        permissions.clone(),
    );
    let ids = |roles: Vec<Role>| roles.into_iter().map(|r| r.id).collect::<Vec<_>>();

    let platform = lab
        .actions
        .run(&lab.uc, &admin(), list_grantable(None, None))
        .await
        .unwrap();
    assert_eq!(ids(platform), [SYSTEM_ADMIN_ROLE, PROJECT_VIEWER_ROLE]);
    assert!(permissions.permissions().is_empty());

    let in_o1 = lab
        .actions
        .run(
            &lab.uc,
            &admin(),
            list_grantable(Some(ScopeKind::Project), Some("o1")),
        )
        .await
        .unwrap();
    assert_eq!(ids(in_o1), [PROJECT_VIEWER_ROLE, "deployer-o1"]);
    assert_eq!(
        permissions.permissions(),
        vec![Permission::ReadOrganization(OrganizationId::new("o1"))]
    );
}

#[tokio::test]
async fn the_grantable_roles_refuse_anonymous() {
    let lab = lab_with(vec![], vec![], Arc::new(DenyingPermissionService::new()));

    let err = lab
        .actions
        .run(
            &lab.uc,
            &CallerContext::Anonymous,
            list_grantable(None, None),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));
}

#[tokio::test]
async fn a_role_of_another_organization_is_an_unknown_role() {
    let lab = lab_with(
        vec![Grant::new(
            Principal::User(UserId::new("carol")),
            RoleName::new(ORGANIZATION_MEMBER_ROLE).unwrap(),
            org(),
        )],
        vec![
            test_role(
                ORGANIZATION_MEMBER_ROLE,
                ScopeKind::Organization,
                &["readOrganization"],
            ),
            owned_role("deployer-o1", "o1", &["runPipeline"]),
            owned_role("deployer-o2", "o2", &["runPipeline"]),
        ],
        Arc::new(RecordingPermissionService::new()),
    );
    let give = |role: &str| {
        Grant::new(
            Principal::User(UserId::new("carol")),
            RoleName::new(role).unwrap(),
            Scope::Project(ProjectId::new("p1")),
        )
    };

    let foreign = lab
        .grant(&service(), &give("deployer-o2"))
        .await
        .unwrap_err();
    let unknown = lab.grant(&service(), &give("ghost")).await.unwrap_err();
    assert_eq!(
        foreign.to_string(),
        unknown.to_string().replace("ghost", "deployer-o2")
    );
    assert!(matches!(foreign, DomainError::Validation(_)));

    lab.grant(&service(), &give("deployer-o1")).await.unwrap();
}

#[test]
fn grant_carries_the_role_it_confers() {
    let grant = Grant::new(
        Principal::User(UserId::new("u1")),
        RoleName::new(ORGANIZATION_ADMIN_ROLE).unwrap(),
        org(),
    );
    assert_eq!(grant.role.as_str(), ORGANIZATION_ADMIN_ROLE);
}

#[tokio::test]
async fn a_create_checks_the_permission_of_its_scope_then_stores_the_grant() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab_with(
        vec![],
        vec![test_role(
            ORGANIZATION_ADMIN_ROLE,
            ScopeKind::Organization,
            &[FULL_CONTROL],
        )],
        permissions.clone(),
    );
    let grant = owner_grant(Principal::User(UserId::new("alice")), org());

    let stored = lab.grant(&service(), &grant).await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageOrgGrants(OrganizationId::new("o1"))]
    );
    assert_eq!(stored.principal, grant.principal);
    assert_eq!(stored.scope, grant.scope);
    assert_eq!(lab.grants.created().as_slice(), [stored]);
    assert!(
        lab.registry.wakes().is_empty(),
        "a user grant wakes no agent"
    );
}

#[tokio::test]
async fn a_denied_create_never_stores() {
    let lab = lab_with(
        vec![],
        vec![test_role(
            ORGANIZATION_ADMIN_ROLE,
            ScopeKind::Organization,
            &[FULL_CONTROL],
        )],
        Arc::new(DenyingPermissionService::new()),
    );
    let grant = owner_grant(Principal::User(UserId::new("alice")), org());

    let err = lab.grant(&service(), &grant).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.grants.created().is_empty());
}

#[tokio::test]
async fn anti_escalation_blocks_granting_more_than_you_hold() {
    let roles = vec![
        test_role(
            ORGANIZATION_ADMIN_ROLE,
            ScopeKind::Organization,
            &[FULL_CONTROL],
        ),
        test_role(
            "grant-manager",
            ScopeKind::Organization,
            &["manageOrgGrants"],
        ),
    ];
    let lab = lab_with(
        vec![
            Grant::new(
                Principal::User(UserId::new("bob")),
                RoleName::new("grant-manager").unwrap(),
                org(),
            ),
            owner_grant(Principal::User(UserId::new("carol")), org()),
        ],
        roles,
        Arc::new(RecordingPermissionService::new()),
    );
    let bob = CallerContext::User(UserId::new("bob"));
    let carol = CallerContext::User(UserId::new("carol"));

    assert!(
        lab.grant(
            &bob,
            &owner_grant(Principal::User(UserId::new("alice")), org())
        )
        .await
        .is_err(),
        "escalation to full control must be blocked",
    );

    assert!(
        lab.grant(
            &bob,
            &Grant::new(
                Principal::User(UserId::new("alice")),
                RoleName::new("grant-manager").unwrap(),
                org(),
            ),
        )
        .await
        .is_ok(),
        "delegating a role whose permissions you hold is allowed",
    );

    assert!(
        lab.grant(
            &carol,
            &owner_grant(Principal::User(UserId::new("dave")), org())
        )
        .await
        .is_ok(),
        "a full-control admin may grant",
    );
}

#[tokio::test]
async fn custom_role_grantable_via_db_with_scope_check() {
    let mut custom = test_role(
        "01customrole",
        ScopeKind::Organization,
        &["readOrganization"],
    );
    custom.builtin = false;
    custom.key = None;
    let admin_role = test_role(
        ORGANIZATION_ADMIN_ROLE,
        ScopeKind::Organization,
        &[FULL_CONTROL],
    );
    let lab = lab_with(
        vec![owner_grant(Principal::User(UserId::new("owner")), org())],
        vec![custom, admin_role],
        Arc::new(RecordingPermissionService::new()),
    );
    let owner = CallerContext::User(UserId::new("owner"));

    assert!(
        lab.grant(
            &owner,
            &Grant::new(
                Principal::User(UserId::new("alice")),
                RoleName::new("01customrole").unwrap(),
                org(),
            ),
        )
        .await
        .is_ok(),
        "a custom role valid at its scope must be grantable",
    );

    assert!(
        lab.grant(
            &owner,
            &Grant::new(
                Principal::User(UserId::new("alice")),
                RoleName::new("ghost").unwrap(),
                org(),
            ),
        )
        .await
        .is_err(),
        "unknown role must be rejected",
    );

    assert!(
        lab.grant(
            &owner,
            &Grant::new(
                Principal::User(UserId::new("alice")),
                RoleName::new("01customrole").unwrap(),
                Scope::Project(ProjectId::new("p1")),
            ),
        )
        .await
        .is_err(),
        "a custom role on the wrong scope kind must be rejected",
    );
}

#[tokio::test]
async fn a_project_grant_requires_admission_to_the_organization() {
    let roles = vec![
        test_role(PROJECT_ADMIN_ROLE, ScopeKind::Project, &[FULL_CONTROL]),
        test_role(
            ORGANIZATION_MEMBER_ROLE,
            ScopeKind::Organization,
            &["readOrganization"],
        ),
    ];
    let alice = Principal::User(UserId::new("alice"));
    let grant = Grant::new(
        alice.clone(),
        RoleName::new(PROJECT_ADMIN_ROLE).unwrap(),
        Scope::Project(ProjectId::new("p1")),
    );

    let lab = lab_with(
        vec![],
        roles.clone(),
        Arc::new(RecordingPermissionService::new()),
    );
    assert!(
        lab.grant(&service(), &grant).await.is_err(),
        "a project grant to someone the organization has not admitted must be refused"
    );

    let admitted = Grant::new(
        alice,
        RoleName::new(ORGANIZATION_MEMBER_ROLE).unwrap(),
        org(),
    );
    let lab = lab_with(
        vec![admitted],
        roles,
        Arc::new(RecordingPermissionService::new()),
    );
    assert!(
        lab.grant(&service(), &grant).await.is_ok(),
        "a project grant to an admitted user is accepted"
    );
}

#[tokio::test]
async fn an_organization_grant_needs_no_prior_admission() {
    let roles = vec![test_role(
        ORGANIZATION_ADMIN_ROLE,
        ScopeKind::Organization,
        &[FULL_CONTROL],
    )];
    let lab = lab_with(vec![], roles, Arc::new(RecordingPermissionService::new()));
    let grant = owner_grant(Principal::User(UserId::new("alice")), org());
    assert!(lab.grant(&service(), &grant).await.is_ok());
}

fn agent_roles() -> Vec<Role> {
    vec![
        test_role(
            PROJECT_AGENT_ROLE,
            ScopeKind::Project,
            &["readPipeline", "executeJob"],
        ),
        test_role(
            ORGANIZATION_AGENT_ROLE,
            ScopeKind::Organization,
            &["readPipeline", "executeJob"],
        ),
    ]
}

fn app_in(organization: &str) -> StubAncestry {
    StubAncestry {
        apps: HashMap::from([(AppId::new("agent-1"), OrganizationId::new(organization))]),
    }
}

fn agent_grant(role: &str, scope: Scope) -> Grant {
    Grant::new(
        Principal::App(AppId::new("agent-1")),
        RoleName::new(role).unwrap(),
        scope,
    )
}

#[tokio::test]
async fn an_app_of_the_organization_needs_no_admission() {
    let lab = lab_in(
        vec![],
        agent_roles(),
        Arc::new(RecordingPermissionService::new()),
        app_in("o1"),
    );
    let grant = agent_grant(PROJECT_AGENT_ROLE, Scope::Project(ProjectId::new("p1")));
    assert!(lab.grant(&service(), &grant).await.is_ok());
    assert_eq!(lab.registry.wakes(), [Some(AppId::new("agent-1"))]);
}

#[tokio::test]
async fn an_app_of_another_organization_is_refused() {
    let lab = lab_in(
        vec![],
        agent_roles(),
        Arc::new(RecordingPermissionService::new()),
        app_in("o2"),
    );
    for grant in [
        agent_grant(ORGANIZATION_AGENT_ROLE, org()),
        agent_grant(PROJECT_AGENT_ROLE, Scope::Project(ProjectId::new("p1"))),
    ] {
        let err = lab.grant(&service(), &grant).await.unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{err:?}");
    }
    assert!(lab.grants.created().is_empty());
}

#[tokio::test]
async fn an_agent_role_is_refused_to_a_user() {
    let lab = lab_with(
        vec![],
        agent_roles(),
        Arc::new(RecordingPermissionService::new()),
    );
    let grant = Grant::new(
        Principal::User(UserId::new("alice")),
        RoleName::new(ORGANIZATION_AGENT_ROLE).unwrap(),
        org(),
    );
    let err = lab.grant(&service(), &grant).await.unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)));
}

#[tokio::test]
async fn a_project_grant_manager_cannot_confer_more_than_it_holds() {
    let p1 = || Scope::Project(ProjectId::new("p1"));
    let roles = vec![
        test_role(PROJECT_ADMIN_ROLE, ScopeKind::Project, &[FULL_CONTROL]),
        test_role(
            "project-grants",
            ScopeKind::Project,
            &["manageProjectGrants", "readProject"],
        ),
        test_role(
            ORGANIZATION_MEMBER_ROLE,
            ScopeKind::Organization,
            &["readOrganization"],
        ),
    ];
    let bob = Principal::User(UserId::new("bob"));
    let alice = Principal::User(UserId::new("alice"));
    let lab = lab_with(
        vec![
            Grant::new(bob, RoleName::new("project-grants").unwrap(), p1()),
            Grant::new(
                alice.clone(),
                RoleName::new(ORGANIZATION_MEMBER_ROLE).unwrap(),
                org(),
            ),
        ],
        roles,
        Arc::new(RecordingPermissionService::new()),
    );
    let bob = CallerContext::User(UserId::new("bob"));

    let err = lab
        .grant(
            &bob,
            &Grant::new(
                alice.clone(),
                RoleName::new(PROJECT_ADMIN_ROLE).unwrap(),
                p1(),
            ),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert!(
        lab.grant(
            &bob,
            &Grant::new(alice, RoleName::new("project-grants").unwrap(), p1())
        )
        .await
        .is_ok()
    );
}

#[tokio::test]
async fn a_repeated_create_returns_the_stored_grant() {
    let existing = owner_grant(Principal::User(UserId::new("alice")), org());
    let lab = lab_with(
        vec![existing.clone()],
        vec![test_role(
            ORGANIZATION_ADMIN_ROLE,
            ScopeKind::Organization,
            &[FULL_CONTROL],
        )],
        Arc::new(RecordingPermissionService::new()),
    );

    let stored = lab
        .grant(
            &service(),
            &owner_grant(Principal::User(UserId::new("alice")), org()),
        )
        .await
        .unwrap();

    assert_eq!(stored, existing);
}

#[tokio::test]
async fn a_list_without_a_scope_asks_for_the_system_permission_and_returns_every_grant() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let other = Scope::Organization(OrganizationId::new("o2"));
    let lab = lab_with(
        vec![
            owner_grant(Principal::User(UserId::new("u1")), org()),
            owner_grant(Principal::User(UserId::new("u2")), other),
        ],
        vec![],
        permissions.clone(),
    );

    let grants = lab.list(None).await.unwrap();

    assert_eq!(grants.len(), 2);
    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageSystemGrants]
    );
}

#[tokio::test]
async fn a_list_with_a_scope_asks_for_that_scope_and_keeps_its_grants() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let other = Scope::Organization(OrganizationId::new("o2"));
    let lab = lab_with(
        vec![
            owner_grant(Principal::User(UserId::new("u1")), org()),
            owner_grant(Principal::User(UserId::new("u2")), other),
        ],
        vec![],
        permissions.clone(),
    );

    let grants = lab.list(Some(org())).await.unwrap();

    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].scope, org());
    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageOrgGrants(OrganizationId::new("o1"))]
    );
}

#[tokio::test]
async fn revoking_app_grant_disconnects_the_agent() {
    let grant = Grant::new(
        Principal::App(AppId::new("agent-1")),
        RoleName::new(ORGANIZATION_AGENT_ROLE).unwrap(),
        org(),
    );
    let lab = lab(vec![grant.clone()]);

    lab.revoke(&grant.id).await.unwrap();

    assert_eq!(lab.registry.disconnected(), vec![AppId::new("agent-1")]);
}

#[tokio::test]
async fn cannot_revoke_last_owner_of_scope() {
    let grant = owner_grant(Principal::User(UserId::new("u1")), org());
    let lab = lab(vec![grant.clone()]);

    assert!(
        lab.revoke(&grant.id).await.is_err(),
        "revoking the last owner must be blocked"
    );
}

#[tokio::test]
async fn can_revoke_owner_when_another_exists() {
    let g1 = owner_grant(Principal::User(UserId::new("u1")), org());
    let g2 = owner_grant(Principal::User(UserId::new("u2")), org());
    let lab = lab(vec![g1.clone(), g2]);

    assert!(
        lab.revoke(&g1.id).await.is_ok(),
        "revoking one of two owners is allowed"
    );
}

#[tokio::test]
async fn revoking_user_grant_leaves_agents_alone() {
    let grant = owner_grant(Principal::User(UserId::new("u1")), org());
    let co_owner = owner_grant(Principal::User(UserId::new("u2")), org());
    let lab = lab(vec![grant.clone(), co_owner]);

    lab.revoke(&grant.id).await.unwrap();

    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_revoke_checks_the_permission_on_the_grant_then_deletes_it() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let grant = Grant::new(
        Principal::App(AppId::new("agent-1")),
        RoleName::new(ORGANIZATION_AGENT_ROLE).unwrap(),
        org(),
    );
    let lab = lab_with(vec![grant.clone()], vec![], permissions.clone());

    let deleted = lab.revoke(&grant.id).await.unwrap().unwrap();

    assert_eq!(deleted.last_state(), &grant);
    assert_eq!(
        permissions.permissions(),
        vec![Permission::RevokeGrant(GrantId::new(grant.id.clone()))]
    );
    assert_eq!(lab.grants.deleted(), vec![grant.id]);
}

#[tokio::test]
async fn a_denied_revoke_never_deletes() {
    let grant = owner_grant(Principal::User(UserId::new("u1")), org());
    let co_owner = owner_grant(Principal::User(UserId::new("u2")), org());
    let lab = lab_with(
        vec![grant.clone(), co_owner],
        vec![],
        Arc::new(DenyingPermissionService::new()),
    );

    let err = lab.revoke(&grant.id).await.unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.grants.deleted().is_empty());
}

#[tokio::test]
async fn an_allowed_revoke_of_an_unknown_grant_changes_nothing() {
    let lab = lab(vec![]);

    assert!(lab.revoke("unknown").await.unwrap().is_none());
    assert!(lab.grants.deleted().is_empty());
}

#[tokio::test]
async fn revoke_all_access_refuses_to_strip_the_last_owner() {
    let alice = Principal::User(UserId::new("alice"));
    let lab = lab(vec![owner_grant(alice.clone(), org())]);

    assert!(
        lab.revoke_all_access(&service(), &alice, &org())
            .await
            .is_err(),
        "stripping the only owner of an organization must be refused"
    );
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn revoke_all_access_checks_the_scope_permission_and_disconnects_a_machine_principal() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let app = Principal::App(AppId::new("agent-1"));
    let project = Scope::Project(ProjectId::new("p1"));
    let lab = lab_with(vec![], vec![], permissions.clone());

    lab.revoke_all_access(&service(), &app, &project)
        .await
        .expect("revoke all");

    assert_eq!(
        permissions.permissions(),
        vec![Permission::ManageProjectGrants(ProjectId::new("p1"))]
    );
    assert_eq!(lab.registry.disconnected(), vec![AppId::new("agent-1")]);
}

#[tokio::test]
async fn revoke_all_access_at_system_scope_refuses_to_strip_the_last_admin_of_an_organization() {
    let alice = Principal::User(UserId::new("alice"));
    let lab = lab(vec![
        Grant::new(
            alice.clone(),
            RoleName::new(SYSTEM_ADMIN_ROLE).unwrap(),
            Scope::System,
        ),
        owner_grant(alice.clone(), org()),
    ]);

    let err = lab
        .revoke_all_access(&service(), &alice, &Scope::System)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::BusinessRule(_)));
    assert!(lab.grants.revoked_all().is_empty());
}

#[tokio::test]
async fn revoking_the_last_organization_grant_of_a_user_strips_the_access_beneath_it() {
    let alice = Principal::User(UserId::new("alice"));
    let member = Grant::new(
        alice.clone(),
        RoleName::new(ORGANIZATION_MEMBER_ROLE).unwrap(),
        org(),
    );
    let project = Grant::new(
        alice.clone(),
        RoleName::new(PROJECT_VIEWER_ROLE).unwrap(),
        Scope::Project(ProjectId::new("p1")),
    );
    let lab = lab(vec![member.clone(), project]);

    lab.revoke(&member.id).await.unwrap().unwrap();

    assert_eq!(lab.grants.revoked_all(), vec![(alice, org())]);
    assert!(lab.grants.deleted().is_empty());
}

#[tokio::test]
async fn revoking_one_of_two_organization_grants_of_a_user_deletes_only_it() {
    let alice = Principal::User(UserId::new("alice"));
    let member = Grant::new(
        alice.clone(),
        RoleName::new(ORGANIZATION_MEMBER_ROLE).unwrap(),
        org(),
    );
    let viewer = Grant::new(
        alice,
        RoleName::new(ORGANIZATION_VIEWER_ROLE).unwrap(),
        org(),
    );
    let lab = lab(vec![member.clone(), viewer]);

    lab.revoke(&member.id).await.unwrap().unwrap();

    assert_eq!(lab.grants.deleted(), vec![member.id]);
    assert!(lab.grants.revoked_all().is_empty());
}

#[tokio::test]
async fn the_last_project_admin_may_be_revoked() {
    let grant = Grant::new(
        Principal::User(UserId::new("u1")),
        RoleName::new(PROJECT_ADMIN_ROLE).unwrap(),
        Scope::Project(ProjectId::new("p1")),
    );
    let lab = lab(vec![grant.clone()]);

    assert!(lab.revoke(&grant.id).await.is_ok());
    assert_eq!(lab.grants.deleted(), vec![grant.id]);
}
