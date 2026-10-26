//! Tests the Auth module of Scylla, written with strata: the CE application tests of the session
//! actions and of the public reset links on stub ports, and the gRPC adapter over a real tonic
//! server.

use async_trait::async_trait;
use scylla_auth::authz::PermissionService;
use scylla_core::application::auth::{
    Login, PurgeExpiredSessions, RevokeToken, ValidateToken, record_activity,
};
use scylla_core::application::user::reset::{
    RESET_LINK_TTL, RequestPasswordReset, ResetLinks, ResetPassword, issue_and_deliver,
    mint_reset_token,
};
use scylla_core::application::user::{
    AccountRepository, PasswordResetDelivery, PasswordResetMessage, PasswordResetSender,
    UserRepository, UserSession,
};
use scylla_core::application::{HashService, PrincipalAccess, SessionRepository};
use scylla_core::test_support::authz::{DenyingPermissionService, RecordingPermissionService};
use scylla_core::test_support::sessions::SessionBuilder;
use scylla_core::test_support::stubs::{
    ScopesByAgent, StubAccounts, StubHash, StubSessions, StubUsers, plain_hash,
};
use scylla_core::test_support::users::UserBuilder;
use scylla_domain::domain::caller::{CallerContext, ServiceIdentity};
use scylla_domain::domain::clock;
use scylla_domain::domain::errors::{DomainError, DomainResult};
use scylla_domain::domain::ids::{SessionId, UserId};
use scylla_domain::domain::session::{ACTIVITY_INTERVAL, Session, SessionClient};
use scylla_domain::domain::user::{
    DisplayName, Email, Password, PasswordReset, RESET_LINK_INVALID, ResetToken, User,
};
use scylla_proto::auth::v1::LoginRequest;
use scylla_strata::grpc::{AuthServiceClient, AuthServiceServer};
use scylla_strata::{AccessModel, Auth, AuthRpc, ResetLinkBase, ScyllaCtx};
use sqlx::types::chrono::{DateTime, Utc};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;
use strata::{Contextual, Edition};
use strata_tonic::Grpc;
use tokio::sync::Semaphore;
use tonic::transport::server::TcpIncoming;
use tonic::transport::{Channel, Server};

type Duration = <DateTime<Utc> as std::ops::Sub>::Output;

const PASSWORD: &str = "SecurePass123!";
const NEW_PASSWORD: &str = "NewPass123!";
const PUBLIC_URL: &str = "https://scylla.example.com/";

#[strata::edition(modules(scylla_strata::Access, scylla_strata::Auth))]
pub struct TestBase;

#[strata::edition(extends = TestBase, overrides(scylla_strata::Access))]
pub struct TestCe;

#[strata::overrides]
impl scylla_strata::Access for TestCe {
    type Permissions = scylla_strata::CedarPermissions;
}

/// Records each message. A held sender waits for `release` before it records.
#[derive(Default)]
struct Outbox {
    sent: Mutex<Vec<PasswordResetMessage>>,
    gate: Option<Semaphore>,
    fails: bool,
}

impl Outbox {
    fn held() -> Self {
        Self {
            gate: Some(Semaphore::new(0)),
            ..Self::default()
        }
    }

    fn failing() -> Self {
        Self {
            fails: true,
            ..Self::default()
        }
    }

    fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.add_permits(1);
        }
    }

    fn sent(&self) -> Vec<PasswordResetMessage> {
        self.sent.lock().unwrap().clone()
    }

    async fn wait_for(&self, count: usize) {
        tokio::time::timeout(StdDuration::from_secs(5), async {
            while self.sent.lock().unwrap().len() < count {
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        })
        .await
        .expect("the link was not delivered");
    }
}

#[async_trait]
impl PasswordResetSender for Outbox {
    fn delivery(&self) -> PasswordResetDelivery {
        PasswordResetDelivery::Mail
    }

    async fn send(&self, message: &PasswordResetMessage) -> DomainResult<()> {
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        if self.fails {
            return Err(DomainError::infrastructure("the mail server is down"));
        }
        self.sent.lock().unwrap().push(message.clone());
        Ok(())
    }
}

