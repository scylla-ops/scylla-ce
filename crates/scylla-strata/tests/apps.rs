use async_trait::async_trait;
use scylla_auth::authz::{
    Grant, ORGANIZATION_AGENT_ROLE, PermissionService, Scope, Visibility, VisibilityResolver,
};
use scylla_core::application::agent::{AgentDispatch, AgentStream, JobDispatch as AgentJob};
use scylla_core::application::app::{CreatedApp, CreatedAppSecret, mint_app_secret};
use scylla_core::application::{
    AppCredentialRepository, AppRepository, AppTokenRepository, HashService, PrincipalAccess,
};
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_domain::domain::agent::Agent;
use scylla_domain::domain::app::{
    App, AppCredential, AppKind, AppName, AppSecret, AppSecretHash, AppSecretLabel, AppToken,
    TRIGGER_RUNNER_APP_NAME,
};
use scylla_domain::domain::caller::CallerContext;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{AppCredentialId, AppId, JobId, OrganizationId, UserId};
use scylla_domain::domain::organization::OrganizationName;
use scylla_domain::domain::permission::Permission;
use scylla_domain::domain::role::{RoleDisplayName, RoleName};
use scylla_domain::domain::user::{Password, PasswordHash};
use scylla_proto::app::v1::{GetAppRequest, IssueTokenRequest};
use scylla_proto::common::v1::AppId as PbAppId;
use scylla_strata::grpc::{
    AppAuthServiceClient, AppAuthServiceServer, AppServiceClient, AppServiceServer,
};
use scylla_strata::{AccessModel, AppsRpc, CedarPermissions, ScyllaCtx};
use sqlx::types::chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use strata::{Config, Contextual, Edition};
use strata_tonic::Grpc;
use tonic::Code;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

#[strata::edition(modules(scylla_strata::Access, scylla_strata::Apps))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = CedarPermissions;
}

#[derive(Default)]
struct StubApps {
    rows: Mutex<HashMap<AppId, App>>,
    credentials: Mutex<Vec<AppCredential>>,
    access: Mutex<Vec<(AppId, PrincipalAccess)>>,
}

#[async_trait]
impl AppRepository for StubApps {
    async fn create_app(&self, app: &App, credential: &AppCredential) -> DomainResult<()> {
        self.rows
            .lock()
            .unwrap()
            .insert(app.id().clone(), app.clone());
        self.credentials.lock().unwrap().push(credential.clone());
        Ok(())
    }
    async fn provision_agent(
        &self,
        _: &App,
        _: &AppCredential,
        _: &Agent,
        _: &Grant,
    ) -> DomainResult<()> {
        unreachable!("no agent in an app action")
    }
    async fn provision(&self, _: &App, _: &Grant) -> DomainResult<()> {
        unreachable!("no grant in an app action")
    }
    async fn find_by_id(&self, id: &AppId) -> DomainResult<App> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("App", id))
    }
    async fn find_trigger_runner(&self, _: &OrganizationId) -> DomainResult<Option<AppId>> {
        unreachable!("only a trigger action looks up the runner")
    }
    async fn list_by_organization(
        &self,
        organization_id: &OrganizationId,
    ) -> DomainResult<Vec<App>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|a| a.organization_id() == organization_id)
            .cloned()
            .collect())
    }
    async fn set_active(&self, id: &AppId, active: bool) -> DomainResult<()> {
        let mut rows = self.rows.lock().unwrap();
        if let Some(a) = rows.get(id).cloned() {
            rows.insert(
                id.clone(),
                App::from_persistence(
                    a.id().clone(),
                    a.organization_id().clone(),
                    a.name().clone(),
                    a.kind(),
                    active,
                    a.created_at(),
                    a.updated_at(),
                ),
            );
        }
        Ok(())
    }
    async fn delete(&self, id: &AppId) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(id);
        Ok(())
    }
    async fn list_access(&self, id: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        Ok(self
            .access
            .lock()
            .unwrap()
            .iter()
            .filter(|(app, _)| app == id)
            .map(|(_, access)| access.clone())
            .collect())
    }
}

