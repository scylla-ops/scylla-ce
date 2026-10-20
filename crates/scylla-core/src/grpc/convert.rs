use crate::grpc::mappers::domain_error_to_status;
use chrono::{DateTime, TimeZone, Utc};
use prost_types::Timestamp;
use scylla_auth::authz::{Principal, Scope, ScopeKind};
use scylla_domain::domain::errors::DomainResult;
use scylla_domain::domain::ids::{AppId, OrganizationId, ProjectId, SessionId, UserId};
use scylla_domain::domain::session::SessionClient;
use scylla_proto::authz::v1::{
    Permission, PrincipalRef, ScopeKind as ProtoScopeKind, ScopeRef, principal_ref, scope_ref,
};
use scylla_proto::common::v1 as common;
use tonic::Status;

pub trait Wrapper: Sized {
    fn wrap(value: String) -> Self;
    fn into_value(self) -> String;
}

macro_rules! impl_wrapper {
    ($($t:ty),+ $(,)?) => {$(
        impl Wrapper for $t {
            fn wrap(value: String) -> Self { Self { value } }
            fn into_value(self) -> String { self.value }
        }
    )+};
}

impl_wrapper!(
    common::UserId,
    common::OrganizationId,
    common::ProjectId,
    common::PipelineId,
    common::JobId,
    common::JobLogId,
    common::AppId,
    common::AppSecretId,
    common::NodeId,
    common::SecretId,
    common::TriggerId,
    common::GrantId,
    common::RoleId,
    common::SessionId,
    common::Email,
);

pub fn wrap<T: Wrapper>(value: impl Into<String>) -> Option<T> {
    Some(T::wrap(value.into()))
}

pub fn required<T: Wrapper>(field: Option<T>, name: &str) -> Result<String, Status> {
    field
        .map(T::into_value)
        .ok_or_else(|| Status::invalid_argument(format!("missing {name}")))
}

pub fn optional<T: Wrapper>(field: Option<T>) -> Option<String> {
    field.map(T::into_value)
}

/// A required id wrapper as its domain id.
pub fn id<W: Wrapper, I: From<String>>(field: Option<W>, name: &str) -> Result<I, Status> {
    required(field, name).map(I::from)
}

/// An optional id wrapper as its domain id. A wrapper with an empty value is invalid.
pub fn optional_id<W: Wrapper, I: From<String>>(
    field: Option<W>,
    name: &str,
) -> Result<Option<I>, Status> {
    match optional(field) {
        Some(value) if value.is_empty() => Err(Status::invalid_argument(format!("invalid {name}"))),
        value => Ok(value.map(I::from)),
    }
}

/// A domain value built from a wire string; a validation failure is the client's fault.
pub fn valid<T>(
    value: impl Into<String>,
    new: impl FnOnce(String) -> DomainResult<T>,
) -> Result<T, Status> {
    new(value.into()).map_err(domain_error_to_status)
}

/// A request knows the command or query it becomes, so a handler never assembles one by hand.
pub trait Parse: Sized {
    type Into;

    fn parse(self) -> Result<Self::Into, Status>;
}

/// `Parse` for a request whose action spares the session of the call: the interceptor names that
/// session, not the request.
pub trait ParseInSession: Sized {
    type Into;

    fn parse_in_session(self, session: Option<SessionId>) -> Result<Self::Into, Status>;
}

/// `Parse` for a request whose action opens a session: the transport names the client, not the
/// request.
pub trait ParseWithClient: Sized {
    type Into;

    fn parse_with_client(self, client: SessionClient) -> Result<Self::Into, Status>;
}

#[must_use]
pub fn ts(dt: DateTime<Utc>) -> Option<Timestamp> {
    Some(Timestamp {
        seconds: dt.timestamp(),
        nanos: i32::try_from(dt.timestamp_subsec_nanos()).unwrap_or(0),
    })
}

#[must_use]
pub fn dt(ts: Option<Timestamp>) -> Option<DateTime<Utc>> {
    ts.and_then(|t| {
        Utc.timestamp_opt(t.seconds, u32::try_from(t.nanos).unwrap_or(0))
            .single()
    })
}

// prost strips the `PERMISSION_` prefix from the variant but not from `as_str_name()`.

const PERMISSION_PREFIX: &str = "PERMISSION_";

fn screaming_snake(camel: &str) -> String {
    let mut out = String::new();
    for ch in camel.chars() {
        if ch.is_ascii_uppercase() {
            out.push('_');
        }
        out.push(ch.to_ascii_uppercase());
    }
    out
}

fn camel_case(screaming: &str) -> String {
    let mut out = String::new();
    let mut upper_next = false;
    for ch in screaming.chars() {
        if ch == '_' {
            upper_next = true;
        } else if upper_next {
            out.push(ch);
            upper_next = false;
        } else {
            out.push(ch.to_ascii_lowercase());
        }
    }
    out
}