/// The `StubAccounts` of a lab. `issue_reset` waits for `release` when the store is held, and
/// fails when the store is broken.
struct Store {
    inner: Arc<StubAccounts>,
    gate: Option<Semaphore>,
    broken: bool,
}

impl Store {
    fn release(&self) {
        if let Some(gate) = &self.gate {
            gate.add_permits(1);
        }
    }
}

#[async_trait]
impl AccountRepository for Store {
    async fn update_signed_out(&self, user: &User, keep: Option<&SessionId>) -> DomainResult<User> {
        self.inner.update_signed_out(user, keep).await
    }
    async fn revoke_sessions(
        &self,
        user_id: &UserId,
        keep: Option<&SessionId>,
    ) -> DomainResult<u64> {
        self.inner.revoke_sessions(user_id, keep).await
    }
    async fn list_sessions(&self, user_id: &UserId) -> DomainResult<Vec<UserSession>> {
        self.inner.list_sessions(user_id).await
    }
    async fn revoke_session(&self, user_id: &UserId, id: &SessionId) -> DomainResult<bool> {
        self.inner.revoke_session(user_id, id).await
    }
    async fn issue_reset(
        &self,
        reset: &PasswordReset,
        cooldown: Option<Duration>,
    ) -> DomainResult<bool> {
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        if self.broken {
            return Err(DomainError::infrastructure("the database is down"));
        }
        self.inner.issue_reset(reset, cooldown).await
    }
    async fn find_reset(&self, token: &ResetToken) -> DomainResult<PasswordReset> {
        self.inner.find_reset(token).await
    }
    async fn redeem_reset(&self, reset: &PasswordReset, user: &User) -> DomainResult<User> {
        self.inner.redeem_reset(reset, user).await
    }
    async fn list_access(&self, user_id: &UserId) -> DomainResult<Vec<PrincipalAccess>> {
        self.inner.list_access(user_id).await
    }
}

struct Lab {
    app: TestCe,
    users: Arc<StubUsers>,
    sessions: Arc<StubSessions>,
    accounts: Arc<StubAccounts>,
    store: Arc<Store>,
    outbox: Arc<Outbox>,
}

fn ctx(caller: CallerContext) -> ScyllaCtx {
    ScyllaCtx {
        caller,
        ..ScyllaCtx::default()
    }
}

fn anonymous() -> ScyllaCtx {
    ScyllaCtx::default()
}

impl Lab {
    async fn login(&self, identifier: &str, password: &str) -> DomainResult<Session> {
        self.login_from(identifier, password, SessionClient::default())
            .await
    }

    async fn login_from(
        &self,
        identifier: &str,
        password: &str,
        client: SessionClient,
    ) -> DomainResult<Session> {
        let login = Login {
            identifier: identifier.to_string(),
            password: Password::new(password).unwrap(),
            client: SessionClient::default(),
        };
        let ctx = ScyllaCtx {
            client: Some(client),
            ..anonymous()
        };
        AuthRpc::login(&self.app.scoped(&ctx), login).await
    }

    async fn validate(&self, token: &str) -> bool {
        let query = ValidateToken {
            token: token.to_string(),
        };
        AuthRpc::validate_token(&self.app.scoped(&anonymous()), query)
            .await
            .unwrap()
    }

    async fn request(&self, email: &str) -> DomainResult<PasswordResetDelivery> {
        let cmd = RequestPasswordReset {
            email: Email::new(email).unwrap(),
        };
        AuthRpc::request_password_reset(&self.app.scoped(&anonymous()), cmd).await
    }

    async fn reset(&self, token: &ResetToken, password: &str) -> DomainResult<()> {
        let cmd = ResetPassword {
            token: token.clone(),
            new_password: Password::new(password).unwrap(),
        };
        AuthRpc::reset_password(&self.app.scoped(&anonymous()), cmd).await
    }

    fn stored(&self, id: &UserId) -> User {
        self.users.rows()[id].clone()
    }
}

struct Setup {
    permissions: Arc<dyn PermissionService>,
    sessions: StubSessions,
    outbox: Outbox,
    gate: Option<Semaphore>,
    broken: bool,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            permissions: Arc::new(DenyingPermissionService::new()),
            sessions: StubSessions::default(),
            outbox: Outbox::default(),
            gate: None,
            broken: false,
        }
    }
}