#[derive(Default)]
struct StubCredentials {
    rows: Mutex<HashMap<AppCredentialId, AppCredential>>,
}

impl StubCredentials {
    fn rewrite(&self, id: &AppCredentialId, label: Option<&AppSecretLabel>, enabled: Option<bool>) {
        let mut rows = self.rows.lock().unwrap();
        if let Some(c) = rows.get(id).cloned() {
            rows.insert(
                id.clone(),
                AppCredential::from_persistence(
                    c.id().clone(),
                    c.app_id().clone(),
                    label.cloned().unwrap_or_else(|| c.label().clone()),
                    c.secret_hash().clone(),
                    enabled.unwrap_or(c.is_enabled()),
                    c.created_at(),
                    c.updated_at(),
                ),
            );
        }
    }
}

#[async_trait]
impl AppCredentialRepository for StubCredentials {
    async fn create(&self, credential: &AppCredential) -> DomainResult<()> {
        self.rows
            .lock()
            .unwrap()
            .insert(credential.id().clone(), credential.clone());
        Ok(())
    }
    async fn find_by_id(&self, id: &AppCredentialId) -> DomainResult<AppCredential> {
        self.rows
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| DomainError::not_found("AppCredential", id))
    }
    async fn list_by_app(&self, app_id: &AppId) -> DomainResult<Vec<AppCredential>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|c| c.app_id() == app_id)
            .cloned()
            .collect())
    }
    async fn list_enabled_by_app(&self, app_id: &AppId) -> DomainResult<Vec<AppCredential>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .values()
            .filter(|c| c.app_id() == app_id && c.is_enabled())
            .cloned()
            .collect())
    }
    async fn set_enabled(&self, id: &AppCredentialId, enabled: bool) -> DomainResult<()> {
        self.rewrite(id, None, Some(enabled));
        Ok(())
    }
    async fn set_label(&self, id: &AppCredentialId, label: &AppSecretLabel) -> DomainResult<()> {
        let taken = {
            let rows = self.rows.lock().unwrap();
            let Some(c) = rows.get(id) else { return Ok(()) };
            rows.values()
                .any(|o| o.id() != id && o.app_id() == c.app_id() && o.label() == label)
        };
        if taken {
            return Err(DomainError::conflict(
                "App secret label already exists on this app",
            ));
        }
        self.rewrite(id, Some(label), None);
        Ok(())
    }
    async fn delete(&self, id: &AppCredentialId) -> DomainResult<()> {
        self.rows.lock().unwrap().remove(id);
        Ok(())
    }
}

#[derive(Default)]
struct StubHash {
    hashed: Mutex<usize>,
    verified: Mutex<usize>,
}

impl StubHash {
    fn hashed(&self) -> usize {
        *self.hashed.lock().unwrap()
    }
    fn verified(&self) -> usize {
        *self.verified.lock().unwrap()
    }
}

#[async_trait]
impl HashService for StubHash {
    async fn hash(&self, _: &Password) -> DomainResult<PasswordHash> {
        unreachable!("no password in an app action")
    }
    async fn verify(&self, _: &Password, _: &PasswordHash) -> DomainResult<bool> {
        unreachable!("no password in an app action")
    }
    async fn hash_secret(&self, secret: &AppSecret) -> DomainResult<AppSecretHash> {
        *self.hashed.lock().unwrap() += 1;
        AppSecretHash::new(format!("${}", secret.as_str()))
    }
    async fn verify_secret(&self, secret: &AppSecret, hash: &AppSecretHash) -> DomainResult<bool> {
        *self.verified.lock().unwrap() += 1;
        Ok(hash.as_str() == format!("${}", secret.as_str()))
    }
}

#[derive(Default)]
struct StubTokens {
    rows: Mutex<Vec<AppToken>>,
}

