use crate::authz::role::{FULL_CONTROL, Role, RoleKind};
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::ids::{AppId, OrganizationId, ProjectId, UserId};
use crate::domain::permission::Permission;
use crate::domain::role::RoleName;
use std::collections::BTreeSet;

pub const SYSTEM_ADMIN_ROLE: &str = "system-admin";
pub const ORGANIZATION_ADMIN_ROLE: &str = "organization-admin";
pub const PROJECT_ADMIN_ROLE: &str = "project-admin";
pub const ORGANIZATION_AGENT_ROLE: &str = "organization-agent";
pub const PROJECT_AGENT_ROLE: &str = "project-agent";
pub const ORGANIZATION_TRIGGER_RUNNER_ROLE: &str = "organization-trigger-runner";
pub const ORGANIZATION_VIEWER_ROLE: &str = "organization-viewer";
pub const ORGANIZATION_MEMBER_ROLE: &str = "organization-member";
pub const PROJECT_DEVELOPER_ROLE: &str = "project-developer";
pub const PROJECT_VIEWER_ROLE: &str = "project-viewer";
/// System scoped: create organizations, nothing else.
pub const ORGANIZATION_CREATOR_ROLE: &str = "organization-creator";

/// The system and every organization keep a human holder of their owner role; a project need not.
pub fn ensure_owner_remains(
    grants: &[Grant],
    removed: impl Fn(&Grant) -> bool,
) -> DomainResult<()> {
    let owner = |g: &Grant| {
        g.principal.kind() == PrincipalKind::User
            && matches!(g.role.as_str(), SYSTEM_ADMIN_ROLE | ORGANIZATION_ADMIN_ROLE)
    };
    let orphaned = grants.iter().filter(|g| owner(g) && removed(g)).find(|g| {
        !grants
            .iter()
            .any(|o| owner(o) && !removed(o) && o.scope == g.scope)
    });
    orphaned.map_or(Ok(()), |g| {
        Err(DomainError::business_rule(format!(
            "cannot remove the last owner of {}: appoint another first",
            g.scope
        )))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    System,
    Organization(OrganizationId),
    Project(ProjectId),
}

impl Scope {
    #[must_use]
    pub fn kind(&self) -> ScopeKind {
        match self {
            Self::System => ScopeKind::System,
            Self::Organization(_) => ScopeKind::Organization,
            Self::Project(_) => ScopeKind::Project,
        }
    }

    /// Cedar's `resource in ?resource` confines the caller to its own subtree; no trust in the caller.
    #[must_use]
    pub fn manage_permission(&self) -> Permission {
        match self {
            Self::System => Permission::ManageSystemGrants,
            Self::Organization(id) => Permission::ManageOrgGrants(id.clone()),
            Self::Project(id) => Permission::ManageProjectGrants(id.clone()),
        }
    }
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::System => write!(f, "system"),
            Self::Organization(id) => write!(f, "organization:{}", id.as_str()),
            Self::Project(id) => write!(f, "project:{}", id.as_str()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    System,
    Organization,
    Project,
}

impl ScopeKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Organization => "organization",
            Self::Project => "project",
        }
    }

    fn depth(self) -> u8 {
        match self {
            Self::System => 0,
            Self::Organization => 1,
            Self::Project => 2,
        }
    }

    #[must_use]
    pub fn covers(self, inner: ScopeKind) -> bool {
        self.depth() <= inner.depth()
    }
}

/// The one gate of `CreateGrant`: the role, its scope kind, an agent role for a user, and escalation.
pub fn check_grantable(
    roles: &[Role],
    grants: &[Grant],
    delegator: &CallerContext,
    grantee: PrincipalKind,
    role: &RoleName,
    scope: &Scope,
    organization: Option<&OrganizationId>,
) -> DomainResult<()> {
    let wanted = grantable_role(roles, grantee, role, scope, organization)?;
    ensure_no_escalation(
        roles,
        grants,
        delegator,
        &wanted.permissions,
        scope,
        organization,
    )
}

/// `check_grantable` without the escalation rule: the role exists and is usable in the
/// organization, its scope kind is the kind of `scope`, and an agent role goes to an app only.
pub fn grantable_role<'r>(
    roles: &'r [Role],
    grantee: PrincipalKind,
    role: &RoleName,
    scope: &Scope,
    organization: Option<&OrganizationId>,
) -> DomainResult<&'r Role> {
    let wanted = roles
        .iter()
        .find(|r| r.id == role.as_str() && r.usable_in(organization))
        .ok_or_else(|| DomainError::validation(format!("unknown role '{}'", role.as_str())))?;
    if wanted.scope != scope.kind() {
        return Err(DomainError::validation(format!(
            "role '{}' is grantable only on {} scope, not {}",
            role.as_str(),
            wanted.scope.as_str(),
            scope.kind().as_str()
        )));
    }
    if grantee == PrincipalKind::User && wanted.kind == RoleKind::Agent {
        return Err(DomainError::validation(format!(
            "role '{}' is for apps only",
            role.as_str()
        )));
    }
    Ok(wanted)
}