async fn lab_of(setup: Setup) -> Lab {
    let users = Arc::new(StubUsers::default());
    let accounts = Arc::new(StubAccounts::new(users.clone()));
    let store = Arc::new(Store {
        inner: accounts.clone(),
        gate: setup.gate,
        broken: setup.broken,
    });
    let sessions = Arc::new(setup.sessions);
    let outbox = Arc::new(setup.outbox);
    let config = strata::Config::new();
    let app = TestCe::builder(&config)
        .provide(AccessModel {
            permissions: setup.permissions,
            visibility: Arc::new(ScopesByAgent::everything()),
        })
        .provide_arc::<dyn UserRepository>(users.clone())
        .provide_arc::<dyn SessionRepository>(sessions.clone())
        .provide_arc::<dyn HashService>(Arc::new(StubHash::plain()))
        .provide_arc::<dyn AccountRepository>(store.clone())
        .provide_arc::<dyn PasswordResetSender>(outbox.clone())
        .provide(ResetLinkBase(ResetLinks::new(Some(PUBLIC_URL))))
        .build()
        .await
        .unwrap();
    Lab {
        app,
        users,
        sessions,
        accounts,
        store,
        outbox,
    }
}

async fn session_lab(active: bool, sessions: StubSessions) -> Lab {
    let lab = lab_of(Setup {
        sessions,
        ..Setup::default()
    })
    .await;
    lab.users.insert(
        UserBuilder::new("kevin")
            .id(UserId::new("kevin"))
            .email("kevin@example.com")
            .password_hash(plain_hash(PASSWORD).as_str())
            .is_active(active)
            .build(),
    );
    lab
}

#[tokio::test]
async fn a_login_by_username_or_email_stores_a_session_without_asking_a_permission() {
    let lab = session_lab(true, StubSessions::default()).await;

    let by_name = lab.login("kevin", PASSWORD).await.unwrap();
    let by_email = lab.login("kevin@example.com", PASSWORD).await.unwrap();

    assert_eq!(by_name.user_id(), &UserId::new("kevin"));
    assert_ne!(by_name.token(), by_email.token());
    assert_eq!(lab.sessions.rows().len(), 2);
}

#[tokio::test]
async fn a_login_records_the_client_of_the_call_on_the_new_session() {
    let lab = session_lab(true, StubSessions::default()).await;
    let client = SessionClient::new(Some("Firefox/131.0"), Some("203.0.113.7"));

    let session = lab
        .login_from("kevin", PASSWORD, client.clone())
        .await
        .unwrap();

    assert_eq!(session.client(), &client);
    assert_eq!(lab.sessions.rows()[0].client(), &client);
}

#[tokio::test]
async fn a_wrong_password_or_an_unknown_account_gives_one_opaque_error() {
    let lab = session_lab(true, StubSessions::default()).await;

    for (identifier, password) in [
        ("kevin", "WrongPass123!"),
        ("kevin@example.com", "WrongPass123!"),
        ("ghost", PASSWORD),
        ("ghost@example.com", PASSWORD),
    ] {
        let err = lab.login(identifier, password).await.unwrap_err();
        assert!(matches!(&err, DomainError::Unauthorized(m) if m == "Invalid credentials"));
    }
    assert!(lab.sessions.rows().is_empty());
}

#[tokio::test]
async fn an_inactive_account_cannot_log_in() {
    let lab = session_lab(false, StubSessions::default()).await;

    let err = lab.login("kevin", PASSWORD).await.unwrap_err();

    assert!(matches!(&err, DomainError::Unauthorized(m) if m == "User account is inactive"));
    assert!(lab.sessions.rows().is_empty());
}

#[tokio::test]
async fn an_inactive_account_with_a_wrong_password_gives_the_opaque_error() {
    let lab = session_lab(false, StubSessions::default()).await;

    for identifier in ["kevin", "kevin@example.com"] {
        let err = lab.login(identifier, "WrongPass123!").await.unwrap_err();
        assert!(matches!(&err, DomainError::Unauthorized(m) if m == "Invalid credentials"));
    }
    assert!(lab.sessions.rows().is_empty());
}