#[async_trait]
impl AppTokenRepository for StubTokens {
    async fn create(&self, token: &AppToken) -> DomainResult<()> {
        self.rows.lock().unwrap().push(token.clone());
        Ok(())
    }
    async fn find_by_token(&self, _: &str) -> DomainResult<AppToken> {
        unreachable!("the token exchange never reads a token")
    }
}

#[derive(Default)]
struct StubRegistry {
    disconnected: Mutex<Vec<AppId>>,
    wakes: Mutex<Vec<Option<AppId>>>,
}

impl StubRegistry {
    fn disconnected(&self) -> Vec<AppId> {
        self.disconnected.lock().unwrap().clone()
    }
    fn wakes(&self) -> Vec<Option<AppId>> {
        self.wakes.lock().unwrap().clone()
    }
}

impl AgentDispatch for StubRegistry {
    fn connected(&self) -> Vec<AgentStream> {
        Vec::new()
    }
    fn run(&self, _: &AgentStream, _: AgentJob) -> DomainResult<()> {
        unreachable!("no dispatch in an app action")
    }
    fn cancel(&self, _: &AppId, _: &JobId) -> DomainResult<()> {
        unreachable!("no cancel in an app action")
    }
    fn disconnect(&self, agent: &AppId) {
        self.disconnected.lock().unwrap().push(agent.clone());
    }
    fn wake(&self, agent: Option<&AppId>) {
        self.wakes.lock().unwrap().push(agent.cloned());
    }
}

struct SeeAll;

#[async_trait]
impl VisibilityResolver for SeeAll {
    async fn visible_scopes(&self, _: &CallerContext, _: &str) -> DomainResult<Visibility> {
        Ok(Visibility::All)
    }
}

struct Lab {
    app: TestCe,
    apps: Arc<StubApps>,
    credentials: Arc<StubCredentials>,
    hash: Arc<StubHash>,
    tokens: Arc<StubTokens>,
    registry: Arc<StubRegistry>,
}

async fn lab(permissions: Arc<dyn PermissionService>) -> Lab {
    let apps = Arc::new(StubApps::default());
    let credentials = Arc::new(StubCredentials::default());
    let hash = Arc::new(StubHash::default());
    let tokens = Arc::new(StubTokens::default());
    let registry = Arc::new(StubRegistry::default());
    let app = TestCe::builder(&Config::new())
        .provide(AccessModel {
            permissions,
            visibility: Arc::new(SeeAll),
        })
        .provide_arc::<dyn AppRepository>(apps.clone())
        .provide_arc::<dyn AppCredentialRepository>(credentials.clone())
        .provide_arc::<dyn AppTokenRepository>(tokens.clone())
        .provide_arc::<dyn HashService>(hash.clone())
        .provide_arc::<dyn AgentDispatch>(registry.clone())
        .build()
        .await
        .unwrap();
    Lab {
        app,
        apps,
        credentials,
        hash,
        tokens,
        registry,
    }
}

fn alice() -> ScyllaCtx {
    ScyllaCtx {
        caller: CallerContext::User(UserId::new("alice")),
        ..ScyllaCtx::default()
    }
}

fn anonymous() -> ScyllaCtx {
    ScyllaCtx::default()
}

fn org() -> OrganizationId {
    OrganizationId::new("acme")
}

fn ci_bot() -> AppName {
    AppName::new("ci-bot").unwrap()
}

impl Lab {
    async fn create(&self) -> DomainResult<CreatedApp> {
        self.create_with(ci_bot(), None).await
    }

    async fn create_with(
        &self,
        name: AppName,
        secret_label: Option<AppSecretLabel>,
    ) -> DomainResult<CreatedApp> {
        AppsRpc::create_app(&self.app.scoped(&alice()), org(), name, secret_label).await
    }

    async fn set_active(&self, id: &AppId, is_active: bool) -> DomainResult<App> {
        AppsRpc::set_app_active(&self.app.scoped(&alice()), id.clone(), is_active).await
    }

