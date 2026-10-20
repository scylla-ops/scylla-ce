use super::PgAccountRepository;
use crate::domain::clock;
use crate::domain::errors::DomainError;
use crate::domain::ids::{SessionId, UserId};
use crate::domain::role::RoleName;
use crate::domain::session::{Session, SessionClient};
use crate::domain::user::{
    DisplayName, PasswordHash, PasswordReset, RESET_LINK_INVALID, ResetToken, User,
};
use crate::postgres::{PgGrantRepository, PgSessionRepository, PgUserRepository};
use crate::test_support::prelude::*;
use chrono::Duration;
use scylla_auth::authz::{
    Grant, GrantRepository, ORGANIZATION_ADMIN_ROLE, PROJECT_ADMIN_ROLE, Principal,
    SYSTEM_ADMIN_ROLE, Scope,
};
use scylla_core::application::{AccountRepository, SessionRepository, UserRepository};
use sqlx::PgPool;

fn token(seed: char) -> ResetToken {
    ResetToken::new(seed.to_string().repeat(43)).unwrap()
}

fn reset(user: &User, seed: char) -> PasswordReset {
    PasswordReset::create(user.id().clone(), token(seed), Duration::hours(1))
}

fn aged(reset: &PasswordReset, age: Duration) -> PasswordReset {
    PasswordReset::from_persistence(
        reset.id().clone(),
        reset.token().clone(),
        reset.user_id().clone(),
        reset.created_at() - age,
        reset.expires_at() - age,
        None,
    )
}

async fn tokens_of(pool: &PgPool, user: &User) -> Vec<String> {
    sqlx::query_scalar("SELECT token_hash FROM password_resets WHERE user_id = $1 ORDER BY id")
        .bind(user.id().as_str())
        .fetch_all(pool)
        .await
        .unwrap()
}