#[tokio::test]
async fn a_live_token_is_valid_and_a_check_deletes_nothing() {
    let user_id = UserId::new("kevin");
    let live = session_lab(
        true,
        StubSessions::with(SessionBuilder::new(&user_id).token("live").build()),
    )
    .await;
    let expired = session_lab(
        true,
        StubSessions::with(
            SessionBuilder::new(&user_id)
                .token("expired")
                .expired(true)
                .build(),
        ),
    )
    .await;

    assert!(live.validate("live").await);
    assert!(!live.validate("unknown").await);
    assert!(!live.validate("").await);
    assert!(!expired.validate("expired").await);
    assert!(expired.sessions.deleted().is_empty());
    assert_eq!(expired.sessions.rows().len(), 1);
}

#[tokio::test]
async fn a_purge_deletes_the_expired_sessions_and_refuses_a_caller_that_is_not_a_service() {
    let user_id = UserId::new("kevin");
    let lab = session_lab(
        true,
        StubSessions::with(SessionBuilder::new(&user_id).token("live").build()),
    )
    .await;
    lab.sessions
        .create(
            &SessionBuilder::new(&user_id)
                .token("expired")
                .expired(true)
                .build(),
        )
        .await
        .unwrap();
    let service = ctx(CallerContext::Service(ServiceIdentity::session_sweeper()));

    let refused =
        AuthRpc::purge_expired_sessions(&lab.app.scoped(&anonymous()), PurgeExpiredSessions)
            .await
            .unwrap_err();
    let purged = AuthRpc::purge_expired_sessions(&lab.app.scoped(&service), PurgeExpiredSessions)
        .await
        .unwrap();

    assert!(matches!(refused, DomainError::Forbidden(_)));
    assert_eq!(purged, 1);
    let rows = lab.sessions.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].token(), "live");
}

#[tokio::test]
async fn the_sweeper_worker_purges_as_the_sweeper_service() {
    let user_id = UserId::new("kevin");
    let lab = session_lab(
        true,
        StubSessions::with(SessionBuilder::new(&user_id).token("live").build()),
    )
    .await;
    lab.sessions
        .create(
            &SessionBuilder::new(&user_id)
                .token("expired")
                .expired(true)
                .build(),
        )
        .await
        .unwrap();

    assert_eq!(scylla_strata::sweep_sessions(&lab.app).await, 1);
    assert_eq!(scylla_strata::sweep_sessions(&lab.app).await, 0);

    let rows = lab.sessions.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].token(), "live");
}

#[tokio::test]
async fn a_revoke_deletes_the_session() {
    let user_id = UserId::new("kevin");
    let lab = session_lab(
        true,
        StubSessions::with(SessionBuilder::new(&user_id).token("t").build()),
    )
    .await;

    AuthRpc::revoke_token(
        &lab.app.scoped(&anonymous()),
        RevokeToken {
            token: "t".to_string(),
        },
    )
    .await
    .unwrap();

    assert!(lab.sessions.rows().is_empty());
    assert_eq!(lab.sessions.deleted(), vec!["t".to_string()]);
}

#[tokio::test]
async fn an_activity_moves_at_most_once_in_the_interval() {
    let user_id = UserId::new("kevin");
    let now = clock::now();
    let fresh = SessionBuilder::new(&user_id)
        .last_active_at(now - ACTIVITY_INTERVAL + Duration::seconds(30))
        .expires_at(now + Duration::hours(1))
        .build();
    let idle = SessionBuilder::new(&user_id)
        .last_active_at(now - ACTIVITY_INTERVAL - Duration::seconds(1))
        .expires_at(now + Duration::hours(1))
        .build();
    let sessions = StubSessions::with(fresh.clone());
    sessions.create(&idle).await.unwrap();

    record_activity(&sessions, &fresh).await;
    record_activity(&sessions, &idle).await;
    let moved = sessions.find_by_token(idle.token()).await.unwrap();
    record_activity(&sessions, &moved).await;

    assert_eq!(sessions.touched(), vec![idle.id().clone()]);
    assert!(moved.last_active_at() >= now);
    assert_eq!(
        sessions
            .find_by_token(fresh.token())
            .await
            .unwrap()
            .last_active_at(),
        fresh.last_active_at()
    );
}