    async fn delete(&self, id: &AppId) -> DomainResult<()> {
        AppsRpc::delete_app(&self.app.scoped(&alice()), id.clone()).await
    }

    async fn create_secret(&self, app_id: &AppId) -> DomainResult<CreatedAppSecret> {
        self.create_secret_labelled(app_id, "ci").await
    }

    async fn create_secret_labelled(
        &self,
        app_id: &AppId,
        label: &str,
    ) -> DomainResult<CreatedAppSecret> {
        let label = AppSecretLabel::new(label).unwrap();
        AppsRpc::create_app_secret(&self.app.scoped(&alice()), app_id.clone(), label).await
    }

    async fn set_secret_label(
        &self,
        id: &AppCredentialId,
        label: &str,
    ) -> DomainResult<AppCredential> {
        let label = AppSecretLabel::new(label).unwrap();
        AppsRpc::set_app_secret_label(&self.app.scoped(&alice()), id.clone(), label).await
    }

    async fn revoke_secret(&self, id: &AppCredentialId) -> DomainResult<()> {
        AppsRpc::revoke_app_secret(&self.app.scoped(&alice()), id.clone()).await
    }

    async fn set_secret_enabled(
        &self,
        id: &AppCredentialId,
        enabled: bool,
    ) -> DomainResult<AppCredential> {
        AppsRpc::set_app_secret_enabled(&self.app.scoped(&alice()), id.clone(), enabled).await
    }

    async fn list_access(&self, app_id: &AppId) -> DomainResult<Vec<PrincipalAccess>> {
        AppsRpc::list_app_access(&self.app.scoped(&alice()), app_id.clone()).await
    }

    async fn issue(&self, app_id: &str, secret: &str) -> DomainResult<AppToken> {
        let secret = AppSecret::new(secret).unwrap();
        AppsRpc::issue_app_token(&self.app.scoped(&anonymous()), AppId::new(app_id), secret).await
    }
}

#[tokio::test]
async fn a_create_checks_the_permission_then_stores_the_app_with_its_default_secret() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;

    let created = lab.create().await.unwrap();

    assert_eq!(
        permissions.permissions(),
        vec![Permission::CreateApp(org())]
    );
    assert!(!created.secret.as_str().is_empty());
    assert!(lab.apps.rows.lock().unwrap().contains_key(created.app.id()));
    let credentials = lab.apps.credentials.lock().unwrap();
    assert_eq!(credentials.len(), 1);
    assert_eq!(credentials[0].app_id(), created.app.id());
    assert_eq!(credentials[0].label().as_str(), "ci-bot");
}

#[tokio::test]
async fn a_create_with_a_secret_label_names_the_first_secret_with_it() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let label = AppSecretLabel::new("build-01.example.com").unwrap();
    lab.create_with(ci_bot(), Some(label)).await.unwrap();

    let credentials = lab.apps.credentials.lock().unwrap();
    assert_eq!(credentials[0].label().as_str(), "build-01.example.com");
}

#[tokio::test]
async fn a_create_without_a_label_cuts_a_long_name_to_the_label_limit() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    lab.create_with(AppName::new("b".repeat(100)).unwrap(), None)
        .await
        .unwrap();

    let credentials = lab.apps.credentials.lock().unwrap();
    assert_eq!(credentials[0].label().as_str(), "b".repeat(64));
}

#[tokio::test]
async fn a_denied_create_never_hashes_or_persists() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;

    let err = lab.create().await.err().unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert_eq!(lab.hash.hashed(), 0);
    assert!(lab.apps.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn reads_check_their_permission() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    let caller = alice();
    let alice = lab.app.scoped(&caller);

    let app = AppsRpc::get_app(&alice, id.clone()).await.unwrap();
    let apps = AppsRpc::list_apps(&alice, org()).await.unwrap();
    let secrets = AppsRpc::list_app_secrets(&alice, id.clone()).await.unwrap();

    assert_eq!(app.id(), &id);
    assert_eq!(apps.len(), 1);
    assert!(secrets.is_empty());
    assert_eq!(
        permissions.permissions()[1..],
        [
            Permission::ReadApp(id.clone()),
            Permission::ListAppsByOrganization(org()),
            Permission::ReadApp(id)
        ]
    );
}

