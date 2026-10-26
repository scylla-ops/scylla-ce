//! The CE edition on Postgres, built as an out-of-tree edition builds it: `strata_shared`, then
//! `EditionHost::edition`. These flows ran on the Actions engine before every slice ran on strata.

#![allow(clippy::unused_async_trait_impl)]

use async_trait::async_trait;
use scylla_auth::authz::{
    Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, ORGANIZATION_MEMBER_ROLE,
    ORGANIZATION_VIEWER_ROLE, Principal, Scope,
};
use scylla_core::application::app::{IssueAppToken, mint_app_secret};
use scylla_core::application::auth::Login;
use scylla_core::application::grant::CreateGrant;
use scylla_core::application::project::CreateProject;
use scylla_core::application::role::{GetEffectivePermissions, GetMyPermissions};
use scylla_core::application::user::UpdateUser;
use scylla_core::application::user::reset::{RequestPasswordReset, ResetPassword};
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
use scylla_core::domain::project::{Project, ProjectName};
use scylla_core::domain::role::RoleName;
use scylla_core::domain::session::SessionClient;
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
    async fn quota_before_create(&self, cmd: &CreateProject) -> DomainResult<()> {
        let mut seen = self.counter().0.lock().unwrap();
        let count = seen.entry(cmd.organization_id.clone()).or_insert(0);
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
    async fn seat_before_grant(&self, cmd: &CreateGrant) -> DomainResult<()> {
        let (Principal::User(user), Scope::Organization(organization)) =
            (&cmd.principal, &cmd.scope)
        else {
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

fn login(identifier: &str, password: &str) -> Login {
    Login {
        identifier: identifier.to_string(),
        password: Password::new(password).unwrap(),
        client: SessionClient::default(),
    }
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_login_takes_the_email_or_the_username(pool: PgPool) {
    seed_user(&pool, "kevin", Some("SecurePass123!")).await;
    let app = edition::<ScyllaCeOnDb>(&pool).await;
    let anonymous = anonymous();
    let auth = app.scoped(&anonymous);

    AuthRpc::login(&auth, login("kevin@example.com", "SecurePass123!"))
        .await
        .expect("login by email");
    AuthRpc::login(&auth, login("kevin", "SecurePass123!"))
        .await
        .expect("login by username");
    let err = AuthRpc::login(&auth, login("kevin", "WrongPass123!"))
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
    let old_session = AuthRpc::login(&auth, login("kevin", "OldPass123!"))
        .await
        .unwrap();

    let delivery = AuthRpc::request_password_reset(
        &auth,
        RequestPasswordReset {
            email: Email::new("kevin@example.com").unwrap(),
        },
    )
    .await
    .unwrap();
    let token = outbox.token().await;
    AuthRpc::reset_password(
        &auth,
        ResetPassword {
            token: token.clone(),
            new_password: Password::new("NewPass123!").unwrap(),
        },
    )
    .await
    .unwrap();

    assert_eq!(delivery, PasswordResetDelivery::ServerLog);
    assert!(sessions.find_by_token(old_session.token()).await.is_err());
    assert!(
        AuthRpc::login(&auth, login("kevin", "OldPass123!"))
            .await
            .is_err()
    );
    AuthRpc::login(&auth, login("kevin", "NewPass123!"))
        .await
        .unwrap();
    let again = AuthRpc::reset_password(
        &auth,
        ResetPassword {
            token,
            new_password: Password::new("ThirdPass123!").unwrap(),
        },
    )
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
    let update = |display_name: Option<&str>, email: Option<&str>| UpdateUser {
        id: kevin.id().clone(),
        username: None,
        display_name: display_name.map(|d| Some(DisplayName::new(d).unwrap())),
        email: email.map(|e| Email::new(e).unwrap()),
    };

    let named = UsersRpc::update_user(&own, update(Some("Kev"), None))
        .await
        .unwrap();
    let err = UsersRpc::update_user(&own, update(None, Some("kev@example.com")))
        .await
        .unwrap_err();

    assert_eq!(named.display_name().map(DisplayName::as_str), Some("Kev"));
    assert!(matches!(err, DomainError::Forbidden(_)), "{err}");
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
    AppsRpc::issue_app_token(
        &app.scoped(&anonymous),
        IssueAppToken {
            app_id: seeded.id().clone(),
            secret,
        },
    )
    .await
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
        GetEffectivePermissions {
            principal: Principal::User(alice.id().clone()),
        },
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

    let scopes = RolesRpc::get_my_permissions(&app.scoped(&alice), GetMyPermissions)
        .await
        .expect("own permissions");
    assert_eq!(scopes.len(), 1);
    assert!(matches!(&scopes[0].scope, Scope::Organization(o) if o == &org));
    assert!(scopes[0].full_control, "organization-admin confers '*'");

    let scopes = RolesRpc::get_my_permissions(&app.scoped(&bob), GetMyPermissions)
        .await
        .unwrap();
    assert!(scopes.is_empty());

    let other = RolesRpc::get_effective_permissions(
        &app.scoped(&alice),
        GetEffectivePermissions {
            principal: Principal::User(UserId::new("bob")),
        },
    )
    .await;
    assert!(
        other.is_err(),
        "reading another principal must still require manageSystemGrants"
    );
    let refused = RolesRpc::get_my_permissions(&app.scoped(&recorder), GetMyPermissions).await;
    assert!(refused.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_hook_before_the_create_vetoes_a_project_over_the_quota(pool: PgPool) {
    let org = seed_org(&pool, "limited").await;
    let app = edition::<QuotaCe>(&pool).await;
    let recorder = ctx(CallerContext::Service(ServiceIdentity::recorder()));
    let create = |name: &str| CreateProject {
        organization_id: org.clone(),
        name: ProjectName::new(name).unwrap(),
        description: None,
    };

    for name in ["a", "b"] {
        ProjectsRpc::create_project(&app.scoped(&recorder), create(name))
            .await
            .expect("under quota");
    }
    let err = ProjectsRpc::create_project(&app.scoped(&recorder), create("c"))
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
    let grant = |user: &User, name: &str| CreateGrant {
        principal: Principal::User(user.id().clone()),
        role: role(name),
        scope: on_acme(),
    };

    let refused = GrantsRpc::create_grant(
        &app.scoped(&caller),
        grant(&newcomer, ORGANIZATION_MEMBER_ROLE),
    )
    .await;
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

    GrantsRpc::create_grant(
        &app.scoped(&caller),
        grant(&member, ORGANIZATION_VIEWER_ROLE),
    )
    .await
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