#[tokio::test]
async fn a_store_that_cannot_record_the_activity_does_not_fail_the_call() {
    struct Silent;

    #[async_trait]
    impl SessionRepository for Silent {
        async fn create(&self, _: &Session) -> DomainResult<Session> {
            unreachable!()
        }
        async fn find_by_token(&self, _: &str) -> DomainResult<Session> {
            unreachable!()
        }
        async fn delete_by_token(&self, _: &str) -> DomainResult<()> {
            unreachable!()
        }
        async fn delete_expired(&self) -> DomainResult<u64> {
            unreachable!()
        }
    }

    let idle = SessionBuilder::new(&UserId::new("kevin"))
        .last_active_at(clock::now() - ACTIVITY_INTERVAL * 2)
        .build();
    let now: DateTime<Utc> = clock::now();

    assert!(Silent.touch(idle.id(), now).await.is_err());
    record_activity(&Silent, &idle).await;
}

async fn reset_lab(outbox: Outbox) -> (Lab, Arc<RecordingPermissionService>) {
    reset_lab_with(outbox, None, false).await
}

async fn reset_lab_with(
    outbox: Outbox,
    gate: Option<Semaphore>,
    broken: bool,
) -> (Lab, Arc<RecordingPermissionService>) {
    let permissions = Arc::new(RecordingPermissionService::new());
    let lab = lab_of(Setup {
        permissions: permissions.clone(),
        outbox,
        gate,
        broken,
        ..Setup::default()
    })
    .await;
    (lab, permissions)
}

fn account(lab: &Lab, id: &str, active: bool) -> User {
    let user = UserBuilder::new(id)
        .id(UserId::new(id))
        .email(format!("{id}@example.com"))
        .display_name("Kevin K.")
        .password_hash(plain_hash(PASSWORD).as_str())
        .is_active(active)
        .build();
    lab.users.insert(user.clone());
    user
}

fn token_of(message: &PasswordResetMessage) -> ResetToken {
    let token = message.link.rsplit_once("#token=").unwrap().1;
    ResetToken::new(token).unwrap()
}

fn aged(reset: &PasswordReset, age: Duration) -> PasswordReset {
    PasswordReset::from_persistence(
        reset.id().clone(),
        reset.token().clone(),
        reset.user_id().clone(),
        reset.created_at() - age,
        reset.expires_at() - age,
        reset.used_at(),
    )
}

fn invalid(result: DomainResult<()>) -> bool {
    matches!(result, Err(DomainError::BusinessRule(m)) if m == RESET_LINK_INVALID)
}

#[tokio::test]
async fn a_request_for_an_active_account_stores_a_link_and_delivers_it() {
    let (lab, permissions) = reset_lab(Outbox::default()).await;
    let kevin = account(&lab, "kevin", true);

    let delivery = lab.request("Kevin@Example.com").await.unwrap();
    lab.outbox.wait_for(1).await;

    assert_eq!(delivery, PasswordResetDelivery::Mail);
    let message = &lab.outbox.sent()[0];
    let stored = lab.accounts.resets(kevin.id());
    assert_eq!(stored.len(), 1);
    assert_eq!(message.user_id, *kevin.id());
    assert_eq!(message.email.as_str(), "kevin@example.com");
    assert_eq!(message.username, *kevin.username());
    assert_eq!(
        message.display_name.as_ref().map(DisplayName::as_str),
        Some("Kevin K.")
    );
    assert_eq!(
        message.link,
        format!(
            "https://scylla.example.com/reset-password#token={}",
            stored[0].token().as_str()
        )
    );
    assert_eq!(message.expires_at, stored[0].expires_at());
    assert_eq!(
        stored[0].expires_at() - stored[0].created_at(),
        Duration::hours(1)
    );
    assert!(permissions.permissions().is_empty());
}

#[tokio::test]
async fn the_answer_is_the_same_for_an_unknown_email_and_an_inactive_account() {
    let (lab, _) = reset_lab(Outbox::default()).await;
    let idle = account(&lab, "idle", false);
    account(&lab, "kevin", true);

    let unknown = lab.request("ghost@example.com").await.unwrap();
    let inactive = lab.request("idle@example.com").await.unwrap();
    let known = lab.request("kevin@example.com").await.unwrap();
    lab.outbox.wait_for(1).await;

    assert_eq!(unknown, known);
    assert_eq!(inactive, known);
    assert!(lab.accounts.resets(idle.id()).is_empty());
    assert_eq!(lab.outbox.sent().len(), 1);
    assert_eq!(lab.outbox.sent()[0].user_id, UserId::new("kevin"));
}