#[tokio::test]
async fn a_deactivation_disconnects_the_app_and_returns_the_fresh_row() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();

    let app = lab.set_active(&id, false).await.unwrap();

    assert!(!app.is_active());
    assert_eq!(lab.registry.disconnected(), vec![id.clone()]);
    assert_eq!(permissions.permissions()[1], Permission::DeleteApp(id));
}

#[tokio::test]
async fn an_activation_keeps_the_stream() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create().await.unwrap();

    lab.set_active(created.app.id(), true).await.unwrap();

    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_delete_closes_the_stream_and_wakes_the_dispatcher() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();

    lab.delete(&id).await.unwrap();

    assert!(lab.apps.rows.lock().unwrap().is_empty());
    assert_eq!(lab.registry.disconnected(), vec![id.clone()]);
    assert_eq!(lab.registry.wakes(), [None]);
    assert_eq!(permissions.permissions()[1], Permission::DeleteApp(id));
}

#[tokio::test]
async fn a_denied_delete_keeps_the_stream_and_the_row() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;

    let err = lab.delete(&AppId::new("app-1")).await.err().unwrap();

    assert!(matches!(err, DomainError::Forbidden(_)));
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_create_refuses_the_reserved_runner_name_before_hashing() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let err = lab
        .create_with(AppName::new(TRIGGER_RUNNER_APP_NAME).unwrap(), None)
        .await;

    assert!(matches!(err.err().unwrap(), DomainError::Validation(_)));
    assert_eq!(lab.hash.hashed(), 0);
    assert!(lab.apps.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn the_trigger_runner_cannot_be_disabled_deleted_or_given_a_secret() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let runner = App::trigger_runner(org()).unwrap();
    let id = runner.id().clone();
    lab.apps.rows.lock().unwrap().insert(id.clone(), runner);

    let disable = lab.set_active(&id, false).await.err().unwrap();
    let delete = lab.delete(&id).await.err().unwrap();
    let secret = lab.create_secret(&id).await.err().unwrap();

    assert!(matches!(disable, DomainError::BusinessRule(_)));
    assert!(matches!(delete, DomainError::BusinessRule(_)));
    assert!(matches!(secret, DomainError::BusinessRule(_)));
    assert!(lab.apps.rows.lock().unwrap()[&id].is_active());
    assert!(lab.credentials.rows.lock().unwrap().is_empty());
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn an_allowed_action_on_an_unknown_app_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let ghost = AppId::new("ghost");

    let delete = lab.delete(&ghost).await.err().unwrap();
    let disable = lab.set_active(&ghost, false).await.err().unwrap();

    assert!(matches!(delete, DomainError::NotFound(_)));
    assert!(matches!(disable, DomainError::NotFound(_)));
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_secret_create_checks_the_app_exists_then_stores_the_credential() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();

    let secret = lab.create_secret(&id).await.unwrap();

    assert_eq!(secret.credential.app_id(), &id);
    assert_eq!(secret.credential.label().as_str(), "ci");
    assert!(
        lab.credentials
            .rows
            .lock()
            .unwrap()
            .contains_key(secret.credential.id())
    );
    assert_eq!(permissions.permissions()[1], Permission::DeleteApp(id));
}

#[tokio::test]
async fn a_secret_for_a_missing_app_is_not_found_and_never_hashed() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;

    let err = lab.create_secret(&AppId::new("ghost")).await.err().unwrap();

    assert!(matches!(err, DomainError::NotFound(_)));
    assert_eq!(lab.hash.hashed(), 0);
    assert!(lab.credentials.rows.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_revoke_checks_the_permission_on_the_secret_then_disconnects_its_app() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    let secret = lab.create_secret(&id).await.unwrap();
    let secret_id = secret.credential.id().clone();

    lab.revoke_secret(&secret_id).await.unwrap();

    assert!(lab.credentials.rows.lock().unwrap().is_empty());
    assert_eq!(lab.registry.disconnected(), vec![id]);
    assert_eq!(
        permissions.permissions()[2],
        Permission::ManageAppSecret(secret_id)
    );
}

