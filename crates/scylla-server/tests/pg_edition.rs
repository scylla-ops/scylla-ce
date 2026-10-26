//! The CE edition on Postgres, built as an out-of-tree edition builds it: `strata_shared`, then
//! `EditionHost::edition`. These flows ran on the Actions engine before every slice ran on strata.

#![allow(clippy::unused_async_trait_impl)]

use async_trait::async_trait;
use scylla_auth::authz::{
    EffectiveScope, Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, ORGANIZATION_MEMBER_ROLE,
    ORGANIZATION_VIEWER_ROLE, Principal, Scope,
};
use scylla_core::application::app::mint_app_secret;
use scylla_core::application::{
    AppRepository, AppTokenRepository, HashService, OrganizationRepository, PasswordResetDelivery,
    PasswordResetMessage, PasswordResetSender, ProjectRepository, SessionRepository,
    UserRepository,
};
use scylla_core::config::ControlPlaneConfig;
use scylla_core::domain::app::{App, AppCredential, AppName, AppSecret, AppSecretLabel, AppToken};
use scylla_core::domain::caller::{CallerContext, ServiceIdentity};
use scylla_core::domain::errors::{DomainError, DomainResult};
use scylla_core::domain::ids::{OrganizationId, UserId};
use scylla_core::domain::organization::{Organization, OrganizationName};
use scylla_core::domain::permission::Permission;
use scylla_core::domain::project::{Project, ProjectDescription, ProjectName};
use scylla_core::domain::role::RoleName;
use scylla_core::domain::user::{
    DisplayName, Email, Password, RESET_LINK_INVALID, ResetToken, User, Username,
};
use scylla_core::infrastructure::{Argon2HashService, LogPasswordResetSender};
use scylla_db::{
    PgAppRepository, PgAppTokenRepository, PgGrantRepository, PgOrganizationRepository,
    PgProjectRepository, PgSessionRepository, PgUserRepository,
};
use scylla_strata::{
    Access, Agents, Apps, AppsRpc, Auth, AuthRpc, EditionHost, Grants, GrantsRpc, JobControl,
    Organizations, Permissions, Pipelines, Projects, ProjectsRpc, Roles, RolesRpc, ScyllaCeOnDb,
    ScyllaCeOnDbApp, ScyllaCtx, ScyllaRepos, Secrets, Slice, Triggers, Users, UsersRpc,
};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use strata::{BuildCx, BuildError, Component, Contextual, Edition};

/// Grants each permission to each caller.
#[derive(Default)]
pub struct AllowAll;

impl Permissions for AllowAll {
    async fn check(&self, _: &CallerContext, _: Permission) -> DomainResult<()> {
        Ok(())
    }
}

/// Refuses each permission to each caller.
#[derive(Default)]
pub struct DenyAll;

impl Permissions for DenyAll {
    async fn check(&self, _: &CallerContext, permission: Permission) -> DomainResult<()> {
        Err(DomainError::forbidden(format!(
            "{} is not granted",
            permission.key()
        )))
    }
}

#[strata::edition(extends = ScyllaCeOnDb, overrides(scylla_strata::Access))]
pub struct OpenCe;

#[strata::overrides]
impl scylla_strata::Access for OpenCe {
    type Permissions = AllowAll;
}

#[strata::edition(extends = ScyllaCeOnDb, overrides(scylla_strata::Access))]
pub struct ClosedCe;

#[strata::overrides]
impl scylla_strata::Access for ClosedCe {
    type Permissions = DenyAll;
}

/// Counts the project creates of each organization and refuses one over two, as a quota does.
#[derive(Default)]
pub struct Quota(Mutex<HashMap<OrganizationId, usize>>);

#[strata::module(section = "quota")]
pub trait ProjectQuota: scylla_strata::Projects {
    #[component]
    type Counter = Quota;

    #[before(scylla_strata::Projects::create_project)]
    async fn quota_before_create(
        &self,
        organization_id: &OrganizationId,
        _: &ProjectName,
        _: &Option<ProjectDescription>,
    ) -> DomainResult<()> {
        let mut seen = self.counter().0.lock().unwrap();
        let count = seen.entry(organization_id.clone()).or_insert(0);
        if *count >= 2 {
            return Err(DomainError::quota_exceeded(format!(
                "project quota reached for this organization ({count}/2)"
            )));
        }
        *count += 1;
        Ok(())
    }
}

#[strata::edition(extends = OpenCe, modules(ProjectQuota))]
pub struct QuotaCe;

/// The members of the organizations, on the database of the server.
pub struct Members(PgOrganizationRepository);

impl Component for Members {
    async fn build(cx: &mut BuildCx<'_>) -> Result<Self, BuildError> {
        let repos = cx.resource::<ScyllaRepos>().await?;
        Ok(Members(PgOrganizationRepository::new(repos.db.clone())))
    }
}

