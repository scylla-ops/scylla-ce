use super::authz::{action_key, euid, parent_set, principal_parts, resource_parts, resource_uid};
use crate::audit::{AuditDecision, AuditEntry, AuditLog};
use crate::authz::PermissionService;
use crate::authz::entity_provider::{AuthzEntityProvider, ResourceAncestors};
use crate::authz::grant::{Grant, GrantRepository, Principal, Scope};
use crate::authz::role::{Role, RoleRepository, permissions_by_role};
use crate::authz::visibility::{Visibility, VisibilityResolver, visibility_from_grants};
use crate::domain::caller::CallerContext;
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::permission::{Permission, ResourceRef};
use async_trait::async_trait;
use cedar_policy::{
    Authorizer, Context, Decision, Entities, Entity, EntityUid, PolicyId, PolicySet, Request,
    Schema, SlotId, Template, ValidationMode, Validator,
};
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use tracing::{error, info, instrument, warn};

const SCHEMA_SRC: &str = include_str!("schema.cedarschema");
const POLICIES_SRC: &str = include_str!("policies.cedar");

/// No condition in the body: holding the grant is the authority; `resource in ?resource` bounds it.
fn role_template_src(role: &Role) -> Option<String> {
    let action = if role.is_full_control() {
        "action".to_string()
    } else {
        let actions: Vec<String> = role
            .permissions
            .iter()
            .map(|key| format!("        Scylla::Action::\"{key}\""))
            .collect();
        if actions.is_empty() {
            return None;
        }
        format!("action in [\n{}\n    ]", actions.join(",\n"))
    };
    Some(format!(
        "permit (\n    principal == ?principal,\n    {action},\n    resource in ?resource\n);\n"
    ))
}

struct Built {
    version: i64,
    policies: PolicySet,
}

pub struct CedarPermissionService<EP: AuthzEntityProvider> {
    /// Built off-lock; the write lock is held only for the pointer swap.
    built: RwLock<Arc<Built>>,
    authorizer: Authorizer,
    entity_provider: Arc<EP>,
    role_repo: Arc<dyn RoleRepository>,
    grant_repo: Arc<dyn GrantRepository>,
    audit: Arc<dyn AuditLog>,
}

impl<EP: AuthzEntityProvider> CedarPermissionService<EP> {
    pub async fn new(
        entity_provider: Arc<EP>,
        role_repo: Arc<dyn RoleRepository>,
        grant_repo: Arc<dyn GrantRepository>,
        audit: Arc<dyn AuditLog>,
    ) -> DomainResult<Self> {
        let version = entity_provider.policy_version().await?;
        let built = Self::build(role_repo.as_ref(), grant_repo.as_ref(), version).await?;

        Ok(Self {
            built: RwLock::new(Arc::new(built)),
            authorizer: Authorizer::new(),
            entity_provider,
            role_repo,
            grant_repo,
            audit,
        })
    }

    /// The version is read before the rows, so a change committed in between bumps it again and
    /// the next check rebuilds.
    async fn build(
        role_repo: &dyn RoleRepository,
        grant_repo: &dyn GrantRepository,
        version: i64,
    ) -> DomainResult<Built> {
        let roles = role_repo.list_all().await?;
        let grants = grant_repo.list_all().await?;
        let policies = Self::build_policy_set(&roles, &grants)?;
        Ok(Built { version, policies })
    }

    fn current(&self) -> Arc<Built> {
        self.built.read().expect("policy set lock poisoned").clone()
    }

    /// Fails closed: when the rebuild fails, the check fails rather than decide on a set that may
    /// still hold a revoked grant.
    async fn policies(&self) -> DomainResult<Arc<Built>> {
        let version = self.entity_provider.policy_version().await?;
        let current = self.current();
        if current.version >= version {
            return Ok(current);
        }
        let built = Arc::new(
            Self::build(self.role_repo.as_ref(), self.grant_repo.as_ref(), version).await?,
        );
        let mut slot = self.built.write().expect("policy set lock poisoned");
        if slot.version < built.version {
            *slot = built.clone();
            info!(target: "audit", version, "authorization policy set rebuilt");
        }
        Ok(slot.clone())
    }

    fn build_policy_set(roles: &[Role], grants: &[Grant]) -> DomainResult<PolicySet> {
        let (schema, _warnings) = Schema::from_cedarschema_str(SCHEMA_SRC)
            .map_err(|e| DomainError::Internal(format!("cedar schema parse: {e}")))?;

        let validator = Validator::new(schema);
        let mut policies = PolicySet::from_str(POLICIES_SRC)
            .map_err(|e| DomainError::Internal(format!("cedar policy parse: {e}")))?;
        Self::validate(&validator, &policies)?;

        // One role that does not validate loses its access, not every organization theirs.
        for role in roles {
            if let Err(e) = Self::add_role_template(&validator, &mut policies, role) {
                error!(role_id = %role.id, error = %e, "skipping a role whose template does not validate");
            }
        }

        for grant in grants {
            if let Err(e) = Self::link_grant(&mut policies, grant) {
                warn!(grant_id = %grant.id, error = %e, "skipping unlinkable grant");
            }
        }

        Ok(policies)
    }

    fn validate(validator: &Validator, policies: &PolicySet) -> DomainResult<()> {
        let result = validator.validate(policies, ValidationMode::Strict);
        if result.validation_passed() {
            return Ok(());
        }
        let errs: Vec<String> = result
            .validation_errors()
            .map(ToString::to_string)
            .collect();
        Err(DomainError::Internal(format!(
            "cedar policy validation failed: {errs:?}"
        )))
    }

    fn add_role_template(
        validator: &Validator,
        policies: &mut PolicySet,
        role: &Role,
    ) -> DomainResult<()> {
        let Some(src) = role_template_src(role) else {
            return Ok(());
        };
        let template = Template::parse(Some(PolicyId::new(role.id.as_str())), &src)
            .map_err(|e| DomainError::Internal(format!("cedar template parse: {e}")))?;
        let mut alone = PolicySet::new();
        alone
            .add_template(template.clone())
            .map_err(|e| DomainError::Internal(format!("cedar add template: {e}")))?;
        Self::validate(validator, &alone)?;
        policies
            .add_template(template)
            .map_err(|e| DomainError::Internal(format!("cedar add template: {e}")))
    }