#[tokio::test]
async fn the_answer_comes_before_the_delivery() {
    let (lab, _) = reset_lab(Outbox::held()).await;
    account(&lab, "kevin", true);

    lab.request("kevin@example.com").await.unwrap();

    assert!(lab.outbox.sent().is_empty());
    lab.outbox.release();
    lab.outbox.wait_for(1).await;
}

#[tokio::test]
async fn the_answer_does_not_wait_for_the_store() {
    let (lab, _) = reset_lab_with(Outbox::default(), Some(Semaphore::new(0)), false).await;
    let kevin = account(&lab, "kevin", true);

    let known = lab.request("kevin@example.com").await.unwrap();
    let unknown = lab.request("ghost@example.com").await.unwrap();

    assert_eq!(known, unknown);
    assert!(lab.accounts.resets(kevin.id()).is_empty());
    assert!(lab.outbox.sent().is_empty());
    lab.store.release();
    lab.outbox.wait_for(1).await;
    assert_eq!(lab.accounts.resets(kevin.id()).len(), 1);
}

#[tokio::test]
async fn a_store_error_after_the_answer_reaches_no_caller() {
    let (lab, _) = reset_lab_with(Outbox::default(), None, true).await;
    account(&lab, "kevin", true);

    let known = lab.request("kevin@example.com").await.unwrap();
    let unknown = lab.request("ghost@example.com").await.unwrap();

    assert_eq!(known, unknown);
    assert!(lab.outbox.sent().is_empty());
}

#[tokio::test]
async fn a_second_link_within_a_minute_is_not_stored_and_not_delivered() {
    let (lab, _) = reset_lab(Outbox::default()).await;
    let kevin = account(&lab, "kevin", true);
    let email = kevin.email().unwrap();
    let first = Auth::resets(&lab.app).new_reset(&kevin, email).unwrap();
    let second = Auth::resets(&lab.app).new_reset(&kevin, email).unwrap();

    let stored = issue_and_deliver(&*lab.store, &*lab.outbox, first.clone())
        .await
        .unwrap();
    let again = issue_and_deliver(&*lab.store, &*lab.outbox, second)
        .await
        .unwrap();

    assert!(stored);
    assert!(!again);
    assert_eq!(lab.outbox.sent().len(), 1);
    let kept = lab.accounts.resets(kevin.id());
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].id(), first.reset.id());
}

#[tokio::test]
async fn a_send_that_fails_in_the_task_keeps_the_link() {
    let (lab, _) = reset_lab(Outbox::failing()).await;
    let kevin = account(&lab, "kevin", true);
    let new = Auth::resets(&lab.app)
        .new_reset(&kevin, kevin.email().unwrap())
        .unwrap();

    let err = issue_and_deliver(&*lab.store, &*lab.outbox, new)
        .await
        .unwrap_err();

    assert!(matches!(err, DomainError::Infrastructure(_)), "{err}");
    assert_eq!(lab.accounts.resets(kevin.id()).len(), 1);
}

#[tokio::test]
async fn a_request_after_the_minute_cancels_the_earlier_link() {
    let (lab, _) = reset_lab(Outbox::default()).await;
    let kevin = account(&lab, "kevin", true);
    let old = aged(
        &PasswordReset::create(
            kevin.id().clone(),
            mint_reset_token().unwrap(),
            RESET_LINK_TTL,
        ),
        Duration::seconds(61),
    );
    lab.accounts.put_reset(old.clone());

    lab.request("kevin@example.com").await.unwrap();
    lab.outbox.wait_for(1).await;

    let resets = lab.accounts.resets(kevin.id());
    assert_eq!(resets.len(), 1);
    assert_ne!(resets[0].id(), old.id());
    assert!(invalid(lab.reset(old.token(), NEW_PASSWORD).await));
}

