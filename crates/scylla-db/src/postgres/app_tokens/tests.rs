use super::PgAppTokenRepository;
use crate::domain::app::{App, AppCredential, AppToken};
use crate::domain::app::{AppName, AppSecret, AppSecretLabel};
use crate::domain::caller::CallerContext;
use crate::domain::errors::DomainResult;
use crate::domain::ids::AppId;
use crate::postgres::{PgAppCredentialRepository, PgAppRepository};
use crate::test_support::prelude::*;
use scylla_core::application::HashService;
use scylla_core::application::app::{
    AppRepository, AppTokenRepository, AppTokenUseCases, IssueAppToken,
};
use scylla_core::infrastructure::Argon2HashService;
use scylla_core::test_support::authz::DenyingPermissionService;
use sqlx::PgPool;
use std::sync::Arc;

async fn seed_app(pool: &PgPool, secret: &AppSecret) -> App {
    let org = seed_org(pool, "Acme").await;
    let hash = Argon2HashService::new().hash_secret(secret).await.unwrap();
    let app = App::create(org.id().clone(), AppName::new("ci").unwrap()).unwrap();
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

async fn issue(pool: &PgPool, app_id: AppId, secret: AppSecret) -> DomainResult<AppToken> {
    let use_cases = AppTokenUseCases::new(
        Arc::new(PgAppRepository::new(pool.clone())),
        Arc::new(PgAppTokenRepository::new(pool.clone())),
        Arc::new(PgAppCredentialRepository::new(pool.clone())),
        Arc::new(Argon2HashService::new()),
    );
    actions(Arc::new(DenyingPermissionService::new()))
        .run(
            &use_cases,
            &CallerContext::Anonymous,
            IssueAppToken { app_id, secret },
        )
        .await
}

#[sqlx::test(migrations = "../../migrations")]
async fn issue_with_correct_secret_then_token_resolves(pool: PgPool) {
    let secret = scylla_core::application::app::mint_app_secret();
    let app = seed_app(&pool, &secret).await;

    let token = issue(&pool, app.id().clone(), secret).await.expect("issue");

    let found = PgAppTokenRepository::new(pool.clone())
        .find_by_token(token.token())
        .await
        .expect("token persisted");
    assert_eq!(found.app_id(), app.id());
    assert!(!found.is_expired());
}

#[sqlx::test(migrations = "../../migrations")]
async fn issue_with_wrong_secret_is_unauthorized(pool: PgPool) {
    let secret = scylla_core::application::app::mint_app_secret();
    let app = seed_app(&pool, &secret).await;

    let result = issue(
        &pool,
        app.id().clone(),
        scylla_core::application::app::mint_app_secret(),
    )
    .await;
    assert!(matches!(
        result,
        Err(crate::domain::errors::DomainError::Unauthorized(_))
    ));
}

#[sqlx::test(migrations = "../../migrations")]
async fn the_store_keeps_a_digest_and_each_issued_token_stays_valid(pool: PgPool) {
    let secret = scylla_core::application::app::mint_app_secret();
    let app = seed_app(&pool, &secret).await;
    let first = issue(&pool, app.id().clone(), secret.clone())
        .await
        .unwrap();
    let second = issue(&pool, app.id().clone(), secret).await.unwrap();

    let stored: Vec<String> =
        sqlx::query_scalar("SELECT token_hash FROM app_tokens WHERE app_id = $1")
            .bind(app.id().as_str())
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(stored.len(), 2);
    let repo = PgAppTokenRepository::new(pool.clone());
    for issued in [&first, &second] {
        assert!(!stored.iter().any(|digest| digest == issued.token()));
        let found = repo.find_by_token(issued.token()).await.unwrap();
        assert_eq!(found.id(), issued.id());
        assert_eq!(found.token(), issued.token());
    }
}
