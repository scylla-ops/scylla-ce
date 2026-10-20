//! Wire to command, command outcome to wire. The handler holds none of it.

use crate::application::role::{
    CreateRole, DeleteRole, GetEffectivePermissions, GetMyPermissions, GetRole,
    ListAuthzVocabulary, ListRoles, UpdateRole,
};
use crate::domain::ids::OrganizationId;
use crate::domain::role::{RoleDescription, RoleDisplayName, RoleName};
use crate::grpc::convert::{
    Parse, optional, permission_from_key, permission_key, principal_ref_from_proto, required,
    scope_kind_from_proto, scope_kind_to_proto, scope_ref_to_proto, valid, wrap,
};
use crate::grpc::mappers::grant_mapper::{role_kind_from_proto, role_kind_to_proto};
use scylla_auth::authz::{EffectiveScope, FULL_CONTROL, Role, resource_home_scope};
use scylla_proto::authz::v1::{
    Access, AuthzAction, CreateRoleRequest, DeleteRoleRequest,
    EffectiveScope as ProtoEffectiveScope, GetEffectivePermissionsRequest, GetMyPermissionsRequest,
    GetRoleRequest, ListAuthzVocabularyRequest, ListRolesRequest, Permission, Role as ProtoRole,
    UpdateRoleRequest, access, role,
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
    type Into = CreateRole;

    fn parse(self) -> Result<CreateRole, Status> {
        Ok(CreateRole {
            organization_id: optional(self.organization_id).map(OrganizationId::new),
            kind: role_kind_from_proto(self.kind)?,
            scope: scope_kind_from_proto(self.scope_kind)?,
            permissions: permissions_from_proto(self.access)?,
            name: valid(self.name, RoleDisplayName::new)?,
            description: valid(self.description, RoleDescription::new)?,
        })
    }
}

impl Parse for UpdateRoleRequest {
    type Into = UpdateRole;

    fn parse(self) -> Result<UpdateRole, Status> {
        Ok(UpdateRole {
            id: role_id(self.role_id)?,
            permissions: permissions_from_proto(self.access)?,
            name: valid(self.name, RoleDisplayName::new)?,
            description: valid(self.description, RoleDescription::new)?,
        })
    }
}

fn role_id(field: Option<scylla_proto::common::v1::RoleId>) -> Result<RoleName, Status> {
    valid(required(field, "role_id")?, RoleName::new)
}

impl Parse for DeleteRoleRequest {
    type Into = DeleteRole;

    fn parse(self) -> Result<DeleteRole, Status> {
        Ok(DeleteRole {
            id: role_id(self.role_id)?,
        })
    }
}

impl Parse for GetRoleRequest {
    type Into = GetRole;

    fn parse(self) -> Result<GetRole, Status> {
        Ok(GetRole {
            id: role_id(self.role_id)?,
        })
    }
}

impl Parse for ListRolesRequest {
    type Into = ListRoles;

    fn parse(self) -> Result<ListRoles, Status> {
        Ok(ListRoles {
            organization_id: optional(self.organization_id).map(OrganizationId::new),
        })
    }
}

impl Parse for GetEffectivePermissionsRequest {
    type Into = GetEffectivePermissions;

    fn parse(self) -> Result<GetEffectivePermissions, Status> {
        Ok(GetEffectivePermissions {
            principal: principal_ref_from_proto(self.principal)?,
        })
    }
}

parse!(GetMyPermissionsRequest => GetMyPermissions);
parse!(ListAuthzVocabularyRequest => ListAuthzVocabulary);

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_auth::authz::ScopeKind;
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
    fn a_create_request_becomes_a_command_with_permission_keys() {
        let command = CreateRoleRequest {
            organization_id: None,
            kind: 0,
            name: "CI".to_string(),
            description: "runs the builds".to_string(),
            scope_kind: ProtoScopeKind::Project as i32,
            access: restricted(&[Permission::ReadPipeline, Permission::RunPipeline]),
        }
        .parse()
        .unwrap();

        assert_eq!(command.name.as_str(), "CI");
        assert_eq!(command.scope, ScopeKind::Project);
        assert_eq!(
            command.permissions,
            vec!["readPipeline".to_string(), "runPipeline".to_string()]
        );
    }

    #[test]
    fn full_control_becomes_the_wildcard() {
        let command = CreateRoleRequest {
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

        assert_eq!(command.permissions, vec![FULL_CONTROL.to_string()]);
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
    fn a_get_request_needs_a_role_id() {
        assert_eq!(
            GetRoleRequest {
                role_id: wrap("ci")
            }
            .parse()
            .unwrap()
            .id
            .as_str(),
            "ci"
        );
        let Err(err) = GetRoleRequest { role_id: None }.parse() else {
            panic!("a missing role id must be refused");
        };
        assert_eq!(err.code(), Code::InvalidArgument);
    }
}
