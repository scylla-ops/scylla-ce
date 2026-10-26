//! Wire to the inputs of a use case, its outcome to wire. The handler holds none of it.

use crate::domain::ids::OrganizationId;
use crate::domain::role::{RoleDescription, RoleDisplayName, RoleName};
use crate::grpc::convert::{
    Parse, optional, permission_from_key, permission_key, principal_ref_from_proto, required,
    scope_kind_from_proto, scope_kind_to_proto, scope_ref_to_proto, valid, wrap,
};
use crate::grpc::mappers::grant_mapper::{role_kind_from_proto, role_kind_to_proto};
use scylla_auth::authz::{
    EffectiveScope, FULL_CONTROL, Principal, Role, RoleKind, ScopeKind, resource_home_scope,
};
use scylla_proto::authz::v1::{
    Access, AuthzAction, CreateRoleRequest, DeleteRoleRequest,
    EffectiveScope as ProtoEffectiveScope, GetEffectivePermissionsRequest, GetRoleRequest,
    ListRolesRequest, Permission, Role as ProtoRole, UpdateRoleRequest, access, role,
};
use tonic::Status;

pub fn role_to_proto(role: &Role) -> ProtoRole {
    let origin = match &role.key {
        Some(key) => role::Origin::Builtin(role::Builtin { key: key.clone() }),
        None => role::Origin::Custom(role::Custom {
            owner_organization_id: role.owner_org.as_ref().and_then(|id| wrap(id.to_string())),
        }),
    };
    ProtoRole {
        role_id: wrap(role.id.clone()),
        name: role.name.to_string(),
        description: role.description.to_string(),
        scope_kind: scope_kind_to_proto(role.scope) as i32,
        access: Some(access_from_keys(role.is_full_control(), &role.permissions)),
        origin: Some(origin),
        kind: role_kind_to_proto(role.kind) as i32,
    }
}

pub fn effective_scope_to_proto(es: &EffectiveScope) -> ProtoEffectiveScope {
    ProtoEffectiveScope {
        scope: Some(scope_ref_to_proto(&es.scope)),
        access: Some(access_from_keys(es.full_control, &es.permissions)),
    }
}

/// `resource_type` is derivable from the permission; only `min_scope` ships.
pub fn authz_action_to_proto((key, resource_type): &(&str, &str)) -> AuthzAction {
    AuthzAction {
        permission: permission_from_key(key).map_or(0, |p| p as i32),
        min_scope: scope_kind_to_proto(resource_home_scope(resource_type)) as i32,
    }
}

fn access_from_keys(full_control: bool, keys: &[String]) -> Access {
    let inner = if full_control {
        access::Access::FullControl(access::FullControl {})
    } else {
        access::Access::Restricted(access::Restricted {
            permissions: keys
                .iter()
                .filter_map(|key| permission_from_key(key).map(|p| p as i32))
                .collect(),
        })
    };
    Access {
        access: Some(inner),
    }
}

fn permissions_from_proto(access: Option<Access>) -> Result<Vec<String>, Status> {
    let access = access.ok_or_else(|| Status::invalid_argument("access is required"))?;
    match access.access {
        Some(access::Access::FullControl(_)) => Ok(vec![FULL_CONTROL.to_string()]),
        Some(access::Access::Restricted(r)) => r
            .permissions
            .iter()
            .map(|&p| {
                let perm = Permission::try_from(p)
                    .map_err(|_| Status::invalid_argument("unknown permission value"))?;
                permission_key(perm)
                    .ok_or_else(|| Status::invalid_argument("permission unspecified"))
            })
            .collect(),
        None => Err(Status::invalid_argument(
            "access is required (full_control or restricted)",
        )),
    }
}

impl Parse for CreateRoleRequest {
    type Into = (
        Option<OrganizationId>,
        RoleDisplayName,
        RoleDescription,
        ScopeKind,
        RoleKind,
        Vec<String>,
    );

    fn parse(self) -> Result<Self::Into, Status> {
        let organization_id = optional(self.organization_id).map(OrganizationId::new);
        let kind = role_kind_from_proto(self.kind)?;
        let scope = scope_kind_from_proto(self.scope_kind)?;
        let permissions = permissions_from_proto(self.access)?;
        let name = valid(self.name, RoleDisplayName::new)?;
        let description = valid(self.description, RoleDescription::new)?;
        Ok((organization_id, name, description, scope, kind, permissions))
    }
}

impl Parse for UpdateRoleRequest {
    type Into = (RoleName, RoleDisplayName, RoleDescription, Vec<String>);

    fn parse(self) -> Result<Self::Into, Status> {
        let id = role_id(self.role_id)?;
        let permissions = permissions_from_proto(self.access)?;
        let name = valid(self.name, RoleDisplayName::new)?;
        let description = valid(self.description, RoleDescription::new)?;
        Ok((id, name, description, permissions))
    }
}

fn role_id(field: Option<scylla_proto::common::v1::RoleId>) -> Result<RoleName, Status> {
    valid(required(field, "role_id")?, RoleName::new)
}

impl Parse for DeleteRoleRequest {
    type Into = RoleName;

