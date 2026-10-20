use crate::authz::grant::{Grant, Scope};
use crate::authz::role::FULL_CONTROL;
use crate::domain::caller::CallerContext;
use crate::domain::errors::DomainResult;
use crate::domain::ids::{OrganizationId, ProjectId};
use async_trait::async_trait;
use std::collections::HashMap;

/// Scopes, not ids: an organization grant covers projects created after it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Visibility {
    All,
    Scoped {
        orgs: Vec<OrganizationId>,
        projects: Vec<ProjectId>,
    },
}

impl Visibility {
    #[must_use]
    pub fn none() -> Self {
        Self::Scoped {
            orgs: Vec::new(),
            projects: Vec::new(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        matches!(self, Self::Scoped { orgs, projects } if orgs.is_empty() && projects.is_empty())
    }
}

#[async_trait]
pub trait VisibilityResolver: Send + Sync {
    async fn visible_scopes(
        &self,
        caller: &CallerContext,
        permission_key: &str,
    ) -> DomainResult<Visibility>;
}

#[must_use]
pub fn visibility_from_grants<S: std::hash::BuildHasher>(
    role_permissions: &HashMap<String, Vec<String>, S>,
    grants: &[Grant],
    principal: &crate::authz::grant::Principal,
    permission_key: &str,
) -> Visibility {
    let mut orgs = Vec::new();
    let mut projects = Vec::new();

    for grant in grants.iter().filter(|g| &g.principal == principal) {
        let confers = role_permissions
            .get(grant.role.as_str())
            .is_some_and(|perms| {
                perms
                    .iter()
                    .any(|p| p == FULL_CONTROL || p == permission_key)
            });
        if !confers {
            continue;
        }
        match &grant.scope {
            Scope::System => return Visibility::All,
            Scope::Organization(id) => orgs.push(id.clone()),
            Scope::Project(id) => projects.push(id.clone()),
        }
    }

    Visibility::Scoped { orgs, projects }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::grant::Principal;
    use crate::domain::ids::UserId;
    use crate::domain::role::RoleName;

    fn roles() -> HashMap<String, Vec<String>> {
        HashMap::from([
            ("admin".to_string(), vec![FULL_CONTROL.to_string()]),
            ("viewer".to_string(), vec!["readProject".to_string()]),
            ("runner".to_string(), vec!["runPipeline".to_string()]),
        ])
    }

    fn grant(role: &str, scope: Scope) -> Grant {
        Grant::new(
            Principal::User(UserId::new("alice")),
            RoleName::new(role).unwrap(),
            scope,
        )
    }

    #[test]
    fn a_system_grant_sees_everything() {
        let v = visibility_from_grants(
            &roles(),
            &[grant("admin", Scope::System)],
            &Principal::User(UserId::new("alice")),
            "readProject",
        );
        assert_eq!(v, Visibility::All);
    }

    #[test]
    fn org_and_project_grants_accumulate_by_scope() {
        let grants = vec![
            grant("viewer", Scope::Organization(OrganizationId::new("o1"))),
            grant("viewer", Scope::Project(ProjectId::new("p9"))),
        ];
        let v = visibility_from_grants(
            &roles(),
            &grants,
            &Principal::User(UserId::new("alice")),
            "readProject",
        );
        assert_eq!(
            v,
            Visibility::Scoped {
                orgs: vec![OrganizationId::new("o1")],
                projects: vec![ProjectId::new("p9")],
            }
        );
    }

    #[test]
    fn a_grant_that_does_not_confer_the_permission_is_ignored() {
        let v = visibility_from_grants(
            &roles(),
            &[grant("runner", Scope::Project(ProjectId::new("p1")))],
            &Principal::User(UserId::new("alice")),
            "readProject",
        );
        assert!(v.is_empty());
    }

    #[test]
    fn another_principals_grants_are_not_mine() {
        let v = visibility_from_grants(
            &roles(),
            &[grant("admin", Scope::System)],
            &Principal::User(UserId::new("bob")),
            "readProject",
        );
        assert!(v.is_empty());
    }
}
