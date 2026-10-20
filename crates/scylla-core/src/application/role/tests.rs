//! The role's actions through the engine, on stub ports.

use super::*;
use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::errors::DomainError;
use crate::domain::ids::{OrganizationId, ProjectId, UserId};
use crate::domain::permission::Permission;
use crate::domain::role::{RoleDescription, RoleDisplayName, RoleName};
use crate::test_support::authz::{DenyingPermissionService, RecordingPermissionService, actions};
use crate::test_support::stubs::{StubGrants, StubRoles, alice};
use scylla_auth::authz::{Grant, PermissionService, Role, RoleKind, ScopeKind};
use scylla_extension::Actions;

struct Lab {
    actions: Actions,
    uc: RoleUseCases,
    roles: Arc<StubRoles>,
}

/// Every lab holds `root`, a system admin, so a write by `root` never escalates.
fn lab(permissions: Arc<dyn PermissionService>, roles: Vec<Role>, grants: Vec<Grant>) -> Lab {
    lab_using(permissions, roles, grants, &[])
}

fn lab_using(
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
        actions: actions(permissions),
        uc: RoleUseCases::new(roles.clone(), Arc::new(StubGrants::new(grants))),
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

fn create(permissions: &[&str]) -> CreateRole {
    CreateRole {
        organization_id: None,
        name: RoleDisplayName::new("CI Runner").unwrap(),
        description: RoleDescription::new("").unwrap(),
        scope: ScopeKind::Project,
        kind: RoleKind::Member,
        permissions: permissions.iter().map(ToString::to_string).collect(),
    }
}

fn create_in(organization: &str, permissions: &[&str]) -> CreateRole {
    CreateRole {
        organization_id: Some(OrganizationId::new(organization)),
        ..create(permissions)
    }
}

#[tokio::test]
async fn a_create_checks_manage_roles_then_stores() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone(), vec![], vec![]);

    let created = lab
        .actions
        .run(&lab.uc, &root(), create(&["readPipeline"]))
        .await
        .unwrap();

    assert_eq!(permissions.permissions(), vec![Permission::ManageRoles]);
    assert!(!created.builtin);
    assert!(created.owner_org.is_none());
    assert_eq!(custom(&lab), [created]);
}

#[tokio::test]
async fn a_denied_create_never_stores() {
    let lab = lab(Arc::new(DenyingPermissionService::new()), vec![], vec![]);

    let err = lab
        .actions
        .run(&lab.uc, &alice(), create(&["readPipeline"]))
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(custom(&lab).is_empty());
}

#[tokio::test]
async fn a_permission_out_of_the_role_scope_is_refused_before_anything_is_stored() {
    let lab = lab(Arc::new(RecordingPermissionService::new()), vec![], vec![]);

    let err = lab
        .actions
        .run(&lab.uc, &alice(), create(&["createOrganization"]))
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
    );
    let update = |permissions: &[&str]| UpdateRole {
        id: id("ci"),
        name: RoleDisplayName::new("CI").unwrap(),
        description: RoleDescription::new("runs the builds").unwrap(),
        permissions: permissions.iter().map(ToString::to_string).collect(),
    };

    let err = lab
        .actions
        .run(&lab.uc, &root(), update(&["createProject"]))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Validation(_)));

    let updated = lab
        .actions
        .run(&lab.uc, &root(), update(&["runPipeline"]))
        .await
        .unwrap();
    assert_eq!(updated.name.as_str(), "CI");
    assert_eq!(updated.permissions, vec!["runPipeline".to_string()]);
    assert_eq!(custom(&lab), [updated]);
}

#[tokio::test]
async fn a_missing_role_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new()), vec![], vec![]);

    let err = lab
        .actions
        .run(&lab.uc, &alice(), GetRole { id: id("ghost") })
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
    );
    let delete = |role: &str| DeleteRole { id: id(role) };

    for role in ["organization-admin", "ci"] {
        let err = lab
            .actions
            .run(&lab.uc, &root(), delete(role))
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::BusinessRule(_)), "{role}");
    }

    let deleted = lab
        .actions
        .run(&lab.uc, &root(), delete("unused"))
        .await
        .unwrap();
    assert_eq!(deleted.last_state().id, "unused");
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
    let lab = lab(permissions.clone(), vec![], vec![]);

    let created = lab
        .actions
        .run(&lab.uc, &root(), create_in("o1", &["readPipeline"]))
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
    let lab = lab(Arc::new(RecordingPermissionService::new()), vec![], vec![]);
    let system = CreateRole {
        scope: ScopeKind::System,
        ..create_in("o1", &["readOrganization"])
    };
    let admin = CreateRole {
        kind: RoleKind::Admin,
        ..create(&["readPipeline"])
    };

    for command in [system, admin] {
        let err = lab
            .actions
            .run(&lab.uc, &root(), command)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Validation(_)));
    }
    assert!(custom(&lab).is_empty());
}

