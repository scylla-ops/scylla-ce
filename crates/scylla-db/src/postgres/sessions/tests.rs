use super::PgSessionRepository;
use crate::domain::clock;
use crate::domain::errors::DomainError;
use crate::domain::ids::SessionId;
use crate::domain::session::{ACTIVITY_INTERVAL, SessionClient};
use crate::postgres::PgUserRepository;
use crate::test_support::prelude::*;
use chrono::Duration;
use scylla_core::application::{SessionRepository, UserRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../migrations")]
async fn create_then_find_by_token(pool: PgPool) {
    let user = seed_user(&pool, "alice").await;
    let repo = PgSessionRepository::new(pool);
    let session = SessionBuilder::new(user.id()).build();

    repo.create(&session).await.expect("create");
    let found = repo.find_by_token(session.token()).await.expect("find");

    assert_eq!(found.id(), session.id());
    assert_eq!(found.user_id(), user.id());
    assert_eq!(found.created_at(), session.created_at());
    assert_eq!(found.expires_at(), session.expires_at());
}

#[sqlx::test(migrations = "../../migrations")]
async fn find_by_token_not_found(pool: PgPool) {
    let repo = PgSessionRepository::new(pool);
    let res = repo.find_by_token("does-not-exist").await;
    assert!(matches!(res, Err(DomainError::NotFound(_))));
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_expired_removes_only_past_sessions(pool: PgPool) {
    let user = seed_user(&pool, "expired-dave").await;
    let repo = PgSessionRepository::new(pool);

    let fresh = SessionBuilder::new(user.id()).build();
    repo.create(&fresh).await.expect("seed fresh");

    let expired = SessionBuilder::new(user.id()).expired(true).build();
    repo.create(&expired).await.expect("seed expired");

    let removed = repo.delete_expired().await.expect("sweep");
    assert_eq!(removed, 1);
    assert!(repo.find_by_token(fresh.token()).await.is_ok());
    assert!(repo.find_by_token(expired.token()).await.is_err());
}

#[sqlx::test(migrations = "../../migrations")]
async fn cascade_user_delete_removes_sessions(pool: PgPool) {
    let user = seed_user(&pool, "evictee").await;
    let session_repo = PgSessionRepository::new(pool.clone());
    let user_repo = PgUserRepository::new(pool);

    let session = SessionBuilder::new(user.id()).build();
    session_repo.create(&session).await.expect("create");

    user_repo.delete(&user).await.expect("delete user");

    assert!(matches!(
        session_repo.find_by_token(session.token()).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_store_keeps_a_digest_and_a_revoke_by_token_removes_the_session(pool: PgPool) {
    let user = seed_user(&pool, "digest").await;
    let repo = PgSessionRepository::new(pool.clone());
    let session = SessionBuilder::new(user.id()).build();
    repo.create(&session).await.expect("create");

    let stored: String = sqlx::query_scalar("SELECT token_hash FROM sessions WHERE id = $1")
        .bind(session.id().as_str())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_ne!(stored, session.token());
    assert!(repo.find_by_token(&stored).await.is_err());
    assert_eq!(
        repo.find_by_token(session.token()).await.unwrap().token(),
        session.token()
    );

    repo.delete_by_token(session.token()).await.unwrap();
    assert!(matches!(
        repo.find_by_token(session.token()).await,
        Err(DomainError::NotFound(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_session_keeps_the_client_that_opened_it(pool: PgPool) {
    let user = seed_user(&pool, "alice").await;
    let repo = PgSessionRepository::new(pool);
    let client = SessionClient::new(Some("Mozilla/5.0 (X11; Linux x86_64)"), Some("2001:db8::7"));
    let with = SessionBuilder::new(user.id())
        .client(client.clone())
        .build();
    let without = SessionBuilder::new(user.id()).build();

    repo.create(&with).await.unwrap();
    repo.create(&without).await.unwrap();

    assert_eq!(
        repo.find_by_token(with.token()).await.unwrap().client(),
        &client
    );
    assert_eq!(
        repo.find_by_token(without.token()).await.unwrap().client(),
        &SessionClient::default()
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_touch_moves_the_activity_at_most_once_in_the_interval(pool: PgPool) {
    let user = seed_user(&pool, "touchy").await;
    let repo = PgSessionRepository::new(pool);
    let now = clock::now();
    let idle = SessionBuilder::new(user.id())
        .created_at(now - Duration::hours(1))
        .expires_at(now + Duration::hours(1))
        .last_active_at(now - ACTIVITY_INTERVAL)
        .build();
    let fresh = SessionBuilder::new(user.id())
        .last_active_at(now - ACTIVITY_INTERVAL + Duration::seconds(1))
        .build();
    repo.create(&idle).await.unwrap();
    repo.create(&fresh).await.unwrap();

    assert!(repo.touch(idle.id(), now).await.unwrap());
    assert!(
        !repo
            .touch(idle.id(), now + Duration::minutes(1))
            .await
            .unwrap()
    );
    assert!(!repo.touch(fresh.id(), now).await.unwrap());
    assert!(!repo.touch(&SessionId::new("unknown"), now).await.unwrap());

    let moved = repo.find_by_token(idle.token()).await.unwrap();
    assert_eq!(moved.last_active_at(), now);
    assert_eq!(moved.created_at(), idle.created_at());
    assert_eq!(
        repo.find_by_token(fresh.token())
            .await
            .unwrap()
            .last_active_at(),
        fresh.last_active_at()
    );

    let later = now + ACTIVITY_INTERVAL;
    assert!(repo.touch(idle.id(), later).await.unwrap());
    assert_eq!(
        repo.find_by_token(idle.token())
            .await
            .unwrap()
            .last_active_at(),
        later
    );
}