#[tokio::test]
async fn a_reset_sets_the_password_once_and_signs_out_everywhere() {
    let (lab, _) = reset_lab(Outbox::default()).await;
    let kevin = account(&lab, "kevin", true);
    lab.accounts.open_session(kevin.id());
    lab.accounts.open_session(kevin.id());
    lab.request("kevin@example.com").await.unwrap();
    lab.outbox.wait_for(1).await;
    let token = token_of(&lab.outbox.sent()[0]);

    lab.reset(&token, NEW_PASSWORD).await.unwrap();

    assert_eq!(
        lab.stored(kevin.id()).password_hash(),
        &plain_hash(NEW_PASSWORD)
    );
    assert!(lab.accounts.sessions(kevin.id()).is_empty());
    let resets = lab.accounts.resets(kevin.id());
    assert_eq!(resets.len(), 1);
    assert!(resets[0].used_at().is_some());

    assert!(invalid(lab.reset(&token, "OtherPass123!").await));
    assert_eq!(
        lab.stored(kevin.id()).password_hash(),
        &plain_hash(NEW_PASSWORD)
    );
}

#[tokio::test]
async fn an_unknown_expired_or_inactive_link_gives_one_error() {
    let (lab, _) = reset_lab(Outbox::default()).await;
    let kevin = account(&lab, "kevin", true);
    let idle = account(&lab, "idle", false);
    let expired = aged(
        &PasswordReset::create(
            kevin.id().clone(),
            mint_reset_token().unwrap(),
            RESET_LINK_TTL,
        ),
        Duration::minutes(61),
    );
    let of_idle = PasswordReset::create(
        idle.id().clone(),
        mint_reset_token().unwrap(),
        RESET_LINK_TTL,
    );
    lab.accounts.put_reset(expired.clone());
    lab.accounts.put_reset(of_idle.clone());

    for token in [
        mint_reset_token().unwrap(),
        expired.token().clone(),
        of_idle.token().clone(),
    ] {
        assert!(invalid(lab.reset(&token, NEW_PASSWORD).await));
    }
    assert_eq!(
        lab.stored(kevin.id()).password_hash(),
        &plain_hash(PASSWORD)
    );
    assert_eq!(lab.stored(idle.id()).password_hash(), &plain_hash(PASSWORD));
}

async fn serve(lab: Lab) -> AuthServiceClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Server::builder().add_service(AuthServiceServer::new(Grpc::new(lab.app)));
    tokio::spawn(router.serve_with_incoming(TcpIncoming::from(listener)));
    let channel = Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .user_agent("scylla-test/1.0")
        .unwrap()
        .connect()
        .await
        .unwrap();
    AuthServiceClient::new(channel)
}

fn login_request(bearer: Option<&str>) -> tonic::Request<LoginRequest> {
    let mut request = tonic::Request::new(LoginRequest {
        identifier: "kevin".into(),
        password: PASSWORD.into(),
    });
    if let Some(bearer) = bearer {
        request
            .metadata_mut()
            .insert("authorization", bearer.parse().unwrap());
    }
    request
}

#[tokio::test]
async fn grpc_login_records_the_peer_address_and_the_user_agent() {
    let lab = session_lab(true, StubSessions::default()).await;
    let sessions = lab.sessions.clone();
    let mut client = serve(lab).await;

    let answer = client
        .login(login_request(None))
        .await
        .unwrap()
        .into_inner();

    let rows = sessions.rows();
    assert_eq!(answer.token, rows[0].token());
    assert_eq!(answer.user_id.unwrap().value, "kevin");
    let client = rows[0].client().clone();
    assert_eq!(client.ip_address(), Some("127.0.0.1".parse().unwrap()));
    assert!(
        client
            .user_agent()
            .is_some_and(|agent| agent.starts_with("scylla-test/1.0")),
        "{client:?}"
    );
}

#[tokio::test]
async fn grpc_auth_is_public_and_ignores_a_bad_bearer_header() {
    let lab = session_lab(true, StubSessions::default()).await;
    let sessions = lab.sessions.clone();
    let mut client = serve(lab).await;

    for bearer in ["Basic abc", "Bearer nope"] {
        client.login(login_request(Some(bearer))).await.unwrap();
    }

    assert_eq!(sessions.rows().len(), 2);
}