/// The delegator holds every wanted permission through a grant at System, at the organization or
/// at the scope itself. A service is trusted; `*` covers every permission.
pub fn ensure_no_escalation(
    roles: &[Role],
    grants: &[Grant],
    delegator: &CallerContext,
    wanted: &[String],
    scope: &Scope,
    organization: Option<&OrganizationId>,
) -> DomainResult<()> {
    let Some(delegator) = Principal::from_caller(delegator) else {
        return Ok(());
    };
    let reach = |s: &Scope| match s {
        Scope::System => true,
        Scope::Organization(o) if organization == Some(o) => true,
        s => s == scope,
    };
    let held: BTreeSet<&str> = grants
        .iter()
        .filter(|g| g.principal == delegator && reach(&g.scope))
        .filter_map(|g| roles.iter().find(|r| r.id == g.role.as_str()))
        .flat_map(|r| r.permissions.iter().map(String::as_str))
        .collect();
    if held.contains(FULL_CONTROL) || wanted.iter().all(|p| held.contains(p.as_str())) {
        Ok(())
    } else {
        Err(DomainError::business_rule(
            "cannot confer permissions you do not hold at this scope (no privilege escalation)",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrincipalKind {
    User,
    App,
}

impl PrincipalKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::App => "app",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    User(UserId),
    App(AppId),
}

impl Principal {
    #[must_use]
    pub fn from_caller(caller: &CallerContext) -> Option<Self> {
        match caller {
            CallerContext::User(id) => Some(Self::User(id.clone())),
            CallerContext::App(id) => Some(Self::App(id.clone())),
            CallerContext::Service(_) | CallerContext::Anonymous => None,
        }
    }

    #[must_use]
    pub fn kind(&self) -> PrincipalKind {
        match self {
            Self::User(_) => PrincipalKind::User,
            Self::App(_) => PrincipalKind::App,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::User(id) => id.as_str(),
            Self::App(id) => id.as_str(),
        }
    }
}

impl std::fmt::Display for Principal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind().as_str(), self.id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub id: String,
    pub principal: Principal,
    pub role: RoleName,
    pub scope: Scope,
}

impl Grant {
    #[must_use]
    pub fn new(principal: Principal, role: RoleName, scope: Scope) -> Self {
        Self {
            id: scylla_domain::domain::ids::new_id(),
            principal,
            role,
            scope,
        }
    }
}

mod repository;

pub use repository::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::caller::ServiceIdentity;
    use crate::domain::role::{RoleDescription, RoleDisplayName};

    type Removed = fn(&Grant) -> bool;
    const ADMIN: &str = ORGANIZATION_ADMIN_ROLE;

    fn scope(on: &str) -> Scope {
        match on {
            "system" => Scope::System,
            "p1" => Scope::Project(ProjectId::new(on)),
            _ => Scope::Organization(OrganizationId::new(on)),
        }
    }

    fn grants(rows: &[(&str, &str, &str)]) -> Vec<Grant> {
        rows.iter()
            .map(|(who, role, on)| {
                let principal = match who.strip_prefix("app:") {
                    Some(id) => Principal::App(AppId::new(id)),
                    None => Principal::User(UserId::new(*who)),
                };
                Grant::new(principal, RoleName::new(*role).unwrap(), scope(on))
            })
            .collect()
    }

    fn roles() -> [Role; 6] {
        [
            (ADMIN, ScopeKind::Organization, &[FULL_CONTROL][..]),
            (
                ORGANIZATION_AGENT_ROLE,
                ScopeKind::Organization,
                &["executeJob"],
            ),
            (PROJECT_ADMIN_ROLE, ScopeKind::Project, &[FULL_CONTROL]),
            (
                "project-grants",
                ScopeKind::Project,
                &["manageProjectGrants", "readProject"],
            ),
            ("project-reader", ScopeKind::Project, &["readProject"]),
            (
                "org-keys",
                ScopeKind::Organization,
                &["manageOrgGrants", "readOrganization"],
            ),
        ]
        .map(|(id, scope, permissions)| Role {
            id: id.to_string(),
            key: None,
            name: RoleDisplayName::new(id).unwrap(),
            description: RoleDescription::new("").unwrap(),
            scope,
            kind: if matches!(id, ORGANIZATION_AGENT_ROLE | PROJECT_AGENT_ROLE) {
                RoleKind::Agent
            } else {
                RoleKind::Member
            },
            owner_org: None,
            builtin: false,
            permissions: permissions.iter().map(ToString::to_string).collect(),
            version: 0,
        })
    }

    fn grantable(
        caller: &CallerContext,
        held: &[(&str, &str, &str)],
        role: &str,
        on: &str,
    ) -> DomainResult<()> {
        check_grantable(
            &roles(),
            &grants(held),
            caller,
            PrincipalKind::User,
            &RoleName::new(role).unwrap(),
            &scope(on),
            Some(&OrganizationId::new("o1")),
        )
    }

    #[test]
    fn a_delegator_confers_only_what_it_holds_on_the_scope_its_organization_or_system() {
        let alice = CallerContext::User(UserId::new("alice"));
        for (held, held_on, wanted, on, allowed) in [
            (ADMIN, "o1", PROJECT_ADMIN_ROLE, "p1", true),
            ("project-grants", "p1", "project-reader", "p1", true),
            ("project-grants", "p1", PROJECT_ADMIN_ROLE, "p1", false),
            ("org-keys", "o1", ADMIN, "o1", false),
            (ADMIN, "o2", "project-reader", "p1", false),
        ] {
            let result = grantable(&alice, &[("alice", held, held_on)], wanted, on);
            assert_eq!(result.is_ok(), allowed, "{held} gives {wanted}: {result:?}");
            assert!(allowed || matches!(result, Err(DomainError::BusinessRule(_))));
        }
    }

    #[test]
    fn an_unknown_role_a_role_of_another_scope_kind_or_an_agent_role_for_a_user_is_invalid() {
        let service = CallerContext::Service(ServiceIdentity::bootstrap());
        for wanted in ["no-such-role", PROJECT_ADMIN_ROLE, ORGANIZATION_AGENT_ROLE] {
            let err = grantable(&service, &[], wanted, "o1").unwrap_err();
            assert!(
                matches!(err, DomainError::Validation(_)),
                "{wanted}: {err:?}"
            );
        }
        assert!(grantable(&service, &[], ADMIN, "o1").is_ok());
    }

    #[test]
    fn grantable_role_gives_the_role_without_the_escalation_rule() {
        let roles = roles();
        let o1 = OrganizationId::new("o1");
        let lookup = |grantee, role: &str, on: &str| {
            grantable_role(
                &roles,
                grantee,
                &RoleName::new(role).unwrap(),
                &scope(on),
                Some(&o1),
            )
            .map(|r| r.id.clone())
        };

        assert_eq!(lookup(PrincipalKind::User, ADMIN, "o1").unwrap(), ADMIN);
        assert_eq!(
            lookup(PrincipalKind::App, ORGANIZATION_AGENT_ROLE, "o1").unwrap(),
            ORGANIZATION_AGENT_ROLE
        );
        for (grantee, wanted, on) in [
            (PrincipalKind::User, "no-such-role", "o1"),
            (PrincipalKind::User, PROJECT_ADMIN_ROLE, "o1"),
            (PrincipalKind::User, ORGANIZATION_AGENT_ROLE, "o1"),
        ] {
            let err = lookup(grantee, wanted, on).unwrap_err();
            assert!(
                matches!(err, DomainError::Validation(_)),
                "{wanted}: {err:?}"
            );
        }
    }

    #[test]
    fn a_role_of_another_organization_is_unknown() {
        let mut role = roles()[0].clone();
        role.id = "o2-role".to_string();
        role.owner_org = Some(OrganizationId::new("o2"));
        let result = grantable_role(
            std::slice::from_ref(&role),
            PrincipalKind::User,
            &RoleName::new("o2-role").unwrap(),
            &scope("o1"),
            Some(&OrganizationId::new("o1")),
        );
        assert!(matches!(result, Err(DomainError::Validation(_))));
    }

    #[test]
    fn a_removal_keeps_a_human_owner_on_the_system_and_on_each_organization() {
        let everything: Removed = |g| g.principal.id() == "alice";
        let on_o1: Removed = |g| g.principal.id() == "alice" && g.scope == scope("o1");
        let below_system: Removed = |g| g.principal.id() == "alice" && g.scope != Scope::System;
        let root = SYSTEM_ADMIN_ROLE;
        for (held, removed, kept) in [
            (vec![("alice", ADMIN, "o1")], everything, false),
            (
                vec![("alice", ADMIN, "o1"), ("bob", ADMIN, "o1")],
                everything,
                true,
            ),
            (
                vec![("alice", ORGANIZATION_MEMBER_ROLE, "o1")],
                everything,
                true,
            ),
            (
                vec![("alice", ADMIN, "o1"), ("app:agent-1", ADMIN, "o1")],
                everything,
                false,
            ),
            (
                vec![("alice", ADMIN, "o2"), ("bob", ADMIN, "o1")],
                on_o1,
                true,
            ),
            (
                vec![
                    ("alice", root, "system"),
                    ("root", root, "system"),
                    ("alice", ADMIN, "o1"),
                ],
                below_system,
                false,
            ),
            (vec![("alice", PROJECT_ADMIN_ROLE, "p1")], everything, true),
        ] {
            let result = ensure_owner_remains(&grants(&held), removed);
            assert_eq!(result.is_ok(), kept, "{held:?}");
            assert!(kept || matches!(result, Err(DomainError::BusinessRule(_))));
        }
    }
}
