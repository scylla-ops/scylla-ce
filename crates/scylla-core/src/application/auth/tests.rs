//! The session's actions through the engine, on stub ports.

use super::*;
use crate::domain::app::{AppSecret, AppSecretHash};
use crate::domain::caller::{CallerContext, ServiceIdentity};
use crate::domain::errors::{DomainError, DomainResult};
use crate::domain::session::{ACTIVITY_INTERVAL, SessionClient};
use crate::domain::user::{Password, PasswordHash};
use crate::test_support::authz::{DenyingPermissionService, actions};
use crate::test_support::sessions::SessionBuilder;
use crate::test_support::stubs::{OneUser, StubSessions};
use crate::test_support::users::UserBuilder;
use async_trait::async_trait;
use scylla_extension::Actions;

const PASSWORD: &str = "SecurePass123!";

struct CheckingHash;

#[async_trait]
impl HashService for CheckingHash {
    async fn hash(&self, _: &Password) -> DomainResult<PasswordHash> {
        unreachable!("no password to hash in a session action")
    }
    async fn verify(&self, password: &Password, _: &PasswordHash) -> DomainResult<bool> {
        Ok(password.as_str() == PASSWORD)
    }
    async fn hash_secret(&self, _: &AppSecret) -> DomainResult<AppSecretHash> {
        unreachable!("no app secret in a session action")
    }
    async fn verify_secret(&self, _: &AppSecret, _: &AppSecretHash) -> DomainResult<bool> {
        unreachable!("no app secret in a session action")
    }
}

struct Lab {
    actions: Actions,
    uc: AuthUseCases,
    sessions: Arc<StubSessions>,
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
            client,
        };
        self.actions
            .run(&self.uc, &CallerContext::Anonymous, login)
            .await
    }

    async fn validate(&self, token: &str) -> bool {
        let query = ValidateToken {
            token: token.to_string(),
        };
        self.actions
            .run(&self.uc, &CallerContext::Anonymous, query)
            .await
            .unwrap()
    }
}

fn lab(active: bool, sessions: StubSessions) -> Lab {
    let user = UserBuilder::new("kevin")
        .id(UserId::new("kevin"))
        .email("kevin@example.com")
        .is_active(active)
        .build();
    let sessions = Arc::new(sessions);
    Lab {
        actions: actions(Arc::new(DenyingPermissionService::new())),
        uc: AuthUseCases::new(
            Arc::new(OneUser(user)),
            sessions.clone(),
            Arc::new(CheckingHash),
        ),
        sessions,
    }
}

#[tokio::test]
async fn a_login_by_username_or_email_stores_a_session_without_asking_a_permission() {
    let lab = lab(true, StubSessions::default());

    let by_name = lab.login("kevin", PASSWORD).await.unwrap();
    let by_email = lab.login("kevin@example.com", PASSWORD).await.unwrap();

    assert_eq!(by_name.user_id(), &UserId::new("kevin"));
    assert_ne!(by_name.token(), by_email.token());
    assert_eq!(lab.sessions.rows().len(), 2);
}

#[tokio::test]
async fn a_login_records_the_client_of_the_call_on_the_new_session() {
    let lab = lab(true, StubSessions::default());
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
    let lab = lab(true, StubSessions::default());

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
    let lab = lab(false, StubSessions::default());

    let err = lab.login("kevin", PASSWORD).await.unwrap_err();

    assert!(matches!(&err, DomainError::Unauthorized(m) if m == "User account is inactive"));
    assert!(lab.sessions.rows().is_empty());
}

#[tokio::test]
async fn an_inactive_account_with_a_wrong_password_gives_the_opaque_error() {
    let lab = lab(false, StubSessions::default());

    for identifier in ["kevin", "kevin@example.com"] {
        let err = lab.login(identifier, "WrongPass123!").await.unwrap_err();
        assert!(matches!(&err, DomainError::Unauthorized(m) if m == "Invalid credentials"));
    }
    assert!(lab.sessions.rows().is_empty());
}

#[tokio::test]
async fn a_live_token_is_valid_and_a_check_deletes_nothing() {
    let user_id = UserId::new("kevin");
    let live = lab(
        true,
        StubSessions::with(SessionBuilder::new(&user_id).token("live").build()),
    );
    let expired = lab(
        true,
        StubSessions::with(
            SessionBuilder::new(&user_id)
                .token("expired")
                .expired(true)
                .build(),
        ),
    );

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
    let lab = lab(
        true,
        StubSessions::with(SessionBuilder::new(&user_id).token("live").build()),
    );
    lab.sessions
        .create(
            &SessionBuilder::new(&user_id)
                .token("expired")
                .expired(true)
                .build(),
        )
        .await
        .unwrap();
    let service = CallerContext::Service(ServiceIdentity::session_sweeper());

    let refused = lab
        .actions
        .run(&lab.uc, &CallerContext::Anonymous, PurgeExpiredSessions)
        .await
        .unwrap_err();
    let purged = lab
        .actions
        .run(&lab.uc, &service, PurgeExpiredSessions)
        .await
        .unwrap();

    assert!(matches!(refused, DomainError::Forbidden(_)));
    assert_eq!(purged, 1);
    let rows = lab.sessions.rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].token(), "live");
}

#[tokio::test]
async fn a_revoke_deletes_the_session() {
    let user_id = UserId::new("kevin");
    let lab = lab(
        true,
        StubSessions::with(SessionBuilder::new(&user_id).token("t").build()),
    );

    lab.actions
        .run(
            &lab.uc,
            &CallerContext::Anonymous,
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
    let now = crate::domain::clock::now();
    let fresh = SessionBuilder::new(&user_id)
        .last_active_at(now - ACTIVITY_INTERVAL + chrono::Duration::seconds(30))
        .expires_at(now + chrono::Duration::hours(1))
        .build();
    let idle = SessionBuilder::new(&user_id)
        .last_active_at(now - ACTIVITY_INTERVAL - chrono::Duration::seconds(1))
        .expires_at(now + chrono::Duration::hours(1))
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
        .last_active_at(crate::domain::clock::now() - ACTIVITY_INTERVAL * 2)
        .build();

    assert!(
        Silent
            .touch(idle.id(), crate::domain::clock::now())
            .await
            .is_err()
    );
    record_activity(&Silent, &idle).await;
}