#[tokio::test]
async fn disabling_a_secret_disconnects_its_app_and_returns_the_fresh_row() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    let secret = lab.create_secret(&id).await.unwrap();
    let secret_id = secret.credential.id().clone();

    let credential = lab.set_secret_enabled(&secret_id, false).await.unwrap();

    assert!(!credential.is_enabled());
    assert_eq!(lab.registry.disconnected(), vec![id]);
    assert_eq!(
        permissions.permissions()[2],
        Permission::ManageAppSecret(secret_id)
    );
}

#[tokio::test]
async fn enabling_a_secret_keeps_the_stream() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create().await.unwrap();
    let secret = lab.create_secret(created.app.id()).await.unwrap();

    let credential = lab
        .set_secret_enabled(secret.credential.id(), true)
        .await
        .unwrap();

    assert!(credential.is_enabled());
    assert!(lab.registry.disconnected().is_empty());
}

#[tokio::test]
async fn a_rename_checks_the_permission_on_the_secret_and_keeps_the_stream() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create().await.unwrap();
    let secret = lab.create_secret(created.app.id()).await.unwrap();
    let secret_id = secret.credential.id().clone();

    let credential = lab
        .set_secret_label(&secret_id, "build-01.example.com")
        .await
        .unwrap();

    assert_eq!(credential.id(), &secret_id);
    assert_eq!(credential.label().as_str(), "build-01.example.com");
    assert!(credential.is_enabled());
    assert!(lab.registry.disconnected().is_empty());
    assert_eq!(
        permissions.permissions()[2],
        Permission::ManageAppSecret(secret_id)
    );
}

#[tokio::test]
async fn a_rename_to_a_label_of_the_same_app_conflicts_and_keeps_the_label() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    let first = lab.create_secret_labelled(&id, "first").await.unwrap();
    lab.create_secret_labelled(&id, "second").await.unwrap();

    let err = lab
        .set_secret_label(first.credential.id(), "second")
        .await
        .unwrap_err();
    let same = lab
        .set_secret_label(first.credential.id(), "first")
        .await
        .unwrap();

    assert!(matches!(err, DomainError::Conflict(_)));
    assert_eq!(same.label().as_str(), "first");
}

#[tokio::test]
async fn a_denied_secret_action_never_reads_or_writes() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;
    let credential = AppCredential::create(
        AppId::new("app-1"),
        AppSecretLabel::new("ci").unwrap(),
        lab.hash.hash_secret(&mint_app_secret()).await.unwrap(),
    );
    lab.credentials.create(&credential).await.unwrap();

    let revoke = lab.revoke_secret(credential.id()).await.unwrap_err();
    let disable = lab
        .set_secret_enabled(credential.id(), false)
        .await
        .unwrap_err();
    let rename = lab
        .set_secret_label(credential.id(), "renamed")
        .await
        .unwrap_err();

    assert!(matches!(revoke, DomainError::Forbidden(_)));
    assert!(matches!(disable, DomainError::Forbidden(_)));
    assert!(matches!(rename, DomainError::Forbidden(_)));
    assert!(lab.registry.disconnected().is_empty());
    let kept = lab.credentials.rows.lock().unwrap()[credential.id()].clone();
    assert!(
        kept.is_enabled(),
        "a denied action leaves the secret as it was"
    );
    assert_eq!(kept.label().as_str(), "ci");
}

