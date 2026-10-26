//! Wire to the inputs of a use case, its outcome to wire. The handler holds none of it.

use crate::domain::ids::{GrantId, OrganizationId};
use crate::grpc::convert::{
    Parse, id, optional, principal_ref_from_proto, principal_ref_to_proto, required,
    scope_kind_from_proto, scope_kind_to_proto, scope_ref_from_proto, scope_ref_to_proto, valid,
    wrap,
};
use scylla_auth::authz::{Grant, Principal, Role, RoleKind, Scope, ScopeKind};
use scylla_domain::domain::role::RoleName;
use scylla_proto::authz::v1::{
    CreateGrantRequest, Grant as ProtoGrant, GrantableRole as ProtoGrantableRole,
    ListGrantableRolesRequest, ListGrantsRequest, RevokeAllAccessRequest, RevokeGrantRequest,
    RoleKind as ProtoRoleKind,
};
use tonic::Status;

pub fn grant_to_proto(g: &Grant) -> ProtoGrant {
    ProtoGrant {
        grant_id: wrap(g.id.clone()),
        principal: Some(principal_ref_to_proto(&g.principal)),
        scope: Some(scope_ref_to_proto(&g.scope)),
        role: wrap(g.role.to_string()),
    }
}

pub fn grantable_role_to_proto(r: &Role) -> ProtoGrantableRole {
    ProtoGrantableRole {
        role_id: wrap(r.id.clone()),
        scope_kind: scope_kind_to_proto(r.scope) as i32,
        kind: role_kind_to_proto(r.kind) as i32,
        description: r.description.to_string(),
        name: r.name.to_string(),
        owner_organization_id: r.owner_org.as_ref().and_then(|id| wrap(id.to_string())),
    }
}

pub fn role_kind_to_proto(kind: RoleKind) -> ProtoRoleKind {
    match kind {
        RoleKind::Admin => ProtoRoleKind::Admin,
        RoleKind::Member => ProtoRoleKind::Member,
        RoleKind::Agent => ProtoRoleKind::Agent,
    }
}

/// Unspecified is a member role; the use case refuses an admin one.
pub fn role_kind_from_proto(kind: i32) -> Result<RoleKind, Status> {
    match ProtoRoleKind::try_from(kind) {
        Ok(ProtoRoleKind::Unspecified | ProtoRoleKind::Member) => Ok(RoleKind::Member),
        Ok(ProtoRoleKind::Agent) => Ok(RoleKind::Agent),
        Ok(ProtoRoleKind::Admin) => Ok(RoleKind::Admin),
        Err(_) => Err(Status::invalid_argument("unknown role kind")),
    }
}

impl Parse for CreateGrantRequest {
    type Into = (Principal, RoleName, Scope);

    fn parse(self) -> Result<Self::Into, Status> {
        let principal = principal_ref_from_proto(self.principal)?;
        let scope = scope_ref_from_proto(self.scope)?;
        let role = valid(required(self.role, "role")?, RoleName::new)?;
        Ok((principal, role, scope))
    }
}

impl Parse for RevokeAllAccessRequest {
    type Into = (Principal, Scope);

    fn parse(self) -> Result<Self::Into, Status> {
        let principal = principal_ref_from_proto(self.principal)?;
        let scope = scope_ref_from_proto(self.scope)?;
        Ok((principal, scope))
    }
}

impl Parse for RevokeGrantRequest {
    type Into = GrantId;

    fn parse(self) -> Result<GrantId, Status> {
        id(self.grant_id, "grant_id")
    }
}

impl Parse for ListGrantsRequest {
    type Into = Option<Scope>;

    fn parse(self) -> Result<Self::Into, Status> {
        self.scope
            .map(|scope| scope_ref_from_proto(Some(scope)))
            .transpose()
    }
}

impl Parse for ListGrantableRolesRequest {
    type Into = (Option<ScopeKind>, Option<OrganizationId>);

    fn parse(self) -> Result<Self::Into, Status> {
        let scope_kind = self.scope_kind.map(scope_kind_from_proto).transpose()?;
        let organization_id = optional(self.organization_id).map(OrganizationId::new);
        Ok((scope_kind, organization_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_domain::domain::ids::UserId;
    use scylla_proto::authz::v1::ScopeRef;
    use tonic::Code;

    fn org_scope() -> Scope {
        Scope::Organization(OrganizationId::new("org-1"))
    }

    fn alice() -> Principal {
        Principal::User(UserId::new("alice"))
    }

    #[test]
    fn a_create_request_becomes_its_principal_role_and_scope() {
        let (principal, role, scope) = CreateGrantRequest {
            principal: Some(principal_ref_to_proto(&alice())),
            scope: Some(scope_ref_to_proto(&org_scope())),
            role: wrap("organization-admin"),
        }
        .parse()
        .unwrap();

        assert_eq!(principal, alice());
        assert_eq!(scope, org_scope());
        assert_eq!(role.as_str(), "organization-admin");
    }

    #[test]
    fn a_create_request_without_a_role_is_an_invalid_argument() {
        let Err(err) = CreateGrantRequest {
            principal: Some(principal_ref_to_proto(&alice())),
            scope: Some(scope_ref_to_proto(&org_scope())),
            role: None,
        }
        .parse() else {
            panic!("a missing role must be refused");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn a_create_request_checks_the_scope_before_the_role() {
        let err = CreateGrantRequest {
            principal: Some(principal_ref_to_proto(&alice())),
            scope: None,
            role: None,
        }
        .parse()
        .unwrap_err();

        assert_eq!(err.code(), Code::InvalidArgument);
        assert_eq!(err.message(), "missing scope");
    }

    #[test]
    fn an_empty_organization_id_of_a_grantable_roles_list_is_kept() {
        let (_, organization_id) = ListGrantableRolesRequest {
            scope_kind: None,
            organization_id: wrap(""),
        }
        .parse()
        .unwrap();

        assert_eq!(organization_id, Some(OrganizationId::new("")));
    }

    #[test]
    fn a_revoke_all_request_needs_a_scope() {
        let Err(err) = RevokeAllAccessRequest {
            principal: Some(principal_ref_to_proto(&alice())),
            scope: None,
        }
        .parse() else {
            panic!("a missing scope must be refused");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }

    #[test]
    fn a_list_request_keeps_its_optional_scope() {
        assert!(ListGrantsRequest { scope: None }.parse().unwrap().is_none());
        assert_eq!(
            ListGrantsRequest {
                scope: Some(scope_ref_to_proto(&org_scope())),
            }
            .parse()
            .unwrap(),
            Some(org_scope())
        );
    }

    #[test]
    fn a_list_request_with_an_unset_scope_union_is_an_invalid_argument() {
        let Err(err) = ListGrantsRequest {
            scope: Some(ScopeRef { scope: None }),
        }
        .parse() else {
            panic!("an unset scope union must be refused");
        };

        assert_eq!(err.code(), Code::InvalidArgument);
    }
}