#[tokio::test]
async fn a_role_holds_only_permissions_its_author_holds() {
    let dave = Principal::User(UserId::new("dave"));
    let lab = lab(
        Arc::new(RecordingPermissionService::new()),
        vec![role(
            "role-manager",
            ScopeKind::Organization,
            false,
            &["manageOrgRoles", "readProject"],
        )],
        vec![Grant::new(
            dave,
            id("role-manager"),
            Scope::Organization(OrganizationId::new("o1")),
        )],
    );
    let dave = CallerContext::User(UserId::new("dave"));

    let err = lab
        .actions
        .run(&lab.uc, &dave, create_in("o1", &["deleteProject"]))
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::BusinessRule(_)));
    let err = lab
        .actions
        .run(&lab.uc, &dave, create_in("o2", &["readProject"]))
        .await
        .unwrap_err();
    assert!(
        matches!(err, DomainError::BusinessRule(_)),
        "a grant on o1 confers nothing on o2"
    );

    let created = lab
        .actions
        .run(&lab.uc, &dave, create_in("o1", &["readProject"]))
        .await
        .unwrap();
    assert_eq!(created.owner_org, Some(OrganizationId::new("o1")));
}

#[tokio::test]
async fn an_update_checks_only_the_permissions_it_adds() {
    let dave = Principal::User(UserId::new("dave"));
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
            dave,
            id("role-manager"),
            Scope::Organization(OrganizationId::new("o1")),
        )],
    );
    let dave = CallerContext::User(UserId::new("dave"));
    let update = |role: &str, permissions: &[&str]| UpdateRole {
        id: id(role),
        name: RoleDisplayName::new("Renamed").unwrap(),
        description: RoleDescription::new("").unwrap(),
        permissions: permissions.iter().map(ToString::to_string).collect(),
    };

    lab.actions
        .run(
            &lab.uc,
            &dave,
            update("deployer", &["readProject", "runPipeline"]),
        )
        .await
        .expect("keeping a permission the author lacks is no escalation");
    lab.actions
        .run(
            &lab.uc,
            &dave,
            update("owner", &["readProject", "deleteProject"]),
        )
        .await
        .expect("narrowing full control adds nothing");
    let err = lab
        .actions
        .run(
            &lab.uc,
            &dave,
            update("deployer", &["runPipeline", "deleteProject"]),
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
    );

    let roles = lab
        .actions
        .run(
            &lab.uc,
            &root(),
            ListRoles {
                organization_id: Some(OrganizationId::new("o1")),
            },
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
                RoleName::new("ci").unwrap(),
                Scope::Project(ProjectId::new("p1")),
            ),
            Grant::new(
                alice_principal.clone(),
                RoleName::new("janitor").unwrap(),
                Scope::Organization(OrganizationId::new("o1")),
            ),
            Grant::new(
                alice_principal.clone(),
                RoleName::new("organization-admin").unwrap(),
                Scope::Organization(OrganizationId::new("o2")),
            ),
        ],
    );

    let scopes = lab
        .actions
        .run(
            &lab.uc,
            &CallerContext::Service(ServiceIdentity::recorder()),
            GetEffectivePermissions {
                principal: alice_principal,
            },
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
            RoleName::new("organization-admin").unwrap(),
            Scope::Organization(OrganizationId::new("o1")),
        )],
    );

    let scopes = lab
        .actions
        .run(&lab.uc, &alice(), GetMyPermissions)
        .await
        .unwrap();
    assert_eq!(scopes.len(), 1);
    assert!(scopes[0].full_control);

    let err = lab
        .actions
        .run(
            &lab.uc,
            &alice(),
            GetEffectivePermissions {
                principal: Principal::User(UserId::new("bob")),
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, DomainError::Forbidden(_)));

    for caller in [
        CallerContext::Service(ServiceIdentity::recorder()),
        CallerContext::Anonymous,
    ] {
        let err = lab
            .actions
            .run(&lab.uc, &caller, GetMyPermissions)
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Forbidden(_)), "{caller}");
    }
}

#[tokio::test]
async fn the_vocabulary_is_the_permission_catalog_and_asks_no_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone(), vec![], vec![]);

    let vocabulary = lab
        .actions
        .run(&lab.uc, &alice(), ListAuthzVocabulary)
        .await
        .unwrap();

    assert_eq!(
        vocabulary.len(),
        crate::domain::permission::PERMISSION_CATALOG.len()
    );
    assert!(permissions.permissions().is_empty());
}