    fn link_grant(policies: &mut PolicySet, grant: &Grant) -> DomainResult<()> {
        let principal_uid = match &grant.principal {
            Principal::User(id) => euid("Scylla::User", id.as_str())?,
            Principal::App(id) => euid("Scylla::App", id.as_str())?,
        };
        let resource_uid = match &grant.scope {
            Scope::System => euid("Scylla::System", "root")?,
            Scope::Organization(id) => euid("Scylla::Organization", id.as_str())?,
            Scope::Project(id) => euid("Scylla::Project", id.as_str())?,
        };
        let vals = HashMap::from([
            (SlotId::principal(), principal_uid),
            (SlotId::resource(), resource_uid),
        ]);
        policies
            .link(
                PolicyId::new(grant.role.as_str()),
                PolicyId::new(format!("grant-{}", grant.id)),
                vals,
            )
            .map_err(|e| DomainError::Internal(format!("cedar link: {e}")))
    }

    fn principal_uid(caller: &CallerContext) -> DomainResult<EntityUid> {
        match caller {
            CallerContext::User(id) => euid("Scylla::User", id.as_str()),
            CallerContext::App(id) => euid("Scylla::App", id.as_str()),
            CallerContext::Service(svc) => euid("Scylla::Service", svc.as_str()),
            CallerContext::Anonymous => Err(DomainError::Forbidden(
                "Anonymous caller is not permitted".to_string(),
            )),
        }
    }

    fn resource_entities(
        resource: &ResourceRef,
        ancestors: &ResourceAncestors,
    ) -> DomainResult<(EntityUid, Vec<Entity>)> {
        fn uid_of(kind: &str, id: Option<&impl AsRef<str>>) -> DomainResult<Option<EntityUid>> {
            id.map(|id| euid(kind, id.as_ref())).transpose()
        }
        let uid = resource_uid(resource)?;
        let system_uid = euid("Scylla::System", "root")?;
        let org_uid = uid_of("Scylla::Organization", ancestors.organization.as_ref())?;
        let project_uid = uid_of("Scylla::Project", ancestors.project.as_ref())?;
        let pipeline_uid = uid_of("Scylla::Pipeline", ancestors.pipeline.as_ref())?;
        let app_uid = uid_of("Scylla::App", ancestors.app.as_ref())?;

        let mut entities = vec![Entity::new_no_attrs(system_uid.clone(), HashSet::new())];
        for (ancestor, parent) in [
            (&org_uid, Some(&system_uid)),
            (&project_uid, org_uid.as_ref()),
            (&pipeline_uid, project_uid.as_ref()),
            (&app_uid, org_uid.as_ref()),
        ] {
            if let Some(ancestor) = ancestor {
                entities.push(Entity::new_no_attrs(ancestor.clone(), parent_set(parent)));
            }
        }

        let nearest = match resource {
            ResourceRef::Job(_) | ResourceRef::Trigger(_) => pipeline_uid.as_ref(),
            ResourceRef::Pipeline(_) | ResourceRef::Secret(_) => project_uid.as_ref(),
            ResourceRef::Project(_) | ResourceRef::App(_) | ResourceRef::Role(_) => {
                org_uid.as_ref()
            }
            ResourceRef::AppSecret(_) => app_uid.as_ref(),
            ResourceRef::Grant(_) => project_uid.as_ref().or(org_uid.as_ref()),
            ResourceRef::Organization(_) | ResourceRef::User(_) | ResourceRef::System => None,
        };
        // An unknown resource sits under System: only a System grant reaches it, and the use case answers NotFound.
        if !matches!(resource, ResourceRef::System) {
            entities.push(Entity::new_no_attrs(
                uid.clone(),
                parent_set(Some(nearest.unwrap_or(&system_uid))),
            ));
        }

        Ok((uid, entities))
    }

    fn record_decision(
        &self,
        caller: &CallerContext,
        action: &'static str,
        resource: &ResourceRef,
        decision: AuditDecision,
        reason: Option<String>,
        policies: Vec<String>,
    ) {
        let (principal_kind, principal_id) = principal_parts(caller);
        let (resource_kind, resource_id) = resource_parts(resource);

        match decision {
            AuditDecision::Allow => info!(
                target: "audit",
                who = %caller, action, resource = %resource,
                decision = "allow", policies = ?policies, "action authorized"
            ),
            AuditDecision::Deny => warn!(
                target: "audit",
                who = %caller, action, resource = %resource,
                decision = "deny", policies = ?policies, "action denied"
            ),
        }

        self.audit.record(AuditEntry {
            occurred_at: Utc::now(),
            principal_kind,
            principal_id,
            action,
            resource_kind,
            resource_id,
            decision,
            policies,
            reason,
        });
    }
}