/// A seat limit: a new member of an organization needs a free seat; a member gets more roles.
#[strata::module(section = "seats")]
pub trait SeatLimit: scylla_strata::Grants {
    #[component]
    type Members = Members;

    #[before(scylla_strata::Grants::create_grant)]
    async fn seat_before_grant(
        &self,
        principal: &Principal,
        _: &RoleName,
        scope: &Scope,
    ) -> DomainResult<()> {
        let (Principal::User(user), Scope::Organization(organization)) = (principal, scope) else {
            return Ok(());
        };
        let members = &self.members().0;
        if members.has_member(organization, user).await? {
            return Ok(());
        }
        if members.count_members(organization).await? >= 2 {
            return Err(DomainError::quota_exceeded("no free seat"));
        }
        Ok(())
    }
}

#[strata::edition(extends = OpenCe, modules(SeatLimit))]
pub struct SeatCe;

/// Keeps each reset message.
#[derive(Default)]
struct Outbox(Mutex<Vec<PasswordResetMessage>>);

#[async_trait]
impl PasswordResetSender for Outbox {
    fn delivery(&self) -> PasswordResetDelivery {
        PasswordResetDelivery::ServerLog
    }

    async fn send(&self, message: &PasswordResetMessage) -> DomainResult<()> {
        self.0.lock().unwrap().push(message.clone());
        Ok(())
    }
}

