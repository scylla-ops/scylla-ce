use crate::authz::entity_provider::ResourceAncestors;
use crate::authz::grant::Scope;
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::permission::{Permission, ResourceRef};
use cedar_policy::EntityUid;
use std::collections::HashSet;
use std::str::FromStr;

pub(crate) fn euid(type_name: &str, id: &str) -> DomainResult<EntityUid> {
    EntityUid::from_str(&format!("{type_name}::\"{id}\""))
        .map_err(|e| DomainError::Internal(format!("cedar uid {type_name}::{id}: {e}")))
}

pub(crate) fn parent_set(parent: Option<&EntityUid>) -> HashSet<EntityUid> {
    parent.cloned().into_iter().collect()
}

/// A grant takes the manage-grants action of its scope kind, a role the manage-roles action of its
/// owner. An unknown grant or role has no ancestor and takes the System one.
pub(crate) fn action_key(permission: &Permission, ancestors: &ResourceAncestors) -> &'static str {
    match permission {
        Permission::RevokeGrant(_) => {
            let scope = match (&ancestors.project, &ancestors.organization) {
                (Some(project), _) => Scope::Project(project.clone()),
                (None, Some(organization)) => Scope::Organization(organization.clone()),
                (None, None) => Scope::System,
            };
            scope.manage_permission().key()
        }
        Permission::ManageRole(_) => match &ancestors.organization {
            Some(organization) => Permission::ManageOrgRoles(organization.clone()).key(),
            None => permission.key(),
        },
        _ => permission.key(),
    }
}

pub(crate) fn resource_uid(resource: &ResourceRef) -> DomainResult<EntityUid> {
    match resource {
        ResourceRef::System => euid("Scylla::System", "root"),
        ResourceRef::User(id) => euid("Scylla::User", id.as_str()),
        ResourceRef::Organization(id) => euid("Scylla::Organization", id.as_str()),
        ResourceRef::Project(id) => euid("Scylla::Project", id.as_str()),
        ResourceRef::Pipeline(id) => euid("Scylla::Pipeline", id.as_str()),
        ResourceRef::Job(id) => euid("Scylla::Job", id.as_str()),
        ResourceRef::Secret(id) => euid("Scylla::Secret", id.as_str()),
        ResourceRef::Trigger(id) => euid("Scylla::Trigger", id.as_str()),
        ResourceRef::App(id) => euid("Scylla::App", id.as_str()),
        ResourceRef::AppSecret(id) => euid("Scylla::AppSecret", id.as_str()),
        ResourceRef::Grant(id) => euid("Scylla::Grant", id.as_str()),
        ResourceRef::Role(id) => euid("Scylla::Role", id.as_str()),
    }
}

pub(crate) fn principal_parts(caller: &CallerContext) -> (&'static str, Option<String>) {
    match caller {
        CallerContext::User(id) => ("user", Some(id.as_str().to_string())),
        CallerContext::App(id) => ("app", Some(id.as_str().to_string())),
        CallerContext::Service(svc) => ("service", Some(svc.as_str().to_string())),
        CallerContext::Anonymous => ("anonymous", None),
    }
}

pub(crate) fn resource_parts(resource: &ResourceRef) -> (&'static str, Option<String>) {
    match resource {
        ResourceRef::System => ("system", None),
        ResourceRef::User(id) => ("user", Some(id.as_str().to_string())),
        ResourceRef::Organization(id) => ("organization", Some(id.as_str().to_string())),
        ResourceRef::Project(id) => ("project", Some(id.as_str().to_string())),
        ResourceRef::Pipeline(id) => ("pipeline", Some(id.as_str().to_string())),
        ResourceRef::Job(id) => ("job", Some(id.as_str().to_string())),
        ResourceRef::Secret(id) => ("secret", Some(id.as_str().to_string())),
        ResourceRef::Trigger(id) => ("trigger", Some(id.as_str().to_string())),
        ResourceRef::App(id) => ("app", Some(id.as_str().to_string())),
        ResourceRef::AppSecret(id) => ("app_secret", Some(id.as_str().to_string())),
        ResourceRef::Grant(id) => ("grant", Some(id.as_str().to_string())),
        ResourceRef::Role(id) => ("role", Some(id.as_str().to_string())),
    }
}