#[tokio::test]
async fn an_allowed_action_on_an_unknown_secret_is_not_found() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let missing = AppCredentialId::new("missing");

    let revoke = lab.revoke_secret(&missing).await.unwrap_err();
    let disable = lab.set_secret_enabled(&missing, false).await.unwrap_err();
    let rename = lab.set_secret_label(&missing, "renamed").await.unwrap_err();

    assert!(matches!(revoke, DomainError::NotFound(_)));
    assert!(matches!(disable, DomainError::NotFound(_)));
    assert!(matches!(rename, DomainError::NotFound(_)));
    assert!(lab.registry.disconnected().is_empty());
}

fn agent_access(app_id: &AppId) -> PrincipalAccess {
    PrincipalAccess {
        grant_id: format!("{app_id}-acme"),
        scope: Scope::Organization(org()),
        organization_id: Some(org()),
        organization_name: Some(OrganizationName::new("Acme").unwrap()),
        project_name: None,
        role: RoleName::new(ORGANIZATION_AGENT_ROLE).unwrap(),
        role_name: RoleDisplayName::new("Organization agent").unwrap(),
    }
}

#[tokio::test]
async fn the_access_list_needs_read_app_and_an_unknown_app_holds_nothing() {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab(permissions.clone()).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    lab.apps
        .access
        .lock()
        .unwrap()
        .push((id.clone(), agent_access(&id)));

    let access = lab.list_access(&id).await.unwrap();
    let ghost = lab.list_access(&AppId::new("ghost")).await.unwrap();

    assert_eq!(access, vec![agent_access(&id)]);
    assert!(ghost.is_empty());
    assert_eq!(
        permissions.permissions()[1..],
        [
            Permission::ReadApp(id),
            Permission::ReadApp(AppId::new("ghost"))
        ]
    );
}

const SECRET: &str = "s3cr3t-s3cr3t-s3cr3t-s3cr3t-s3cr3t";
const OTHER: &str = "0th3r-0th3r-0th3r-0th3r-0th3r-0th3r";

async fn token_lab(active: bool) -> (Lab, AppCredentialId) {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;
    let app = App::from_persistence(
        AppId::new("app-1"),
        org(),
        AppName::new("ci").unwrap(),
        AppKind::Standard,
        active,
        Utc::now(),
        Utc::now(),
    );
    let credential = |secret: &str| {
        AppCredential::create(
            app.id().clone(),
            AppSecretLabel::new("default").unwrap(),
            AppSecretHash::new(format!("${secret}")).unwrap(),
        )
    };
    let matching = credential(SECRET);
    lab.credentials.create(&matching).await.unwrap();
    lab.credentials.create(&credential(OTHER)).await.unwrap();
    lab.apps.rows.lock().unwrap().insert(app.id().clone(), app);
    (lab, matching.id().clone())
}

#[tokio::test]
async fn a_matching_secret_stores_a_token_for_that_secret_without_asking_a_permission() {
    let (lab, matching) = token_lab(true).await;

    let token = lab.issue("app-1", SECRET).await.unwrap();

    assert_eq!(token.app_id(), &AppId::new("app-1"));
    assert_eq!(token.secret_id(), &matching);
    assert_eq!(lab.hash.verified(), 2);
    assert_eq!(lab.tokens.rows.lock().unwrap().len(), 1);
    assert_eq!((token.expires_at() - token.created_at()).num_days(), 30);
}

#[tokio::test]
async fn an_issued_app_token_expires_exactly_thirty_days_after_its_creation() {
    let (lab, _) = token_lab(true).await;

    let token = lab.issue("app-1", SECRET).await.unwrap();

    assert_eq!(
        (token.expires_at() - token.created_at()).to_std().unwrap(),
        std::time::Duration::from_secs(30 * 24 * 3600)
    );
}

#[tokio::test]
async fn an_unknown_app_an_inactive_app_and_a_wrong_secret_give_one_opaque_error() {
    let wrong = "wr0ng-wr0ng-wr0ng-wr0ng-wr0ng-wr0ng";
    for (active, app_id, secret) in [
        (true, "app-2", SECRET),
        (false, "app-1", SECRET),
        (true, "app-1", wrong),
    ] {
        let (lab, _) = token_lab(active).await;
        let err = lab.issue(app_id, secret).await.unwrap_err();
        assert!(matches!(&err, DomainError::Unauthorized(m) if m == "Invalid app credentials"));
        assert!(lab.tokens.rows.lock().unwrap().is_empty());
    }
}