impl Outbox {
    async fn token(&self) -> ResetToken {
        for _ in 0..500 {
            if let Some(message) = self.0.lock().unwrap().first() {
                let token = message.link.rsplit_once("#token=").unwrap().1;
                return ResetToken::new(token).unwrap();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the link was not delivered");
    }
}

async fn edition_with<E>(pool: &PgPool, sender: Arc<dyn PasswordResetSender>) -> Arc<E>
where
    E: Edition
        + Access
        + Projects
        + Secrets
        + Pipelines
        + Apps
        + Roles
        + Grants
        + Organizations
        + Users
        + Auth
        + Triggers
        + JobControl
        + Agents
        + Send
        + Sync
        + 'static,
{
    let shared = scylla_server::strata_shared(&ControlPlaneConfig::default(), pool.clone(), sender)
        .await
        .unwrap();
    EditionHost::<E>::new(Slice::ALL)
        .edition(&shared, false)
        .await
        .unwrap()
}

async fn edition<E>(pool: &PgPool) -> Arc<E>
where
    E: Edition
        + Access
        + Projects
        + Secrets
        + Pipelines
        + Apps
        + Roles
        + Grants
        + Organizations
        + Users
        + Auth
        + Triggers
        + JobControl
        + Agents
        + Send
        + Sync
        + 'static,
{
    edition_with::<E>(pool, Arc::new(LogPasswordResetSender)).await
}

fn ctx(caller: CallerContext) -> ScyllaCtx {
    ScyllaCtx {
        caller,
        ..ScyllaCtx::default()
    }
}

fn anonymous() -> ScyllaCtx {
    ctx(CallerContext::Anonymous)
}

fn role(name: &str) -> RoleName {
    RoleName::new(name).unwrap()
}

async fn seed_user(pool: &PgPool, name: &str, password: Option<&str>) -> User {
    let hash = match password {
        Some(password) => Argon2HashService::new()
            .hash(&Password::new(password).unwrap())
            .await
            .unwrap(),
        None => scylla_core::domain::user::PasswordHash::new("$argon2id$stub").unwrap(),
    };
    let user = User::create(
        Username::new(name).unwrap(),
        Some(Email::new(format!("{name}@example.com")).unwrap()),
        hash,
    );
    PgUserRepository::new(pool.clone())
        .create(&user)
        .await
        .unwrap();
    user
}

async fn seed_org(pool: &PgPool, name: &str) -> OrganizationId {
    let organization = Organization::create(OrganizationName::new(name).unwrap(), None).unwrap();
    PgOrganizationRepository::new(pool.clone())
        .create(&organization)
        .await
        .unwrap();
    organization.id().clone()
}

async fn grant_on(pool: &PgPool, user: &User, name: &str, scope: Scope) {
    PgGrantRepository::new(pool.clone())
        .create(&Grant::new(
            Principal::User(user.id().clone()),
            role(name),
            scope,
        ))
        .await
        .unwrap();
}

fn password(value: &str) -> Password {
    Password::new(value).unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_login_takes_the_email_or_the_username(pool: PgPool) {
    seed_user(&pool, "kevin", Some("SecurePass123!")).await;
    let app = edition::<ScyllaCeOnDb>(&pool).await;
    let anonymous = anonymous();
    let auth = app.scoped(&anonymous);

    AuthRpc::login(
        &auth,
        "kevin@example.com".to_string(),
        password("SecurePass123!"),
    )
    .await
    .expect("login by email");
    AuthRpc::login(&auth, "kevin".to_string(), password("SecurePass123!"))
        .await
        .expect("login by username");
    let err = AuthRpc::login(&auth, "kevin".to_string(), password("WrongPass123!"))
        .await
        .expect_err("wrong password rejected");
    assert!(matches!(err, DomainError::Unauthorized(_)), "{err}");
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_reset_link_changes_the_password_and_ends_every_session(pool: PgPool) {
    seed_user(&pool, "kevin", Some("OldPass123!")).await;
    let outbox = Arc::new(Outbox::default());
    let app = edition_with::<ScyllaCeOnDb>(&pool, outbox.clone()).await;
    let anonymous = anonymous();
    let auth = app.scoped(&anonymous);
    let sessions = PgSessionRepository::new(pool.clone());
    let old_session = AuthRpc::login(&auth, "kevin".to_string(), password("OldPass123!"))
        .await
        .unwrap();

    let delivery = AuthRpc::request_password_reset(&auth, Email::new("kevin@example.com").unwrap())
        .await
        .unwrap();
    let token = outbox.token().await;
    AuthRpc::reset_password(&auth, token.clone(), Password::new("NewPass123!").unwrap())
        .await
        .unwrap();

    assert_eq!(delivery, PasswordResetDelivery::ServerLog);
    assert!(sessions.find_by_token(old_session.token()).await.is_err());
    assert!(
        AuthRpc::login(&auth, "kevin".to_string(), password("OldPass123!"))
            .await
            .is_err()
    );
    AuthRpc::login(&auth, "kevin".to_string(), password("NewPass123!"))
        .await
        .unwrap();
    let again = AuthRpc::reset_password(&auth, token, Password::new("ThirdPass123!").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(&again, DomainError::BusinessRule(m) if m == RESET_LINK_INVALID));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_user_sets_its_own_display_name_but_not_its_own_email(pool: PgPool) {
    let kevin = User::create(
        Username::new("kevin").unwrap(),
        None,
        scylla_core::domain::user::PasswordHash::new("$argon2id$stub").unwrap(),
    );
    let users = PgUserRepository::new(pool.clone());
    users.create(&kevin).await.unwrap();
    let app = edition::<ScyllaCeOnDb>(&pool).await;
    let caller = ctx(CallerContext::User(kevin.id().clone()));
    let own = app.scoped(&caller);
    let display_name = |d: &str| Some(DisplayName::new(d).unwrap());
    let id = kevin.id().clone();

    let named = UsersRpc::update_user(&own, id.clone(), None, Some(display_name("Kev")), None)
        .await
        .unwrap();
    let err = UsersRpc::update_user(
        &own,
        id,
        None,
        None,
        Some(Email::new("kev@example.com").unwrap()),
    )
    .await
    .unwrap_err();
    let lena = seed_user(&pool, "lena", None).await;
    let other = UsersRpc::update_user(
        &own,
        lena.id().clone(),
        None,
        Some(Some(DisplayName::new("Not Lena").unwrap())),
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(named.display_name().map(DisplayName::as_str), Some("Kev"));
    assert!(matches!(err, DomainError::Forbidden(_)), "{err}");
    assert!(matches!(other, DomainError::Forbidden(_)), "{other}");
    assert_eq!(
        users
            .find_by_id(lena.id())
            .await
            .unwrap()
            .display_name()
            .map(DisplayName::as_str),
        None
    );
    assert!(
        users
            .find_by_id(kevin.id())
            .await
            .unwrap()
            .email()
            .is_none()
    );
}

async fn seed_app(pool: &PgPool, secret: &AppSecret) -> App {
    let org = seed_org(pool, "Acme").await;
    let hash = Argon2HashService::new().hash_secret(secret).await.unwrap();
    let app = App::create(org, AppName::new("ci").unwrap()).unwrap();
    let credential = AppCredential::create(
        app.id().clone(),
        AppSecretLabel::new("default").unwrap(),
        hash,
    );
    PgAppRepository::new(pool.clone())
        .create_app(&app, &credential)
        .await
        .unwrap();
    app
}

async fn issue(app: &ScyllaCeOnDb, seeded: &App, secret: AppSecret) -> DomainResult<AppToken> {
    let anonymous = anonymous();
    AppsRpc::issue_app_token(&app.scoped(&anonymous), seeded.id().clone(), secret).await
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_app_token_needs_the_right_secret_and_the_store_keeps_its_digest(pool: PgPool) {
    let secret = mint_app_secret();
    let seeded = seed_app(&pool, &secret).await;
    let app = edition::<ScyllaCeOnDb>(&pool).await;
    let wrong = issue(&app, &seeded, mint_app_secret()).await;
    let first = issue(&app, &seeded, secret.clone()).await.unwrap();
    let second = issue(&app, &seeded, secret).await.unwrap();

    assert!(
        matches!(wrong, Err(DomainError::Unauthorized(_))),
        "{wrong:?}"
    );
    let stored: Vec<String> =
        sqlx::query_scalar("SELECT token_hash FROM app_tokens WHERE app_id = $1")
            .bind(seeded.id().as_str())
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(stored.len(), 2);
    let tokens = PgAppTokenRepository::new(pool.clone());
    for issued in [&first, &second] {
        assert!(!stored.iter().any(|digest| digest == issued.token()));
        let found = tokens.find_by_token(issued.token()).await.unwrap();
        assert_eq!(found.id(), issued.id());
        assert_eq!(found.app_id(), seeded.id());
        assert!(!found.is_expired());
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_anonymous_caller_gets_an_app_token_when_the_access_model_refuses_all(pool: PgPool) {
    let secret = mint_app_secret();
    let seeded = seed_app(&pool, &secret).await;
    let app = edition::<ClosedCe>(&pool).await;
    let anonymous = anonymous();

    let token =
        AppsRpc::issue_app_token(&app.scoped(&anonymous), seeded.id().clone(), secret).await;

    assert!(
        token.is_ok(),
        "issue_app_token is public: {:?}",
        token.err()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_effective_permissions_join_the_roles_and_the_direct_grants(pool: PgPool) {
    for sql in [
        "INSERT INTO roles (id, name, scope_kind, builtin) VALUES ('ci', 'CI', 'project', FALSE)",
        "INSERT INTO role_permissions (role_id, permission) VALUES ('ci', 'readPipeline')",
        "INSERT INTO role_permissions (role_id, permission) VALUES ('ci', 'runPipeline')",
        "INSERT INTO roles (id, name, scope_kind, builtin) \
         VALUES ('janitor', 'Janitor', 'organization', FALSE)",
        "INSERT INTO role_permissions (role_id, permission) VALUES ('janitor', 'deleteJob')",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    let org = seed_org(&pool, "o1").await;
    let project = Project::create(ProjectName::new("p1").unwrap(), None, org.clone()).unwrap();
    PgProjectRepository::new(pool.clone())
        .create(&project)
        .await
        .unwrap();
    let alice = seed_user(&pool, "alice", None).await;
    grant_on(&pool, &alice, "ci", Scope::Project(project.id().clone())).await;
    grant_on(&pool, &alice, "janitor", Scope::Organization(org.clone())).await;
    let app = edition::<OpenCe>(&pool).await;
    let recorder = ctx(CallerContext::Service(ServiceIdentity::recorder()));

    let scopes = RolesRpc::get_effective_permissions(
        &app.scoped(&recorder),
        Principal::User(alice.id().clone()),
    )
    .await
    .unwrap();

    assert_eq!(scopes.len(), 2);
    let on_project = scopes
        .iter()
        .find(|s| matches!(&s.scope, Scope::Project(p) if p == project.id()))
        .expect("project p1 scope");
    assert!(!on_project.full_control);
    assert!(on_project.permissions.contains(&"readPipeline".to_string()));
    assert!(on_project.permissions.contains(&"runPipeline".to_string()));
    let on_org = scopes
        .iter()
        .find(|s| matches!(&s.scope, Scope::Organization(o) if o == &org))
        .expect("org o1 scope");
    assert_eq!(on_org.permissions, vec!["deleteJob".to_string()]);
}

#[sqlx::test(migrations = "../../migrations")]
async fn my_permissions_need_no_permission_unlike_the_admin_view(pool: PgPool) {
    let org = seed_org(&pool, "o1").await;
    let alice = seed_user(&pool, "alice", None).await;
    grant_on(
        &pool,
        &alice,
        ORGANIZATION_ADMIN_ROLE,
        Scope::Organization(org.clone()),
    )
    .await;
    let app = edition::<ClosedCe>(&pool).await;
    let alice = ctx(CallerContext::User(alice.id().clone()));
    let bob = ctx(CallerContext::User(UserId::new("bob")));
    let recorder = ctx(CallerContext::Service(ServiceIdentity::recorder()));

    let scopes = RolesRpc::get_my_permissions(&app.scoped(&alice))
        .await
        .expect("own permissions");
    assert_eq!(scopes.len(), 1);
    assert!(matches!(&scopes[0].scope, Scope::Organization(o) if o == &org));
    assert!(scopes[0].full_control, "organization-admin confers '*'");

    let scopes = RolesRpc::get_my_permissions(&app.scoped(&bob))
        .await
        .unwrap();
    assert_eq!(scopes, Vec::<EffectiveScope>::new());

    let other = RolesRpc::get_effective_permissions(
        &app.scoped(&alice),
        Principal::User(UserId::new("bob")),
    )
    .await;
    assert!(
        other.is_err(),
        "reading another principal must still require manageSystemGrants"
    );
    let refused = RolesRpc::get_my_permissions(&app.scoped(&recorder)).await;
    assert!(refused.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_hook_before_the_create_vetoes_a_project_over_the_quota(pool: PgPool) {
    let org = seed_org(&pool, "limited").await;
    let app = edition::<QuotaCe>(&pool).await;
    let recorder = ctx(CallerContext::Service(ServiceIdentity::recorder()));
    let name = |name: &str| ProjectName::new(name).unwrap();

    for project in ["a", "b"] {
        ProjectsRpc::create_project(&app.scoped(&recorder), org.clone(), name(project), None)
            .await
            .expect("under quota");
    }
    let err = ProjectsRpc::create_project(&app.scoped(&recorder), org.clone(), name("c"), None)
        .await
        .expect_err("over quota");

    assert!(matches!(err, DomainError::QuotaExceeded(_)), "{err}");
    let listed = PgProjectRepository::new(pool.clone())
        .list_by_organization(&org, None, &scylla_auth::authz::Visibility::All)
        .await
        .unwrap();
    assert_eq!(listed.items().len(), 2);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_hook_before_create_grant_sees_the_grantee_and_the_members(pool: PgPool) {
    let acme = seed_org(&pool, "Acme").await;
    let admin = seed_user(&pool, "admin", None).await;
    let member = seed_user(&pool, "member", None).await;
    let newcomer = seed_user(&pool, "newcomer", None).await;
    let on_acme = || Scope::Organization(acme.clone());
    grant_on(&pool, &admin, ORGANIZATION_ADMIN_ROLE, on_acme()).await;
    grant_on(&pool, &member, ORGANIZATION_MEMBER_ROLE, on_acme()).await;
    let organizations = PgOrganizationRepository::new(pool.clone());
    let app = edition::<SeatCe>(&pool).await;
    let caller = ctx(CallerContext::User(admin.id().clone()));
    let grant =
        |user: &User, name: &str| (Principal::User(user.id().clone()), role(name), on_acme());

    let refused = {
        let (principal, role, scope) = grant(&newcomer, ORGANIZATION_MEMBER_ROLE);
        GrantsRpc::create_grant(&app.scoped(&caller), principal, role, scope).await
    };
    assert!(
        matches!(refused, Err(DomainError::QuotaExceeded(_))),
        "{refused:?}"
    );
    assert!(
        !organizations
            .has_member(&acme, newcomer.id())
            .await
            .unwrap()
    );

    {
        let (principal, role, scope) = grant(&member, ORGANIZATION_VIEWER_ROLE);
        GrantsRpc::create_grant(&app.scoped(&caller), principal, role, scope).await
    }
    .expect("a member gets one more role");
    assert_eq!(organizations.count_members(&acme).await.unwrap(), 2);
    assert!(
        PgGrantRepository::new(pool.clone())
            .list_all()
            .await
            .unwrap()
            .iter()
            .any(|g| g.principal == Principal::User(member.id().clone())
                && g.role.as_str() == ORGANIZATION_VIEWER_ROLE)
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn strata_shared_runs_the_bootstrap_of_the_config(pool: PgPool) {
    let config = ControlPlaneConfig {
        bootstrap: Some(scylla_core::config::BootstrapConfig {
            username: "root".to_string(),
            password: "SecurePass123!".to_string(),
            email: "root@example.com".to_string(),
        }),
        ..ControlPlaneConfig::default()
    };

    scylla_server::strata_shared(&config, pool.clone(), Arc::new(LogPasswordResetSender))
        .await
        .unwrap();

    let root = PgUserRepository::new(pool.clone())
        .find_by_username(&Username::new("root").unwrap())
        .await
        .unwrap();
    assert!(
        PgGrantRepository::new(pool)
            .list_all()
            .await
            .unwrap()
            .iter()
            .any(|g| g.principal == Principal::User(root.id().clone()) && g.scope == Scope::System)
    );
}

/// Keeps each reset message and answers `Mail`.
#[derive(Default)]
struct Mailbox(Mutex<Vec<PasswordResetMessage>>);

#[async_trait]
impl PasswordResetSender for Mailbox {
    fn delivery(&self) -> PasswordResetDelivery {
        PasswordResetDelivery::Mail
    }

    async fn send(&self, message: &PasswordResetMessage) -> DomainResult<()> {
        self.0.lock().unwrap().push(message.clone());
        Ok(())
    }
}

impl Mailbox {
    fn links(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|m| m.link.clone())
            .collect()
    }

    async fn wait_for(&self, count: usize) -> Vec<String> {
        for _ in 0..500 {
            if self.links().len() >= count {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                return self.links();
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("{count} reset links were not delivered");
    }
}

const PUBLIC_URL: &str = "https://scylla.example.com";

fn served_config() -> ControlPlaneConfig {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut config = ControlPlaneConfig {
        bootstrap: Some(scylla_core::config::BootstrapConfig {
            username: "root".to_string(),
            password: "SecurePass123!".to_string(),
            email: "root@example.com".to_string(),
        }),
        ..ControlPlaneConfig::default()
    };
    config.server.address = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    config.server.public_url = Some(PUBLIC_URL.to_string());
    config
}

/// Runs `body` while `Server::serve` runs on the address of `config`. `body` starts when the
/// server listens. The server stops when `body` ends.
async fn served<T>(
    pool: &PgPool,
    config: ControlPlaneConfig,
    sender: Option<Arc<dyn PasswordResetSender>>,
    body: impl AsyncFnOnce(std::net::SocketAddr) -> T,
) -> T {
    let address = config.server.address;
    let mut server = scylla_server::Server::new(config, pool.clone());
    if let Some(sender) = sender {
        server = server.password_reset_sender(sender);
    }
    let calls = async {
        let mut listening = false;
        for _ in 0..500 {
            if tokio::net::TcpStream::connect(address).await.is_ok() {
                listening = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(listening, "the server did not listen on {address}");
        body(address).await
    };
    tokio::select! {
        stopped = server.serve() => panic!("the server stopped: {stopped:?}"),
        answer = calls => answer,
    }
}

async fn reflection_channel(endpoint: &str) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(endpoint.to_string())
        .unwrap()
        .connect()
        .await
        .unwrap()
}

/// One HTTP/1.1 exchange on a new connection. The answer is the raw response.
async fn http1(address: std::net::SocketAddr, head: &str, body: &[u8]) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "{head}\r\nhost: {address}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

/// A gRPC-Web call over HTTP/1.1. The answer is the raw response in lowercase, without the space
/// after a colon, and with `%20` as a space.
async fn grpc_web(address: std::net::SocketAddr, path: &str, message: &[u8]) -> String {
    let mut frame = vec![0u8];
    frame.extend_from_slice(&u32::try_from(message.len()).unwrap().to_be_bytes());
    frame.extend_from_slice(message);
    let head = format!(
        "POST {path} HTTP/1.1\r\ncontent-type: application/grpc-web+proto\r\nx-grpc-web: 1"
    );
    http1(address, &head, &frame)
        .await
        .replace(": ", ":")
        .replace("%20", " ")
        .to_lowercase()
}

fn proto_string(field: u8, value: &str) -> Vec<u8> {
    let mut bytes = vec![(field << 3) | 2, u8::try_from(value.len()).unwrap()];
    bytes.extend_from_slice(value.as_bytes());
    bytes
}

async fn reflected_v1(endpoint: &str) -> Vec<String> {
    use tonic_reflection::pb::v1::ServerReflectionRequest;
    use tonic_reflection::pb::v1::server_reflection_client::ServerReflectionClient;
    use tonic_reflection::pb::v1::server_reflection_request::MessageRequest;
    use tonic_reflection::pb::v1::server_reflection_response::MessageResponse;
    let mut client = ServerReflectionClient::new(reflection_channel(endpoint).await);
    let request = ServerReflectionRequest {
        host: String::new(),
        message_request: Some(MessageRequest::ListServices(String::new())),
    };
    let mut answers = client
        .server_reflection_info(tonic::codegen::tokio_stream::once(request))
        .await
        .unwrap()
        .into_inner();
    let answer = answers.message().await.unwrap().unwrap();
    let Some(MessageResponse::ListServicesResponse(list)) = answer.message_response else {
        panic!("the v1 reflection gave no service list");
    };
    list.service.into_iter().map(|s| s.name).collect()
}

async fn reflected_v1alpha(endpoint: &str) -> Vec<String> {
    use tonic_reflection::pb::v1alpha::ServerReflectionRequest;
    use tonic_reflection::pb::v1alpha::server_reflection_client::ServerReflectionClient;
    use tonic_reflection::pb::v1alpha::server_reflection_request::MessageRequest;
    use tonic_reflection::pb::v1alpha::server_reflection_response::MessageResponse;
    let mut client = ServerReflectionClient::new(reflection_channel(endpoint).await);
    let request = ServerReflectionRequest {
        host: String::new(),
        message_request: Some(MessageRequest::ListServices(String::new())),
    };
    let mut answers = client
        .server_reflection_info(tonic::codegen::tokio_stream::once(request))
        .await
        .unwrap()
        .into_inner();
    let answer = answers.message().await.unwrap().unwrap();
    let Some(MessageResponse::ListServicesResponse(list)) = answer.message_response else {
        panic!("the v1alpha reflection gave no service list");
    };
    list.service.into_iter().map(|s| s.name).collect()
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_served_router_answers_health_grpc_web_and_reflection(pool: PgPool) {
    let (health, logged_in, anonymous, v1, v1alpha) =
        served(&pool, served_config(), None, async |address| {
            let health = http1(address, "GET /healthz HTTP/1.1", &[]).await;
            let login = [proto_string(1, "root"), proto_string(2, "SecurePass123!")].concat();
            let logged_in = grpc_web(address, "/scylla.auth.v1.AuthService/Login", &login).await;
            let anonymous = grpc_web(address, "/scylla.user.v1.UserService/GetMe", &[]).await;
            let endpoint = format!("http://{address}");
            let v1 = reflected_v1(&endpoint).await;
            let v1alpha = reflected_v1alpha(&endpoint).await;
            (health, logged_in, anonymous, v1, v1alpha)
        })
        .await;

    assert!(health.starts_with("HTTP/1.1 200"), "{health}");
    assert!(health.ends_with("\r\n\r\nok"), "{health}");
    assert!(logged_in.starts_with("http/1.1 200"), "{logged_in}");
    assert!(logged_in.contains("grpc-status:0\r\n"), "{logged_in}");
    assert!(anonymous.contains("grpc-status:16\r\n"), "{anonymous}");
    assert!(
        anonymous.contains("grpc-message:missing or invalid authorization token\r\n"),
        "{anonymous}"
    );
    assert_eq!(scylla_strata::Service::ALL.len(), 14);
    for (listed, reflection) in [
        (&v1, "grpc.reflection.v1.ServerReflection"),
        (&v1alpha, "grpc.reflection.v1alpha.ServerReflection"),
    ] {
        for service in scylla_strata::Service::ALL {
            assert!(
                listed.iter().any(|s| s == service.name()),
                "{service:?}: {listed:?}"
            );
        }
        assert!(listed.iter().any(|s| s == reflection), "{listed:?}");
        assert_eq!(listed.len(), 15, "{listed:?}");
    }
}

/// `RequestPasswordReset` for kevin, then `SendPasswordReset` for lena as the bootstrap admin.
async fn reset_answers(address: std::net::SocketAddr, lena: &User) -> (i32, i32) {
    use scylla_proto::auth::v1::auth_service_client::AuthServiceClient;
    use scylla_proto::auth::v1::{LoginRequest, RequestPasswordResetRequest};
    use scylla_proto::user::v1::SendPasswordResetRequest;
    use scylla_proto::user::v1::user_service_client::UserServiceClient;
    let endpoint = format!("http://{address}");
    let mut auth = AuthServiceClient::connect(endpoint.clone()).await.unwrap();
    let requested = auth
        .request_password_reset(RequestPasswordResetRequest {
            email: Some(scylla_proto::common::v1::Email {
                value: "kevin@example.com".to_string(),
            }),
        })
        .await
        .unwrap()
        .into_inner()
        .delivery;
    let token = auth
        .login(LoginRequest {
            identifier: "root".to_string(),
            password: "SecurePass123!".to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .token;
    let mut users = UserServiceClient::connect(endpoint).await.unwrap();
    let mut send = tonic::Request::new(SendPasswordResetRequest {
        user_id: Some(scylla_proto::common::v1::UserId {
            value: lena.id().to_string(),
        }),
    });
    send.metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let sent = users
        .send_password_reset(send)
        .await
        .unwrap()
        .into_inner()
        .delivery;
    (requested, sent)
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_reset_sender_of_the_server_answers_both_reset_calls(pool: PgPool) {
    use scylla_proto::auth::v1::PasswordResetDelivery as Wire;
    seed_user(&pool, "kevin", Some("OldPass123!")).await;
    let lena = seed_user(&pool, "lena", Some("OldPass123!")).await;
    let mailbox = Arc::new(Mailbox::default());

    let sender: Arc<dyn PasswordResetSender> = mailbox.clone();
    let (mailed, links) = served(&pool, served_config(), Some(sender), async |address| {
        let answers = reset_answers(address, &lena).await;
        (answers, mailbox.wait_for(2).await)
    })
    .await;
    let logged = served(&pool, served_config(), None, async |address| {
        reset_answers(address, &lena).await
    })
    .await;
    let roots: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE username = 'root'")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(mailed, (Wire::Mail as i32, Wire::Mail as i32));
    assert_eq!(links.len(), 2, "{links:?}");
    assert!(
        links
            .iter()
            .all(|link| link.starts_with(&format!("{PUBLIC_URL}/"))),
        "{links:?}"
    );
    assert_eq!(logged, (Wire::ServerLog as i32, Wire::ServerLog as i32));
    assert_eq!(mailbox.links().len(), 2);
    assert_eq!(
        roots, 1,
        "a second start on the same database creates no second admin"
    );
}

/// Keeps each line that the subscriber writes.
#[derive(Clone, Default)]
struct LogLines(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogLines {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn bearer<T>(message: T, token: &str) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    request
}

/// The credentials that the calls carry, as the server sees them over gRPC.
struct Credentials {
    password: String,
    session_tokens: Vec<String>,
    app_secret: String,
    reset_token: String,
    secret_value: String,
}

async fn calls_with_credentials(address: std::net::SocketAddr, mailbox: &Mailbox) -> Credentials {
    use scylla_proto::app::v1::app_auth_service_client::AppAuthServiceClient;
    use scylla_proto::app::v1::app_service_client::AppServiceClient;
    use scylla_proto::app::v1::{CreateAppRequest, IssueTokenRequest};
    use scylla_proto::auth::v1::auth_service_client::AuthServiceClient;
    use scylla_proto::auth::v1::{
        LoginRequest, RequestPasswordResetRequest, ResetPasswordRequest, RevokeTokenRequest,
        ValidateTokenRequest,
    };
    use scylla_proto::organization::v1::CreateOrganizationRequest;
    use scylla_proto::organization::v1::organization_service_client::OrganizationServiceClient;
    use scylla_proto::project::v1::CreateProjectRequest;
    use scylla_proto::project::v1::project_service_client::ProjectServiceClient;
    use scylla_proto::secret::v1::CreateSecretRequest;
    use scylla_proto::secret::v1::secret_service_client::SecretServiceClient;
    let endpoint = format!("http://{address}");
    let password = "SecurePass123!".to_string();
    let mut auth = AuthServiceClient::connect(endpoint.clone()).await.unwrap();
    let login = || LoginRequest {
        identifier: "root".to_string(),
        password: password.clone(),
    };
    let checked = auth.login(login()).await.unwrap().into_inner().token;
    let admin = auth.login(login()).await.unwrap().into_inner().token;
    auth.validate_token(ValidateTokenRequest {
        token: checked.clone(),
    })
    .await
    .unwrap();
    auth.revoke_token(RevokeTokenRequest {
        token: checked.clone(),
    })
    .await
    .unwrap();

    let organization = OrganizationServiceClient::connect(endpoint.clone())
        .await
        .unwrap()
        .create_organization(bearer(
            CreateOrganizationRequest {
                name: "acme".to_string(),
                description: None,
            },
            &admin,
        ))
        .await
        .unwrap()
        .into_inner()
        .organization
        .unwrap()
        .organization_id;
    let project = ProjectServiceClient::connect(endpoint.clone())
        .await
        .unwrap()
        .create_project(bearer(
            CreateProjectRequest {
                organization_id: organization.clone(),
                name: "web".to_string(),
                description: None,
            },
            &admin,
        ))
        .await
        .unwrap()
        .into_inner()
        .project
        .unwrap()
        .project_id;
    let secret_value = "the-value-of-the-secret-0123456789".to_string();
    SecretServiceClient::connect(endpoint.clone())
        .await
        .unwrap()
        .create_secret(bearer(
            CreateSecretRequest {
                project_id: project,
                name: "DB_PASSWORD".to_string(),
                value: secret_value.clone(),
                description: String::new(),
            },
            &admin,
        ))
        .await
        .unwrap();

    let created = AppServiceClient::connect(endpoint.clone())
        .await
        .unwrap()
        .create_app(bearer(
            CreateAppRequest {
                organization_id: organization,
                name: "ci-bot".to_string(),
                secret_label: None,
            },
            &admin,
        ))
        .await
        .unwrap()
        .into_inner();
    let app_secret = created.secret;
    assert!(app_secret.len() >= 32);
    AppAuthServiceClient::connect(endpoint.clone())
        .await
        .unwrap()
        .issue_token(IssueTokenRequest {
            app_id: created.app.unwrap().app_id,
            secret: app_secret.clone(),
        })
        .await
        .unwrap();

    auth.request_password_reset(RequestPasswordResetRequest {
        email: Some(scylla_proto::common::v1::Email {
            value: "root@example.com".to_string(),
        }),
    })
    .await
    .unwrap();
    let link = mailbox.wait_for(1).await.remove(0);
    let reset_token = link.rsplit_once("#token=").unwrap().1.to_string();
    auth.reset_password(ResetPasswordRequest {
        token: reset_token.clone(),
        new_password: "OtherPass123!".to_string(),
    })
    .await
    .unwrap();

    Credentials {
        password,
        session_tokens: vec![checked, admin],
        app_secret,
        reset_token,
        secret_value,
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn no_credential_of_a_call_reaches_the_log(pool: PgPool) {
    let lines = LogLines::default();
    let writer = lines.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_span_events(tracing_subscriber::fmt::format::FmtSpan::FULL)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let mailbox = Arc::new(Mailbox::default());
    let sender: Arc<dyn PasswordResetSender> = mailbox.clone();

    let mut config = served_config();
    config.override_master_key(scylla_core::config::DEV_MASTER_KEY);
    let credentials = served(&pool, config, Some(sender), async |address| {
        calls_with_credentials(address, &mailbox).await
    })
    .await;

    let log = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
    assert!(
        log.contains("grpc"),
        "the spans of the calls are in the log"
    );
    let mut secrets = credentials.session_tokens.clone();
    secrets.extend([
        credentials.password,
        credentials.app_secret,
        credentials.reset_token,
        credentials.secret_value,
    ]);
    for secret in secrets {
        assert!(
            !log.contains(&secret),
            "a credential is in the log: {secret}"
        );
    }
}