async fn sessions_of(pool: &PgPool, user: &User) -> Vec<String> {
    sqlx::query_scalar("SELECT id FROM sessions WHERE user_id = $1 ORDER BY id")
        .bind(user.id().as_str())
        .fetch_all(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_link_keeps_only_the_digest_and_a_new_link_cancels_the_earlier(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let repo = PgAccountRepository::new(pool.clone());
    let first = reset(&kevin, 'a');

    assert!(repo.issue_reset(&first, None).await.unwrap());
    let stored = tokens_of(&pool, &kevin).await;
    assert_eq!(stored.len(), 1);
    assert_ne!(stored[0], first.token().as_str());
    let found = repo.find_reset(first.token()).await.unwrap();
    assert_eq!(found.id(), first.id());
    assert_eq!(found.user_id(), kevin.id());
    assert_eq!(found.expires_at(), first.expires_at());
    assert!(found.used_at().is_none());

    let second = reset(&kevin, 'b');
    assert!(repo.issue_reset(&second, None).await.unwrap());
    assert_eq!(tokens_of(&pool, &kevin).await.len(), 1);
    assert!(matches!(
        repo.find_reset(first.token()).await,
        Err(DomainError::NotFound(_))
    ));
    assert!(repo.find_reset(second.token()).await.is_ok());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_link_within_the_cooldown_is_not_stored(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let repo = PgAccountRepository::new(pool.clone());
    let cooldown = Some(Duration::seconds(60));
    repo.issue_reset(&aged(&reset(&kevin, 'a'), Duration::seconds(30)), None)
        .await
        .unwrap();

    assert!(
        !repo
            .issue_reset(&reset(&kevin, 'b'), cooldown)
            .await
            .unwrap()
    );
    assert!(repo.find_reset(&token('a')).await.is_ok());
    assert!(repo.find_reset(&token('b')).await.is_err());

    let other = seed_user(&pool, "other").await;
    assert!(
        repo.issue_reset(&reset(&other, 'c'), cooldown)
            .await
            .unwrap()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_link_after_the_cooldown_replaces_the_earlier(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let repo = PgAccountRepository::new(pool.clone());
    repo.issue_reset(&aged(&reset(&kevin, 'a'), Duration::seconds(61)), None)
        .await
        .unwrap();

    assert!(
        repo.issue_reset(&reset(&kevin, 'b'), Some(Duration::seconds(60)))
            .await
            .unwrap()
    );
    assert!(repo.find_reset(&token('a')).await.is_err());
    assert!(repo.find_reset(&token('b')).await.is_ok());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_link_for_an_unknown_user_is_not_found(pool: PgPool) {
    let repo = PgAccountRepository::new(pool);
    let ghost = user("ghost");

    assert!(matches!(
        repo.issue_reset(&reset(&ghost, 'a'), None).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_redeem_writes_the_password_marks_the_link_and_signs_out(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    seed_session(&pool, kevin.id()).await;
    seed_session(&pool, kevin.id()).await;
    let carol = seed_user(&pool, "carol").await;
    seed_session(&pool, carol.id()).await;
    let repo = PgAccountRepository::new(pool.clone());
    let link = reset(&kevin, 'a');
    repo.issue_reset(&link, None).await.unwrap();
    sqlx::query(
        "INSERT INTO password_resets (id, user_id, token_hash, created_at, expires_at) \
         VALUES ('other', $1, 'x', NOW(), NOW() + INTERVAL '1 hour')",
    )
    .bind(kevin.id().as_str())
    .execute(&pool)
    .await
    .unwrap();

    let mut changed = PgUserRepository::new(pool.clone())
        .find_by_id(kevin.id())
        .await
        .unwrap();
    changed.set_password_hash(PasswordHash::new("$new$hash").unwrap());
    let written = repo.redeem_reset(&link, &changed).await.unwrap();

    assert_eq!(written.password_hash().as_str(), "$new$hash");
    assert_eq!(written.version(), 1);
    assert!(sessions_of(&pool, &kevin).await.is_empty());
    assert_eq!(sessions_of(&pool, &carol).await.len(), 1);
    assert_eq!(tokens_of(&pool, &kevin).await.len(), 1);
    assert!(
        repo.find_reset(link.token())
            .await
            .unwrap()
            .used_at()
            .is_some()
    );

    let again = repo.redeem_reset(&link, &written).await.unwrap_err();
    assert!(matches!(&again, DomainError::BusinessRule(m) if m == RESET_LINK_INVALID));
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_expired_link_is_not_redeemed_and_nothing_changes(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let session = seed_session(&pool, kevin.id()).await;
    let repo = PgAccountRepository::new(pool.clone());
    let link = aged(&reset(&kevin, 'a'), Duration::minutes(61));
    repo.issue_reset(&link, None).await.unwrap();

    let mut changed = kevin.clone();
    changed.set_password_hash(PasswordHash::new("$new$hash").unwrap());
    let err = repo.redeem_reset(&link, &changed).await.unwrap_err();

    assert!(matches!(&err, DomainError::BusinessRule(m) if m == RESET_LINK_INVALID));
    assert_eq!(
        sessions_of(&pool, &kevin).await,
        vec![session.id().to_string()]
    );
    let stored = PgUserRepository::new(pool)
        .find_by_id(kevin.id())
        .await
        .unwrap();
    assert_eq!(stored.password_hash(), kevin.password_hash());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_stale_user_rolls_the_redeem_back(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    seed_session(&pool, kevin.id()).await;
    let users = PgUserRepository::new(pool.clone());
    let repo = PgAccountRepository::new(pool.clone());
    let link = reset(&kevin, 'a');
    repo.issue_reset(&link, None).await.unwrap();
    let mut renamed = kevin.clone();
    renamed.set_display_name(Some(DisplayName::new("Kev").unwrap()));
    users.update(&renamed).await.unwrap();

    let err = repo.redeem_reset(&link, &kevin).await.unwrap_err();

    assert!(matches!(err, DomainError::Stale(_)), "{err}");
    assert!(
        repo.find_reset(link.token())
            .await
            .unwrap()
            .used_at()
            .is_none()
    );
    assert_eq!(sessions_of(&pool, &kevin).await.len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_update_signed_out_keeps_one_session_and_cancels_the_links(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let kept = seed_session(&pool, kevin.id()).await;
    seed_session(&pool, kevin.id()).await;
    let repo = PgAccountRepository::new(pool.clone());
    repo.issue_reset(&reset(&kevin, 'a'), None).await.unwrap();

    let mut off = kevin.clone();
    assert!(off.set_active(false));
    let written = repo.update_signed_out(&off, Some(kept.id())).await.unwrap();

    assert!(!written.is_active());
    assert_eq!(
        sessions_of(&pool, &kevin).await,
        vec![kept.id().to_string()]
    );
    assert!(tokens_of(&pool, &kevin).await.is_empty());

    let err = repo.update_signed_out(&off, None).await.unwrap_err();
    assert!(matches!(err, DomainError::Stale(_)), "{err}");
    assert_eq!(sessions_of(&pool, &kevin).await.len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_revoke_counts_the_sessions_and_spares_one(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let kept = seed_session(&pool, kevin.id()).await;
    seed_session(&pool, kevin.id()).await;
    seed_session(&pool, kevin.id()).await;
    let carol = seed_user(&pool, "carol").await;
    seed_session(&pool, carol.id()).await;
    let repo = PgAccountRepository::new(pool.clone());

    assert_eq!(
        repo.revoke_sessions(kevin.id(), Some(kept.id()))
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sessions_of(&pool, &kevin).await,
        vec![kept.id().to_string()]
    );
    assert_eq!(repo.revoke_sessions(kevin.id(), None).await.unwrap(), 1);
    assert_eq!(sessions_of(&pool, &carol).await.len(), 1);
    assert!(
        PgSessionRepository::new(pool)
            .find_by_token(kept.token())
            .await
            .is_err()
    );
}

async fn stored_session(
    pool: &PgPool,
    user: &User,
    active: Duration,
    expires: Duration,
    client: SessionClient,
) -> Session {
    let now = clock::now();
    let session = SessionBuilder::new(user.id())
        .created_at(now - active - Duration::minutes(1))
        .last_active_at(now - active)
        .expires_at(now + expires)
        .client(client)
        .build();
    PgSessionRepository::new(pool.clone())
        .create(&session)
        .await
        .unwrap();
    session
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_live_sessions_of_a_user_come_most_recently_active_first(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let carol = seed_user(&pool, "carol").await;
    let hour = Duration::hours(1);
    let browser = SessionClient::new(Some("Firefox/131.0"), Some("203.0.113.7"));
    let old = stored_session(&pool, &kevin, hour, hour, SessionClient::default()).await;
    let new = stored_session(&pool, &kevin, Duration::zero(), hour, browser.clone()).await;
    let mid = stored_session(
        &pool,
        &kevin,
        Duration::minutes(10),
        hour,
        SessionClient::default(),
    )
    .await;
    stored_session(
        &pool,
        &kevin,
        hour * 2,
        -Duration::minutes(1),
        SessionClient::default(),
    )
    .await;
    stored_session(
        &pool,
        &carol,
        Duration::zero(),
        hour,
        SessionClient::default(),
    )
    .await;
    let repo = PgAccountRepository::new(pool.clone());

    let sessions = repo.list_sessions(kevin.id()).await.unwrap();

    let ids: Vec<_> = sessions.iter().map(|s| s.id.clone()).collect();
    assert_eq!(
        ids,
        vec![new.id().clone(), mid.id().clone(), old.id().clone()]
    );
    assert_eq!(sessions[0].client, browser);
    assert_eq!(sessions[0].created_at, new.created_at());
    assert_eq!(sessions[0].last_active_at, new.last_active_at());
    assert_eq!(sessions[0].expires_at, new.expires_at());
    assert_eq!(sessions[2].client, SessionClient::default());
    assert!(sessions.iter().all(|s| !s.current));
    assert!(
        repo.list_sessions(&UserId::new("ghost"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_single_revoke_deletes_only_a_live_session_of_the_user(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let carol = seed_user(&pool, "carol").await;
    let hour = Duration::hours(1);
    let live = stored_session(
        &pool,
        &kevin,
        Duration::zero(),
        hour,
        SessionClient::default(),
    )
    .await;
    let other = stored_session(
        &pool,
        &kevin,
        Duration::zero(),
        hour,
        SessionClient::default(),
    )
    .await;
    let expired = stored_session(
        &pool,
        &kevin,
        hour * 2,
        -Duration::minutes(1),
        SessionClient::default(),
    )
    .await;
    let carols = stored_session(
        &pool,
        &carol,
        Duration::zero(),
        hour,
        SessionClient::default(),
    )
    .await;
    let repo = PgAccountRepository::new(pool.clone());

    assert!(!repo.revoke_session(kevin.id(), carols.id()).await.unwrap());
    assert!(!repo.revoke_session(kevin.id(), expired.id()).await.unwrap());
    assert!(
        !repo
            .revoke_session(kevin.id(), &SessionId::new("unknown"))
            .await
            .unwrap()
    );
    assert!(repo.revoke_session(kevin.id(), live.id()).await.unwrap());
    assert!(!repo.revoke_session(kevin.id(), live.id()).await.unwrap());

    let mut left = vec![other.id().to_string(), expired.id().to_string()];
    left.sort();
    assert_eq!(sessions_of(&pool, &kevin).await, left);
    assert_eq!(
        sessions_of(&pool, &carol).await,
        vec![carols.id().to_string()]
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_deleted_user_takes_its_links_along(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let repo = PgAccountRepository::new(pool.clone());
    repo.issue_reset(&reset(&kevin, 'a'), None).await.unwrap();

    PgUserRepository::new(pool.clone())
        .delete(&kevin)
        .await
        .unwrap();

    assert!(tokens_of(&pool, &kevin).await.is_empty());
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_access_list_names_each_grant_in_order(pool: PgPool) {
    let kevin = seed_user(&pool, "kevin").await;
    let zeta = seed_org(&pool, "Zeta").await;
    let acme = seed_org(&pool, "Acme").await;
    let web = seed_project(&pool, &acme, "Web").await;
    let api = seed_project(&pool, &acme, "Api").await;
    let grants = PgGrantRepository::new(pool.clone());
    let grant = |role: &str, scope: Scope| {
        Grant::new(
            Principal::User(kevin.id().clone()),
            RoleName::new(role).unwrap(),
            scope,
        )
    };
    for g in [
        grant(PROJECT_ADMIN_ROLE, Scope::Project(web.id().clone())),
        grant(
            ORGANIZATION_ADMIN_ROLE,
            Scope::Organization(zeta.id().clone()),
        ),
        grant(PROJECT_ADMIN_ROLE, Scope::Project(api.id().clone())),
        grant(
            ORGANIZATION_ADMIN_ROLE,
            Scope::Organization(acme.id().clone()),
        ),
        grant(SYSTEM_ADMIN_ROLE, Scope::System),
    ] {
        grants.create(&g).await.unwrap();
    }
    let other = seed_user(&pool, "other").await;
    grants
        .create(&Grant::new(
            Principal::User(other.id().clone()),
            RoleName::new(ORGANIZATION_ADMIN_ROLE).unwrap(),
            Scope::Organization(acme.id().clone()),
        ))
        .await
        .unwrap();

    let access = PgAccountRepository::new(pool)
        .list_access(kevin.id())
        .await
        .unwrap();

    let rows: Vec<(String, String, String, String)> = access
        .iter()
        .map(|a| {
            (
                a.scope.to_string(),
                a.organization_name
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                a.project_name
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                a.role_name.to_string(),
            )
        })
        .collect();
    let row = |scope: String, org: &str, project: &str, role: &str| {
        (scope, org.to_owned(), project.to_owned(), role.to_owned())
    };
    assert_eq!(
        rows,
        vec![
            row("system".into(), "", "", "System Admin"),
            row(
                format!("organization:{}", acme.id()),
                "Acme",
                "",
                "Organization Admin"
            ),
            row(
                format!("project:{}", api.id()),
                "Acme",
                "Api",
                "Project Admin"
            ),
            row(
                format!("project:{}", web.id()),
                "Acme",
                "Web",
                "Project Admin"
            ),
            row(
                format!("organization:{}", zeta.id()),
                "Zeta",
                "",
                "Organization Admin"
            ),
        ]
    );
    assert_eq!(access[0].organization_id, None);
    assert_eq!(access[2].organization_id.as_ref(), Some(acme.id()));
    assert_eq!(access[2].role.as_str(), PROJECT_ADMIN_ROLE);
    assert!(!access[0].grant_id.is_empty());
}

mod flows {
    use super::*;
    use crate::domain::caller::CallerContext;
    use crate::domain::ids::UserId;
    use crate::domain::permission::Permission;
    use crate::domain::user::{Email, Password, Username};
    use crate::postgres::PgAuthzEntityProvider;
    use crate::postgres::PgRoleRepository;
    use async_trait::async_trait;
    use scylla_auth::audit::NoopAuditLog;
    use scylla_auth::authz::PermissionService;
    use scylla_auth::cedar::CedarPermissionService;
    use scylla_core::application::auth::{AuthUseCases, Login};
    use scylla_core::application::user::reset::{RequestPasswordReset, ResetLinks, ResetPassword};
    use scylla_core::application::user::{UpdateUser, UserUseCases};
    use scylla_core::application::{
        HashService, PasswordResetDelivery, PasswordResetMessage, PasswordResetSender,
        PasswordResetUseCases,
    };
    use scylla_core::infrastructure::Argon2HashService;
    use scylla_core::test_support::authz::DenyingPermissionService;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Outbox(Mutex<Vec<PasswordResetMessage>>);

    #[async_trait]
    impl PasswordResetSender for Outbox {
        fn delivery(&self) -> PasswordResetDelivery {
            PasswordResetDelivery::ServerLog
        }

        async fn send(
            &self,
            message: &PasswordResetMessage,
        ) -> crate::domain::errors::DomainResult<()> {
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

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_reset_link_changes_the_password_and_ends_every_session(pool: PgPool) {
        let hash = Arc::new(Argon2HashService::new());
        let users = Arc::new(PgUserRepository::new(pool.clone()));
        let sessions = Arc::new(PgSessionRepository::new(pool.clone()));
        let outbox = Arc::new(Outbox::default());
        let auth = AuthUseCases::new(users.clone(), sessions.clone(), hash.clone());
        let resets = PasswordResetUseCases::new(
            users.clone(),
            Arc::new(PgAccountRepository::new(pool.clone())),
            hash.clone(),
            outbox.clone(),
            ResetLinks::new(Some("http://127.0.0.1:8080")),
        );
        let kevin = User::create(
            Username::new("kevin").unwrap(),
            Some(Email::new("kevin@example.com").unwrap()),
            hash.hash(&Password::new("OldPass123!").unwrap())
                .await
                .unwrap(),
        );
        users.create(&kevin).await.unwrap();
        let engine = actions(Arc::new(DenyingPermissionService::new()));
        let anonymous = CallerContext::Anonymous;
        let login = |password: &str| {
            engine.run(
                &auth,
                &anonymous,
                Login {
                    identifier: "kevin".into(),
                    password: Password::new(password).unwrap(),
                    client: SessionClient::default(),
                },
            )
        };
        let old_session = login("OldPass123!").await.unwrap();

        let delivery = engine
            .run(
                &resets,
                &anonymous,
                RequestPasswordReset {
                    email: Email::new("kevin@example.com").unwrap(),
                },
            )
            .await
            .unwrap();
        let token = outbox.token().await;
        engine
            .run(
                &resets,
                &anonymous,
                ResetPassword {
                    token: token.clone(),
                    new_password: Password::new("NewPass123!").unwrap(),
                },
            )
            .await
            .unwrap();

        assert_eq!(delivery, PasswordResetDelivery::ServerLog);
        assert!(sessions.find_by_token(old_session.token()).await.is_err());
        assert!(login("OldPass123!").await.is_err());
        login("NewPass123!").await.unwrap();
        let again = engine
            .run(
                &resets,
                &anonymous,
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
        let kevin = seed_user(&pool, "kevin").await;
        let permissions = Arc::new(
            CedarPermissionService::new(
                Arc::new(PgAuthzEntityProvider::new(pool.clone())),
                Arc::new(PgRoleRepository::new(pool.clone())),
                Arc::new(PgGrantRepository::new(pool.clone())),
                Arc::new(NoopAuditLog),
            )
            .await
            .unwrap(),
        );
        let users = Arc::new(PgUserRepository::new(pool.clone()));
        let uc = UserUseCases::new(
            users.clone(),
            Arc::new(PgGrantRepository::new(pool.clone())),
            Arc::new(Argon2HashService::new()),
            Arc::new(PgAccountRepository::new(pool.clone())),
        );
        let engine = actions(permissions.clone());
        let caller = CallerContext::User(kevin.id().clone());
        let update = |display_name: Option<&str>, email: Option<&str>| UpdateUser {
            id: kevin.id().clone(),
            username: None,
            display_name: display_name.map(|d| Some(DisplayName::new(d).unwrap())),
            email: email.map(|e| Email::new(e).unwrap()),
        };

        let named = engine
            .run(&uc, &caller, update(Some("Kev"), None))
            .await
            .unwrap();
        let err = engine
            .run(&uc, &caller, update(None, Some("kev@example.com")))
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
        assert!(
            permissions
                .check(&caller, Permission::UpdateUser(UserId::new("someone-else")))
                .await
                .is_err()
        );
    }
}