#[async_trait]
impl<EP: AuthzEntityProvider + 'static> PermissionService for CedarPermissionService<EP> {
    #[instrument(skip_all)]
    async fn check(&self, caller: &CallerContext, perm: Permission) -> DomainResult<()> {
        let resource = perm.resource();

        // Re-validated on every action: a long-lived stream must stop when its App is disabled or deleted.
        if let CallerContext::App(app_id) = caller {
            if !self.entity_provider.app_is_active(app_id).await? {
                self.record_decision(
                    caller,
                    perm.key(),
                    &resource,
                    AuditDecision::Deny,
                    Some("app principal is disabled or no longer exists".to_string()),
                    Vec::new(),
                );
                return Err(DomainError::Forbidden("Action denied".to_string()));
            }
        }

        let principal_uid = Self::principal_uid(caller)?;
        let ancestors = self.entity_provider.resource_ancestors(&resource).await?;
        let (resource_uid, resource_entities) = Self::resource_entities(&resource, &ancestors)?;
        let action = action_key(&perm, &ancestors);
        let action_uid = euid("Scylla::Action", action)?;

        let entities = Entities::from_entities(resource_entities, None)
            .map_err(|e| DomainError::Internal(format!("cedar entities: {e}")))?;

        let request = Request::new(
            principal_uid,
            action_uid,
            resource_uid,
            Context::empty(),
            None,
        )
        .map_err(|e| DomainError::Internal(format!("cedar request: {e}")))?;

        let built = self.policies().await?;
        let response = self
            .authorizer
            .is_authorized(&request, &built.policies, &entities);

        let policies: Vec<String> = response
            .diagnostics()
            .reason()
            .map(ToString::to_string)
            .collect();
        let (audit_decision, reason, result) = match response.decision() {
            Decision::Allow => (AuditDecision::Allow, None, Ok(())),
            Decision::Deny => {
                let errors: Vec<String> = response
                    .diagnostics()
                    .errors()
                    .map(ToString::to_string)
                    .collect();
                let reason = (!errors.is_empty()).then(|| errors.join("; "));
                (
                    AuditDecision::Deny,
                    reason,
                    Err(DomainError::Forbidden("Action denied".to_string())),
                )
            }
        };

        self.record_decision(caller, action, &resource, audit_decision, reason, policies);
        result
    }
}