#[must_use]
pub fn permission_key(p: Permission) -> Option<String> {
    (p != Permission::Unspecified).then(|| {
        let name = p.as_str_name();
        camel_case(name.strip_prefix(PERMISSION_PREFIX).unwrap_or(name))
    })
}

#[must_use]
pub fn permission_from_key(key: &str) -> Option<Permission> {
    Permission::from_str_name(&format!("{PERMISSION_PREFIX}{}", screaming_snake(key)))
}

#[must_use]
pub fn scope_kind_to_proto(kind: ScopeKind) -> ProtoScopeKind {
    match kind {
        ScopeKind::System => ProtoScopeKind::System,
        ScopeKind::Organization => ProtoScopeKind::Organization,
        ScopeKind::Project => ProtoScopeKind::Project,
    }
}

pub fn scope_kind_from_proto(kind: i32) -> Result<ScopeKind, Status> {
    match ProtoScopeKind::try_from(kind) {
        Ok(ProtoScopeKind::System) => Ok(ScopeKind::System),
        Ok(ProtoScopeKind::Organization) => Ok(ScopeKind::Organization),
        Ok(ProtoScopeKind::Project) => Ok(ScopeKind::Project),
        Ok(ProtoScopeKind::Unspecified) | Err(_) => Err(Status::invalid_argument(
            "unknown or unspecified scope kind",
        )),
    }
}

#[must_use]
pub fn scope_ref_to_proto(scope: &Scope) -> ScopeRef {
    let inner = match scope {
        Scope::System => scope_ref::Scope::System(scope_ref::System {}),
        Scope::Organization(id) => scope_ref::Scope::Organization(scope_ref::Organization {
            organization_id: wrap(id.to_string()),
        }),
        Scope::Project(id) => scope_ref::Scope::Project(scope_ref::Project {
            project_id: wrap(id.to_string()),
        }),
    };
    ScopeRef { scope: Some(inner) }
}

pub fn scope_ref_from_proto(scope: Option<ScopeRef>) -> Result<Scope, Status> {
    let scope = scope.ok_or_else(|| Status::invalid_argument("missing scope"))?;
    match scope.scope {
        Some(scope_ref::Scope::System(_)) => Ok(Scope::System),
        Some(scope_ref::Scope::Organization(o)) => Ok(Scope::Organization(OrganizationId::new(
            &required(o.organization_id, "scope.organization.organization_id")?,
        ))),
        Some(scope_ref::Scope::Project(p)) => Ok(Scope::Project(ProjectId::new(&required(
            p.project_id,
            "scope.project.project_id",
        )?))),
        None => Err(Status::invalid_argument(
            "scope is required (system, organization or project)",
        )),
    }
}

#[must_use]
pub fn principal_ref_to_proto(principal: &Principal) -> PrincipalRef {
    let inner = match principal {
        Principal::User(id) => principal_ref::Principal::User(principal_ref::User {
            user_id: wrap(id.to_string()),
        }),
        Principal::App(id) => principal_ref::Principal::App(principal_ref::App {
            app_id: wrap(id.to_string()),
        }),
    };
    PrincipalRef {
        principal: Some(inner),
    }
}

pub fn principal_ref_from_proto(principal: Option<PrincipalRef>) -> Result<Principal, Status> {
    let principal = principal.ok_or_else(|| Status::invalid_argument("missing principal"))?;
    match principal.principal {
        Some(principal_ref::Principal::User(u)) => Ok(Principal::User(UserId::new(&required(
            u.user_id,
            "principal.user.user_id",
        )?))),
        Some(principal_ref::Principal::App(a)) => Ok(Principal::App(AppId::new(&required(
            a.app_id,
            "principal.app.app_id",
        )?))),
        None => Err(Status::invalid_argument(
            "principal is required (user or app)",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scylla_domain::domain::permission::PERMISSION_CATALOG;

    #[test]
    fn permission_catalog_matches_proto_enum() {
        for (key, _resource_type) in PERMISSION_CATALOG.iter() {
            let p = permission_from_key(key)
                .unwrap_or_else(|| panic!("no proto Permission for catalog key '{key}'"));
            assert_eq!(
                permission_key(p).as_deref(),
                Some(*key),
                "round-trip for {key}"
            );
        }
    }

    #[test]
    fn scope_ref_round_trips_every_arm() {
        for scope in [
            Scope::System,
            Scope::Organization(OrganizationId::new("org-1")),
            Scope::Project(ProjectId::new("proj-1")),
        ] {
            let round_tripped = scope_ref_from_proto(Some(scope_ref_to_proto(&scope))).unwrap();
            assert_eq!(format!("{round_tripped:?}"), format!("{scope:?}"));
        }
    }

    #[test]
    fn scope_ref_rejects_an_unset_union() {
        assert!(scope_ref_from_proto(Some(ScopeRef { scope: None })).is_err());
        assert!(scope_ref_from_proto(None).is_err());
    }
}
