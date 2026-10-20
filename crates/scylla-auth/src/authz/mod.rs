pub mod entity_provider;
pub mod grant;
pub mod role;
pub mod service;
pub mod visibility;

pub use entity_provider::{AuthzEntityProvider, ResourceAncestors};
pub use grant::{
    Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, ORGANIZATION_AGENT_ROLE,
    ORGANIZATION_CREATOR_ROLE, ORGANIZATION_MEMBER_ROLE, ORGANIZATION_TRIGGER_RUNNER_ROLE,
    ORGANIZATION_VIEWER_ROLE, PROJECT_ADMIN_ROLE, PROJECT_AGENT_ROLE, PROJECT_DEVELOPER_ROLE,
    PROJECT_VIEWER_ROLE, Principal, PrincipalKind, SYSTEM_ADMIN_ROLE, Scope, ScopeKind,
    check_grantable, ensure_no_escalation, ensure_owner_remains, grantable_role,
};
pub use role::{
    EffectiveScope, FULL_CONTROL, ROLE_IN_USE, Role, RoleKind, RoleRepository, permissions_by_role,
    resource_home_scope, validate_role_permissions,
};
pub use service::PermissionService;
pub use visibility::{Visibility, VisibilityResolver, visibility_from_grants};