#[async_trait]
impl<EP: AuthzEntityProvider + 'static> VisibilityResolver for CedarPermissionService<EP> {
    /// Not a Cedar query: Cedar decides one entity at a time and cannot filter a page.
    #[instrument(skip(self, caller))]
    async fn visible_scopes(
        &self,
        caller: &CallerContext,
        permission_key: &str,
    ) -> DomainResult<Visibility> {
        let principal = match caller {
            CallerContext::Service(_) => return Ok(Visibility::All),
            CallerContext::Anonymous => return Ok(Visibility::none()),
            _ => match Principal::from_caller(caller) {
                Some(p) => p,
                None => return Ok(Visibility::none()),
            },
        };

        let role_permissions = permissions_by_role(self.role_repo.as_ref()).await?;
        let grants = self.grant_repo.list_all().await?;

        Ok(visibility_from_grants(
            &role_permissions,
            &grants,
            &principal,
            permission_key,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::entity_provider::ResourceAncestors;
    use crate::authz::grant::{
        ORGANIZATION_ADMIN_ROLE, ORGANIZATION_AGENT_ROLE, ORGANIZATION_TRIGGER_RUNNER_ROLE,
        ORGANIZATION_VIEWER_ROLE, PROJECT_ADMIN_ROLE, PROJECT_AGENT_ROLE, PROJECT_DEVELOPER_ROLE,
        SYSTEM_ADMIN_ROLE, ScopeKind,
    };
    use crate::authz::role::{FULL_CONTROL, RoleKind};
    use crate::domain::caller::ServiceIdentity;
    use crate::domain::ids::{
        AppCredentialId, AppId, GrantId, JobId, OrganizationId, PipelineId, ProjectId, SecretId,
        TriggerId, UserId,
    };
    use crate::domain::role::{RoleDescription, RoleDisplayName, RoleName};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn builtin(id: &str, scope: ScopeKind, permissions: &[&str]) -> Role {
        Role {
            id: id.to_string(),
            key: Some(id.to_string()),
            name: RoleDisplayName::new(id).unwrap(),
            description: RoleDescription::new("").unwrap(),
            scope,
            kind: RoleKind::Member,
            owner_org: None,
            builtin: true,
            permissions: permissions.iter().map(ToString::to_string).collect(),
            version: 0,
        }
    }

    /// Must match the seed migration: the templates are generated from these.
    fn builtin_roles() -> Vec<Role> {
        let agent = &[
            "readPipeline",
            "executeJob",
            "writeJobStatus",
            "appendJobLog",
        ];
        vec![
            builtin(SYSTEM_ADMIN_ROLE, ScopeKind::System, &[FULL_CONTROL]),
            builtin(
                ORGANIZATION_ADMIN_ROLE,
                ScopeKind::Organization,
                &[FULL_CONTROL],
            ),
            builtin(PROJECT_ADMIN_ROLE, ScopeKind::Project, &[FULL_CONTROL]),
            builtin(ORGANIZATION_AGENT_ROLE, ScopeKind::Organization, agent),
            builtin(PROJECT_AGENT_ROLE, ScopeKind::Project, agent),
            builtin(
                ORGANIZATION_TRIGGER_RUNNER_ROLE,
                ScopeKind::Organization,
                &["runPipeline"],
            ),
            builtin(
                PROJECT_DEVELOPER_ROLE,
                ScopeKind::Project,
                &["readPipeline", "runPipeline", "createPipeline"],
            ),
            builtin(
                ORGANIZATION_VIEWER_ROLE,
                ScopeKind::Organization,
                &["readOrganization", "listOrganizationMembers"],
            ),
        ]
    }

    struct StubRoles(Vec<Role>);

    #[async_trait]
    impl RoleRepository for StubRoles {
        async fn list_all(&self) -> DomainResult<Vec<Role>> {
            Ok(self.0.clone())
        }
        async fn get(&self, id: &str) -> DomainResult<Option<Role>> {
            Ok(self.0.iter().find(|r| r.id == id).cloned())
        }
        async fn create(&self, _role: &Role) -> DomainResult<()> {
            Ok(())
        }
        async fn update(&self, _role: &Role) -> DomainResult<()> {
            Ok(())
        }
        async fn delete(&self, _role: &Role) -> DomainResult<()> {
            Ok(())
        }
        async fn in_use(&self, _id: &str) -> DomainResult<bool> {
            Ok(false)
        }
    }

    struct StubProvider {
        ancestors: ResourceAncestors,
        app_active: bool,
        version: AtomicI64,
    }

    #[async_trait]
    impl AuthzEntityProvider for StubProvider {
        async fn resource_ancestors(
            &self,
            _resource: &ResourceRef,
        ) -> DomainResult<ResourceAncestors> {
            Ok(self.ancestors.clone())
        }
        async fn app_is_active(&self, _app: &AppId) -> DomainResult<bool> {
            Ok(self.app_active)
        }
        async fn policy_version(&self) -> DomainResult<i64> {
            Ok(self.version.load(Ordering::SeqCst))
        }
    }

    struct StubGrants(Mutex<Vec<Grant>>);

    #[async_trait]
    impl GrantRepository for StubGrants {
        async fn list_all(&self) -> DomainResult<Vec<Grant>> {
            Ok(self.0.lock().unwrap().clone())
        }
        async fn revoke_all(&self, _p: &Principal, _s: &Scope) -> DomainResult<u64> {
            Ok(0)
        }
        async fn create(&self, grant: &Grant) -> DomainResult<Grant> {
            Ok(grant.clone())
        }
        async fn delete(&self, _id: &str) -> DomainResult<()> {
            Ok(())
        }
    }

    async fn service(
        ancestors: ResourceAncestors,
        grants: Vec<Grant>,
    ) -> CedarPermissionService<StubProvider> {
        service_with_roles(ancestors, builtin_roles(), grants).await
    }

    async fn service_with_roles(
        ancestors: ResourceAncestors,
        roles: Vec<Role>,
        grants: Vec<Grant>,
    ) -> CedarPermissionService<StubProvider> {
        CedarPermissionService::new(
            Arc::new(StubProvider {
                ancestors,
                app_active: true,
                version: AtomicI64::new(0),
            }),
            Arc::new(StubRoles(roles)),
            Arc::new(StubGrants(Mutex::new(grants))),
            Arc::new(crate::audit::NoopAuditLog),
        )
        .await
        .expect("schema + policies must parse, validate, and link")
    }

    async fn service_app_inactive(
        ancestors: ResourceAncestors,
        grants: Vec<Grant>,
    ) -> CedarPermissionService<StubProvider> {
        CedarPermissionService::new(
            Arc::new(StubProvider {
                ancestors,
                app_active: false,
                version: AtomicI64::new(0),
            }),
            Arc::new(StubRoles(builtin_roles())),
            Arc::new(StubGrants(Mutex::new(grants))),
            Arc::new(crate::audit::NoopAuditLog),
        )
        .await
        .expect("schema + policies must parse, validate, and link")
    }

    fn role(name: &str) -> RoleName {
        RoleName::new(name).unwrap()
    }

    #[tokio::test]
    async fn schema_and_policies_validate_and_admin_allows_everything() {
        let svc = service(
            ResourceAncestors::default(),
            vec![Grant::new(
                Principal::User(UserId::new("u-admin")),
                role("system-admin"),
                Scope::System,
            )],
        )
        .await;

        let caller = CallerContext::User(UserId::new("u-admin"));
        assert!(
            svc.check(&caller, Permission::DeleteUser(UserId::new("victim")))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_new_policy_version_applies_grants_and_revokes_on_the_next_check() {
        let provider = Arc::new(StubProvider {
            ancestors: ResourceAncestors::default(),
            app_active: true,
            version: AtomicI64::new(0),
        });
        let grants = Arc::new(StubGrants(Mutex::new(vec![])));
        let svc = CedarPermissionService::new(
            provider.clone(),
            Arc::new(StubRoles(builtin_roles())),
            grants.clone(),
            Arc::new(crate::audit::NoopAuditLog),
        )
        .await
        .unwrap();
        let caller = CallerContext::User(UserId::new("u"));
        let delete = || Permission::DeleteUser(UserId::new("victim"));
        assert!(svc.check(&caller, delete()).await.is_err());

        grants.0.lock().unwrap().push(Grant::new(
            Principal::User(UserId::new("u")),
            role("system-admin"),
            Scope::System,
        ));
        assert!(
            svc.check(&caller, delete()).await.is_err(),
            "same version: the built set stays"
        );

        provider.version.store(1, Ordering::SeqCst);
        assert!(svc.check(&caller, delete()).await.is_ok());

        grants.0.lock().unwrap().clear();
        provider.version.store(2, Ordering::SeqCst);
        assert!(svc.check(&caller, delete()).await.is_err());
    }

    #[tokio::test]
    async fn service_principal_bypasses() {
        let svc = service(ResourceAncestors::default(), vec![]).await;
        let caller = CallerContext::Service(ServiceIdentity::recorder());
        assert!(svc.check(&caller, Permission::CreateJob).await.is_ok());
    }

    #[tokio::test]
    async fn anonymous_denied() {
        let svc = service(ResourceAncestors::default(), vec![]).await;
        assert!(
            svc.check(&CallerContext::Anonymous, Permission::ListUsers)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn project_membership_alone_confers_nothing() {
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(&caller, Permission::RunPipeline(PipelineId::new("pl1")))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_project_role_is_what_confers_rights() {
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![Grant::new(
                Principal::User(UserId::new("u1")),
                role(PROJECT_DEVELOPER_ROLE),
                Scope::Project(ProjectId::new("p1")),
            )],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(&caller, Permission::RunPipeline(PipelineId::new("pl1")))
                .await
                .is_ok(),
            "the developer role confers runPipeline"
        );
        assert!(
            svc.check(&caller, Permission::DeleteProject(ProjectId::new("p1")))
                .await
                .is_err(),
            "and stops there: deleting the project is an admin action"
        );
    }

    #[tokio::test]
    async fn project_membership_confers_nothing_without_org_membership() {
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(&caller, Permission::RunPipeline(PipelineId::new("pl1")))
                .await
                .is_err(),
            "a project membership without the org membership must confer nothing"
        );
    }

    #[tokio::test]
    async fn project_member_denied_outside_scope() {
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p2")),
                pipeline: None,
                app: None,
            },
            vec![],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(&caller, Permission::RunPipeline(PipelineId::new("pl1")))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn linked_project_admin_grant_allows_in_scope() {
        let grant = Grant::new(
            Principal::User(UserId::new("u1")),
            role("project-admin"),
            Scope::Project(ProjectId::new("p1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(&caller, Permission::DeletePipeline(PipelineId::new("pl1")))
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_grant_authorizes_on_its_own() {
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![Grant::new(
                Principal::User(UserId::new("u1")),
                role(PROJECT_ADMIN_ROLE),
                Scope::Project(ProjectId::new("p1")),
            )],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(&caller, Permission::DeletePipeline(PipelineId::new("pl1")))
                .await
                .is_ok(),
            "the grant alone authorizes: nothing else has to be true"
        );

        let elsewhere = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p2")),
                pipeline: None,
                app: None,
            },
            vec![Grant::new(
                Principal::User(UserId::new("u1")),
                role(PROJECT_ADMIN_ROLE),
                Scope::Project(ProjectId::new("p1")),
            )],
        )
        .await;
        assert!(
            elsewhere
                .check(&caller, Permission::DeletePipeline(PipelineId::new("pl9")))
                .await
                .is_err(),
            "a grant on p1 confers nothing on p2"
        );
    }

    #[tokio::test]
    async fn system_scoped_grants_need_no_org_membership() {
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![Grant::new(
                Principal::User(UserId::new("u-admin")),
                role("system-admin"),
                Scope::System,
            )],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u-admin"));
        assert!(
            svc.check(&caller, Permission::DeletePipeline(PipelineId::new("pl1")))
                .await
                .is_ok(),
            "a system admin acts inside any org without being a member"
        );
    }

    #[tokio::test]
    async fn agent_app_grant_allows_execute_job_in_scope_only() {
        let grant = Grant::new(
            Principal::App(AppId::new("agent-1")),
            role(ORGANIZATION_AGENT_ROLE),
            Scope::Organization(OrganizationId::new("o1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::App(AppId::new("agent-1"));
        assert!(
            svc.check(&caller, Permission::ExecuteJob(PipelineId::new("pl1")))
                .await
                .is_ok(),
            "agent app may execute jobs within its granted org"
        );
        assert!(
            svc.check(&caller, Permission::DeletePipeline(PipelineId::new("pl1")))
                .await
                .is_err(),
            "agent role must not confer management actions"
        );
    }

    #[tokio::test]
    async fn trigger_runner_app_grant_allows_run_pipeline_in_scope() {
        let grant = Grant::new(
            Principal::App(AppId::new("trigger-runner-1")),
            role(ORGANIZATION_TRIGGER_RUNNER_ROLE),
            Scope::Organization(OrganizationId::new("o1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::App(AppId::new("trigger-runner-1"));
        assert!(
            svc.check(&caller, Permission::RunPipeline(PipelineId::new("pl1")))
                .await
                .is_ok(),
            "trigger-runner App must run pipelines within its granted org",
        );
        assert!(
            svc.check(&caller, Permission::ManageTriggers(PipelineId::new("pl1")))
                .await
                .is_err(),
            "runner App holds only runPipeline, never manageTriggers",
        );
    }

    #[tokio::test]
    async fn narrow_role_grant_allows_only_its_actions_in_scope() {
        let grant = Grant::new(
            Principal::User(UserId::new("alice")),
            role(ORGANIZATION_TRIGGER_RUNNER_ROLE),
            Scope::Organization(OrganizationId::new("o1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::User(UserId::new("alice"));
        assert!(
            svc.check(&caller, Permission::RunPipeline(PipelineId::new("pl1")))
                .await
                .is_ok(),
            "a direct runPipeline grant must allow running a pipeline beneath the org",
        );
        assert!(
            svc.check(&caller, Permission::DeletePipeline(PipelineId::new("pl1")))
                .await
                .is_err(),
            "a single-permission grant must not confer any other action",
        );
    }

    #[tokio::test]
    async fn disabled_app_denied_even_with_valid_grant() {
        let grant = Grant::new(
            Principal::App(AppId::new("agent-1")),
            role(ORGANIZATION_AGENT_ROLE),
            Scope::Organization(OrganizationId::new("o1")),
        );
        let svc = service_app_inactive(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::App(AppId::new("agent-1"));
        assert!(
            svc.check(&caller, Permission::ExecuteJob(PipelineId::new("pl1")))
                .await
                .is_err(),
            "a disabled app must be denied even where its grant would otherwise allow"
        );
    }

    #[tokio::test]
    async fn agent_app_denied_outside_scope() {
        let grant = Grant::new(
            Principal::App(AppId::new("agent-1")),
            role(ORGANIZATION_AGENT_ROLE),
            Scope::Organization(OrganizationId::new("o1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o2")),
                project: Some(ProjectId::new("p2")),
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::App(AppId::new("agent-1"));
        assert!(
            svc.check(&caller, Permission::ExecuteJob(PipelineId::new("pl1")))
                .await
                .is_err(),
            "agent app must be denied outside its granted scope"
        );
    }

    #[tokio::test]
    async fn org_admin_manages_agents_in_its_org_only() {
        let grant = Grant::new(
            Principal::User(UserId::new("u1")),
            role(ORGANIZATION_ADMIN_ROLE),
            Scope::Organization(OrganizationId::new("o1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: None,
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));

        for perm in [
            Permission::CreateAgent(OrganizationId::new("o1")),
            Permission::ListAgents(OrganizationId::new("o1")),
            Permission::ReadApp(AppId::new("agent-1")),
            Permission::ReadAppStats(AppId::new("agent-1")),
            Permission::DeleteApp(AppId::new("agent-1")),
        ] {
            assert!(
                svc.check(&caller, perm.clone()).await.is_ok(),
                "org admin may manage agents in its org: {perm:?}"
            );
        }

        assert!(
            svc.check(&caller, Permission::CreateAgent(OrganizationId::new("o2")))
                .await
                .is_err(),
            "org admin cannot create agents in another org"
        );
    }

    #[tokio::test]
    async fn user_without_grant_denied_agent_actions() {
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: None,
                pipeline: None,
                app: None,
            },
            vec![],
        )
        .await;
        let caller = CallerContext::User(UserId::new("nobody"));
        assert!(
            svc.check(&caller, Permission::ListAgents(OrganizationId::new("o1")))
                .await
                .is_err(),
            "a user with no grant cannot list agents"
        );
        assert!(
            svc.check(&caller, Permission::ReadAppStats(AppId::new("agent-1")))
                .await
                .is_err(),
            "a user with no grant cannot read agent stats"
        );
    }

    #[tokio::test]
    async fn org_admin_manages_grants_in_its_org_only() {
        let grant = Grant::new(
            Principal::User(UserId::new("u1")),
            role(ORGANIZATION_ADMIN_ROLE),
            Scope::Organization(OrganizationId::new("o1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: None,
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(
                &caller,
                Permission::ManageOrgGrants(OrganizationId::new("o1"))
            )
            .await
            .is_ok(),
            "org admin may manage grants in its own org"
        );
        assert!(
            svc.check(
                &caller,
                Permission::ManageOrgGrants(OrganizationId::new("o2"))
            )
            .await
            .is_err(),
            "org admin may not manage grants in another org (anti-escalation)"
        );
    }

    #[tokio::test]
    async fn org_admin_manages_project_grants_under_its_org() {
        let grant = Grant::new(
            Principal::User(UserId::new("u1")),
            role(ORGANIZATION_ADMIN_ROLE),
            Scope::Organization(OrganizationId::new("o1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(
                &caller,
                Permission::ManageProjectGrants(ProjectId::new("p1"))
            )
            .await
            .is_ok(),
            "org admin may manage grants on a project beneath its org"
        );
    }

    #[tokio::test]
    async fn project_admin_cannot_escalate_to_org_grants() {
        let grant = Grant::new(
            Principal::User(UserId::new("u1")),
            role(PROJECT_ADMIN_ROLE),
            Scope::Project(ProjectId::new("p1")),
        );
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![grant],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(
                &caller,
                Permission::ManageProjectGrants(ProjectId::new("p1"))
            )
            .await
            .is_ok(),
            "project admin manages grants on its project"
        );
        assert!(
            svc.check(
                &caller,
                Permission::ManageOrgGrants(OrganizationId::new("o1"))
            )
            .await
            .is_err(),
            "project admin cannot escalate to org-level grant management"
        );
    }

    #[tokio::test]
    async fn user_lists_its_own_organizations() {
        let svc = service(ResourceAncestors::default(), vec![]).await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            svc.check(
                &caller,
                Permission::ListUserOrganizations(UserId::new("u1"))
            )
            .await
            .is_ok(),
            "a user may list its own organizations"
        );
        assert!(
            svc.check(&caller, Permission::ListUserProjects(UserId::new("u1")))
                .await
                .is_ok(),
            "a user may list its own projects"
        );
        assert!(
            svc.check(&caller, Permission::UpdateUser(UserId::new("u1")))
                .await
                .is_ok(),
            "a user may update its own profile"
        );
        assert!(
            svc.check(&caller, Permission::DeleteUser(UserId::new("u1")))
                .await
                .is_err(),
            "self-deletion is not granted by the self policy"
        );
        assert!(
            svc.check(
                &caller,
                Permission::ListUserOrganizations(UserId::new("u2"))
            )
            .await
            .is_err(),
            "a user may not list another user's organizations"
        );
        assert!(
            svc.check(&caller, Permission::UpdateUser(UserId::new("u2")))
                .await
                .is_err(),
            "a user may not update another user's profile"
        );
    }

    #[tokio::test]
    async fn non_member_without_role_denied() {
        let svc = service(
            ResourceAncestors {
                organization: Some(OrganizationId::new("o1")),
                project: Some(ProjectId::new("p1")),
                pipeline: None,
                app: None,
            },
            vec![],
        )
        .await;
        let caller = CallerContext::User(UserId::new("nobody"));
        assert!(
            svc.check(&caller, Permission::ReadPipeline(PipelineId::new("pl1")))
                .await
                .is_err()
        );
    }

    fn secret_in(project: &str) -> ResourceAncestors {
        ResourceAncestors {
            organization: Some(OrganizationId::new("o1")),
            project: Some(ProjectId::new(project)),
            pipeline: None,
            app: None,
        }
    }

    fn project_admin() -> Vec<Grant> {
        vec![Grant::new(
            Principal::User(UserId::new("u1")),
            role(PROJECT_ADMIN_ROLE),
            Scope::Project(ProjectId::new("p1")),
        )]
    }

    #[tokio::test]
    async fn a_secret_delete_is_reached_through_the_secret_project() {
        let caller = CallerContext::User(UserId::new("u1"));
        let delete = || Permission::DeleteSecret(SecretId::new("s1"));

        let own = service(secret_in("p1"), project_admin()).await;
        assert!(own.check(&caller, delete()).await.is_ok());

        let other = service(secret_in("p2"), project_admin()).await;
        assert!(
            other.check(&caller, delete()).await.is_err(),
            "a grant on p1 confers nothing on a secret of p2"
        );
    }

    fn trigger_in(project: &str) -> ResourceAncestors {
        ResourceAncestors {
            organization: Some(OrganizationId::new("o1")),
            project: Some(ProjectId::new(project)),
            pipeline: Some(PipelineId::new(format!("{project}-pl"))),
            app: None,
        }
    }

    #[tokio::test]
    async fn a_trigger_action_is_reached_through_the_trigger_pipeline_project() {
        let caller = CallerContext::User(UserId::new("u1"));
        let manage = || Permission::ManageTrigger(TriggerId::new("t1"));
        let run = || Permission::RunTriggerPipeline(TriggerId::new("t1"));

        let own = service(trigger_in("p1"), project_admin()).await;
        assert!(own.check(&caller, manage()).await.is_ok());
        assert!(own.check(&caller, run()).await.is_ok());

        let other = service(trigger_in("p2"), project_admin()).await;
        assert!(other.check(&caller, manage()).await.is_err());
        assert!(other.check(&caller, run()).await.is_err());
    }

    #[tokio::test]
    async fn a_trigger_action_keeps_the_pipeline_action_roles() {
        let developer = service(
            trigger_in("p1"),
            vec![Grant::new(
                Principal::User(UserId::new("u1")),
                role(PROJECT_DEVELOPER_ROLE),
                Scope::Project(ProjectId::new("p1")),
            )],
        )
        .await;
        let caller = CallerContext::User(UserId::new("u1"));
        assert!(
            developer
                .check(
                    &caller,
                    Permission::RunTriggerPipeline(TriggerId::new("t1"))
                )
                .await
                .is_ok()
        );
        assert!(
            developer
                .check(&caller, Permission::ManageTrigger(TriggerId::new("t1")))
                .await
                .is_err(),
            "runPipeline does not confer manageTriggers"
        );
    }

    fn in_organization(organization: &str) -> ResourceAncestors {
        ResourceAncestors {
            organization: Some(OrganizationId::new(organization)),
            project: None,
            pipeline: None,
            app: None,
        }
    }

    fn org_grant(role_name: &str) -> Vec<Grant> {
        vec![Grant::new(
            Principal::User(UserId::new("u1")),
            role(role_name),
            Scope::Organization(OrganizationId::new("o1")),
        )]
    }

    fn app_secret_in(organization: &str) -> ResourceAncestors {
        ResourceAncestors {
            organization: Some(OrganizationId::new(organization)),
            project: None,
            pipeline: None,
            app: Some(AppId::new(format!("{organization}-app"))),
        }
    }

    #[tokio::test]
    async fn an_app_secret_action_is_reached_through_the_secret_app_organization() {
        let caller = CallerContext::User(UserId::new("u1"));
        let manage = || Permission::ManageAppSecret(AppCredentialId::new("s1"));

        let own = service(app_secret_in("o1"), org_grant(ORGANIZATION_ADMIN_ROLE)).await;
        assert!(own.check(&caller, manage()).await.is_ok());

        let other = service(app_secret_in("o2"), org_grant(ORGANIZATION_ADMIN_ROLE)).await;
        assert!(
            other.check(&caller, manage()).await.is_err(),
            "a grant on o1 confers nothing on a secret of an app of o2"
        );

        let viewer = service(app_secret_in("o1"), org_grant(ORGANIZATION_VIEWER_ROLE)).await;
        assert!(
            viewer.check(&caller, manage()).await.is_err(),
            "an organization viewer does not manage app secrets"
        );
    }

    fn system_admin() -> Vec<Grant> {
        vec![Grant::new(
            Principal::User(UserId::new("u-admin")),
            role(SYSTEM_ADMIN_ROLE),
            Scope::System,
        )]
    }

    #[tokio::test]
    async fn an_unknown_resource_is_reached_only_by_a_system_grant() {
        let unknown = [
            Permission::ReadProject(ProjectId::new("missing")),
            Permission::ReadPipeline(PipelineId::new("missing")),
            Permission::ReadJob(JobId::new("missing")),
            Permission::ReadApp(AppId::new("missing")),
            Permission::DeleteSecret(SecretId::new("missing")),
            Permission::ManageTrigger(TriggerId::new("missing")),
            Permission::ManageAppSecret(AppCredentialId::new("missing")),
            Permission::RevokeGrant(GrantId::new("missing")),
        ];
        let system = service(ResourceAncestors::default(), system_admin()).await;
        let org = service(
            ResourceAncestors::default(),
            org_grant(ORGANIZATION_ADMIN_ROLE),
        )
        .await;
        for perm in unknown {
            assert!(
                system
                    .check(&CallerContext::User(UserId::new("u-admin")), perm.clone())
                    .await
                    .is_ok(),
                "a System grant reaches {perm:?}"
            );
            assert!(
                org.check(&CallerContext::User(UserId::new("u1")), perm.clone())
                    .await
                    .is_err(),
                "an organization grant does not reach {perm:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_system_admin_may_delete_its_own_user() {
        let svc = service(ResourceAncestors::default(), system_admin()).await;
        assert!(
            svc.check(
                &CallerContext::User(UserId::new("u-admin")),
                Permission::DeleteUser(UserId::new("u-admin"))
            )
            .await
            .is_ok()
        );
    }

    #[tokio::test]
    async fn an_app_with_organization_admin_reads_itself() {
        let svc = service(
            in_organization("o1"),
            vec![Grant::new(
                Principal::App(AppId::new("a1")),
                role(ORGANIZATION_ADMIN_ROLE),
                Scope::Organization(OrganizationId::new("o1")),
            )],
        )
        .await;
        assert!(
            svc.check(
                &CallerContext::App(AppId::new("a1")),
                Permission::ReadApp(AppId::new("a1"))
            )
            .await
            .is_ok()
        );
    }

    fn grant_manager_roles() -> Vec<Role> {
        let manager = |id: &str, scope: ScopeKind, key: &str| Role {
            key: None,
            builtin: false,
            ..builtin(id, scope, &[key])
        };
        let mut roles = builtin_roles();
        roles.extend([
            manager("system-grants", ScopeKind::System, "manageSystemGrants"),
            manager("org-grants", ScopeKind::Organization, "manageOrgGrants"),
            manager("project-grants", ScopeKind::Project, "manageProjectGrants"),
        ]);
        roles
    }

    fn grant_in(organization: Option<&str>, project: Option<&str>) -> ResourceAncestors {
        ResourceAncestors {
            organization: organization.map(OrganizationId::new),
            project: project.map(ProjectId::new),
            ..Default::default()
        }
    }

    async fn may_revoke(ancestors: ResourceAncestors, role_name: &str, scope: Scope) -> bool {
        let svc = service_with_roles(
            ancestors,
            grant_manager_roles(),
            vec![Grant::new(
                Principal::User(UserId::new("u1")),
                role(role_name),
                scope,
            )],
        )
        .await;
        svc.check(
            &CallerContext::User(UserId::new("u1")),
            Permission::RevokeGrant(GrantId::new("g1")),
        )
        .await
        .is_ok()
    }

    #[tokio::test]
    async fn a_grant_revoke_takes_the_manage_grants_action_of_the_grant_scope() {
        let o1 = || Scope::Organization(OrganizationId::new("o1"));
        let p1 = || Scope::Project(ProjectId::new("p1"));
        let org_grant = || grant_in(Some("o1"), None);
        let project_grant = || grant_in(Some("o1"), Some("p1"));

        assert!(may_revoke(org_grant(), "org-grants", o1()).await);
        assert!(
            !may_revoke(project_grant(), "org-grants", o1()).await,
            "manageOrgGrants does not reach a project grant"
        );
        assert!(!may_revoke(grant_in(Some("o2"), None), "org-grants", o1()).await);
        assert!(!may_revoke(ResourceAncestors::default(), "org-grants", o1()).await);

        assert!(may_revoke(project_grant(), "project-grants", p1()).await);
        assert!(may_revoke(project_grant(), "project-grants", o1()).await);
        assert!(
            !may_revoke(org_grant(), "project-grants", o1()).await,
            "manageProjectGrants does not reach an organization grant"
        );

        assert!(may_revoke(project_grant(), ORGANIZATION_ADMIN_ROLE, o1()).await);
        assert!(
            !may_revoke(
                project_grant(),
                PROJECT_ADMIN_ROLE,
                Scope::Project(ProjectId::new("p2"))
            )
            .await
        );
    }

    #[tokio::test]
    async fn a_system_or_unknown_grant_is_reached_only_by_a_system_grant_manager() {
        let system = ResourceAncestors::default;

        assert!(may_revoke(system(), "system-grants", Scope::System).await);
        assert!(may_revoke(system(), SYSTEM_ADMIN_ROLE, Scope::System).await);
        assert!(
            !may_revoke(
                system(),
                ORGANIZATION_ADMIN_ROLE,
                Scope::Organization(OrganizationId::new("o1"))
            )
            .await
        );
        assert!(
            !may_revoke(grant_in(Some("o1"), None), "system-grants", Scope::System).await,
            "manageSystemGrants does not reach an organization grant"
        );
    }

    async fn may_manage_role(ancestors: ResourceAncestors, role_name: &str, scope: Scope) -> bool {
        let svc = service(
            ancestors,
            vec![Grant::new(
                Principal::User(UserId::new("u1")),
                role(role_name),
                scope,
            )],
        )
        .await;
        svc.check(
            &CallerContext::User(UserId::new("u1")),
            Permission::ManageRole(role("deployer")),
        )
        .await
        .is_ok()
    }

    #[tokio::test]
    async fn a_role_is_managed_with_the_manage_roles_action_of_its_owner() {
        let owned_by_o1 = || grant_in(Some("o1"), None);
        let o1 = || Scope::Organization(OrganizationId::new("o1"));
        let o2 = || Scope::Organization(OrganizationId::new("o2"));

        assert!(may_manage_role(owned_by_o1(), ORGANIZATION_ADMIN_ROLE, o1()).await);
        assert!(!may_manage_role(owned_by_o1(), ORGANIZATION_ADMIN_ROLE, o2()).await);
        assert!(may_manage_role(owned_by_o1(), SYSTEM_ADMIN_ROLE, Scope::System).await);
        assert!(
            !may_manage_role(ResourceAncestors::default(), ORGANIZATION_ADMIN_ROLE, o1()).await,
            "a platform role, or an unknown one, needs manageRoles on System"
        );
        assert!(
            may_manage_role(
                ResourceAncestors::default(),
                SYSTEM_ADMIN_ROLE,
                Scope::System
            )
            .await
        );
    }

    #[tokio::test]
    async fn a_role_of_an_organization_confers_rights_only_where_it_is_granted() {
        let mut roles = builtin_roles();
        roles.push(Role {
            owner_org: Some(OrganizationId::new("o1")),
            builtin: false,
            ..builtin("deployer", ScopeKind::Project, &["runPipeline"])
        });
        let grant = || {
            vec![Grant::new(
                Principal::User(UserId::new("u1")),
                role("deployer"),
                Scope::Project(ProjectId::new("p1")),
            )]
        };
        let caller = CallerContext::User(UserId::new("u1"));
        let run = || Permission::RunPipeline(PipelineId::new("pl"));

        let in_p1 =
            service_with_roles(grant_in(Some("o1"), Some("p1")), roles.clone(), grant()).await;
        assert!(in_p1.check(&caller, run()).await.is_ok());
        let in_o2 = service_with_roles(grant_in(Some("o2"), Some("p2")), roles, grant()).await;
        assert!(in_o2.check(&caller, run()).await.is_err());
    }

    #[tokio::test]
    async fn a_role_template_that_does_not_validate_loses_only_its_own_access() {
        let mut roles = builtin_roles();
        roles.push(builtin("broken", ScopeKind::System, &["noSuchAction"]));
        let svc = service_with_roles(
            ResourceAncestors::default(),
            roles,
            vec![
                Grant::new(
                    Principal::User(UserId::new("root")),
                    role(SYSTEM_ADMIN_ROLE),
                    Scope::System,
                ),
                Grant::new(
                    Principal::User(UserId::new("u1")),
                    role("broken"),
                    Scope::System,
                ),
            ],
        )
        .await;
        let delete = || Permission::DeleteUser(UserId::new("victim"));
        assert!(
            svc.check(&CallerContext::User(UserId::new("root")), delete())
                .await
                .is_ok()
        );
        assert!(
            svc.check(&CallerContext::User(UserId::new("u1")), delete())
                .await
                .is_err()
        );
    }
}