    fn parse(self) -> Result<RoleName, Status> {
        role_id(self.role_id)
    }
}

impl Parse for GetRoleRequest {
    type Into = RoleName;

    fn parse(self) -> Result<RoleName, Status> {
        role_id(self.role_id)
    }
}

impl Parse for ListRolesRequest {
    type Into = Option<OrganizationId>;

    fn parse(self) -> Result<Self::Into, Status> {
        Ok(optional(self.organization_id).map(OrganizationId::new))
    }
}

impl Parse for GetEffectivePermissionsRequest {
    type Into = Principal;

    fn parse(self) -> Result<Principal, Status> {
        principal_ref_from_proto(self.principal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_proto::authz::v1::ScopeKind as ProtoScopeKind;
    use tonic::Code;

    fn restricted(permissions: &[Permission]) -> Option<Access> {
        Some(Access {
            access: Some(access::Access::Restricted(access::Restricted {
                permissions: permissions.iter().map(|p| *p as i32).collect(),
            })),
        })
    }

    #[test]
    fn a_create_request_becomes_the_inputs_with_permission_keys() {
        let (_, name, _, scope, _, permissions) = CreateRoleRequest {
            organization_id: None,
            kind: 0,
            name: "CI".to_string(),
            description: "runs the builds".to_string(),
            scope_kind: ProtoScopeKind::Project as i32,
            access: restricted(&[Permission::ReadPipeline, Permission::RunPipeline]),
        }
        .parse()
        .unwrap();

        assert_eq!(name.as_str(), "CI");
        assert_eq!(scope, ScopeKind::Project);
        assert_eq!(
            permissions,
            vec!["readPipeline".to_string(), "runPipeline".to_string()]
        );
    }

    #[test]
    fn full_control_becomes_the_wildcard() {
        let (.., permissions) = CreateRoleRequest {
            organization_id: None,
            kind: 0,
            name: "Admin".to_string(),
            description: String::new(),
            scope_kind: ProtoScopeKind::Organization as i32,
            access: Some(Access {
                access: Some(access::Access::FullControl(access::FullControl {})),
            }),
        }
        .parse()
        .unwrap();

        assert_eq!(permissions, vec![FULL_CONTROL.to_string()]);
    }

    #[test]
    fn an_unspecified_scope_kind_is_an_invalid_argument() {
        let Err(err) = CreateRoleRequest {
            organization_id: None,
            kind: 0,
            name: "CI".to_string(),
            description: String::new(),
            scope_kind: ProtoScopeKind::Unspecified as i32,
            access: restricted(&[Permission::ReadPipeline]),
        }
        .parse() else {
            panic!("an unspecified scope kind must be refused");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn an_update_without_access_is_an_invalid_argument() {
        let Err(err) = UpdateRoleRequest {
            role_id: wrap("ci"),
            name: "CI".to_string(),
            description: String::new(),
            access: None,
        }
        .parse() else {
            panic!("a missing access must be refused");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn a_blank_name_is_an_invalid_argument() {
        for name in ["", "   "] {
            let Err(err) = CreateRoleRequest {
                organization_id: None,
                kind: 0,
                name: name.to_string(),
                description: String::new(),
                scope_kind: ProtoScopeKind::Project as i32,
                access: restricted(&[Permission::ReadPipeline]),
            }
            .parse() else {
                panic!("a blank create name must be refused");
            };
            assert_eq!(err.code(), Code::InvalidArgument);

            let Err(err) = UpdateRoleRequest {
                role_id: wrap("ci"),
                name: name.to_string(),
                description: String::new(),
                access: restricted(&[Permission::ReadPipeline]),
            }
            .parse() else {
                panic!("a blank update name must be refused");
            };
            assert_eq!(err.code(), Code::InvalidArgument);
        }
    }

    #[test]
    fn a_create_request_checks_the_scope_kind_before_the_name() {
        let err = CreateRoleRequest {
            organization_id: None,
            kind: 0,
            name: "  ".to_string(),
            description: String::new(),
            scope_kind: ProtoScopeKind::Unspecified as i32,
            access: restricted(&[Permission::ReadPipeline]),
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "unknown or unspecified scope kind");
    }

    #[test]
    fn an_update_request_checks_the_access_before_the_name() {
        let err = UpdateRoleRequest {
            role_id: wrap("ci"),
            name: String::new(),
            description: String::new(),
            access: None,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "access is required");
    }

    #[test]
    fn an_empty_organization_id_of_a_role_list_is_kept() {
        let organization_id = ListRolesRequest {
            organization_id: wrap(""),
        }
        .parse()
        .unwrap();

        assert_eq!(organization_id, Some(OrganizationId::new("")));
    }

    #[test]
    fn a_get_request_needs_a_role_id() {
        assert_eq!(
            GetRoleRequest {
                role_id: wrap("ci")
            }
            .parse()
            .unwrap()
            .as_str(),
            "ci"
        );
        let Err(err) = GetRoleRequest { role_id: None }.parse() else {
            panic!("a missing role id must be refused");
        };
        assert_eq!(err.code(), Code::InvalidArgument);
    }
}