async fn channel(router: tonic::transport::server::Router) -> Channel {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect()
        .await
        .unwrap()
}

fn get_app(id: &AppId, bearer: Option<&str>) -> tonic::Request<GetAppRequest> {
    let mut request = tonic::Request::new(GetAppRequest {
        app_id: Some(PbAppId {
            value: id.to_string(),
        }),
    });
    if let Some(bearer) = bearer {
        request
            .metadata_mut()
            .insert("authorization", bearer.parse().unwrap());
    }
    request
}

#[tokio::test]
async fn grpc_serves_the_app_service() {
    let lab = lab(Arc::new(RecordingPermissionService::new())).await;
    let created = lab.create().await.unwrap();
    let id = created.app.id().clone();
    let mut client = AppServiceClient::new(
        channel(Server::builder().add_service(AppServiceServer::new(Grpc::new(lab.app)))).await,
    );

    let got = client
        .get_app(get_app(&id, Some("Bearer user:alice")))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.app.unwrap().name, "ci-bot");

    let anonymous = client.get_app(get_app(&id, None)).await.unwrap_err();
    assert_eq!(anonymous.code(), Code::Unauthenticated, "{anonymous:?}");
    assert_eq!(
        anonymous.message(),
        "Missing or invalid authorization token"
    );
    let unparsed = client
        .get_app(tonic::Request::new(GetAppRequest { app_id: None }))
        .await
        .unwrap_err();
    assert_eq!(unparsed.code(), Code::Unauthenticated, "{unparsed:?}");
}

#[tokio::test]
async fn grpc_refuses_a_denied_caller_on_the_app_service() {
    let lab = lab(Arc::new(DenyingPermissionService::new())).await;
    let mut client = AppServiceClient::new(
        channel(Server::builder().add_service(AppServiceServer::new(Grpc::new(lab.app)))).await,
    );

    let denied = client
        .get_app(get_app(&AppId::new("app-1"), Some("Bearer user:alice")))
        .await
        .unwrap_err();
    assert_eq!(denied.code(), Code::PermissionDenied, "{denied:?}");
}

fn issue_token(secret: &str, bearer: Option<&str>) -> tonic::Request<IssueTokenRequest> {
    let mut request = tonic::Request::new(IssueTokenRequest {
        app_id: Some(PbAppId {
            value: "app-1".to_owned(),
        }),
        secret: secret.to_owned(),
    });
    if let Some(bearer) = bearer {
        request
            .metadata_mut()
            .insert("authorization", bearer.parse().unwrap());
    }
    request
}

#[tokio::test]
async fn grpc_issues_a_token_to_an_anonymous_caller() {
    let (lab, _) = token_lab(true).await;
    let tokens = lab.tokens.clone();
    let server = Server::builder().add_service(AppAuthServiceServer::new(Grpc::new(lab.app)));
    let mut client = AppAuthServiceClient::new(channel(server).await);

    let issued = client
        .issue_token(issue_token(SECRET, None))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(issued.token, tokens.rows.lock().unwrap()[0].token());
    assert!(issued.expires_at.is_some());

    let bad_token = client
        .issue_token(issue_token(SECRET, Some("Bearer nobody")))
        .await
        .unwrap();
    assert!(
        !bad_token.into_inner().token.is_empty(),
        "a public call ignores the bearer token"
    );

    let wrong = client
        .issue_token(issue_token(OTHER.replace('0', "o").as_str(), None))
        .await
        .unwrap_err();
    assert_eq!(wrong.code(), Code::Unauthenticated, "{wrong:?}");
    assert_eq!(wrong.message(), "Invalid app credentials");
}
