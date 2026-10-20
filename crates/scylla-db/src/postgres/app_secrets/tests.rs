use super::PgAppCredentialRepository;
use crate::domain::app::{App, AppCredential, AppName, AppSecretHash, AppSecretLabel};
use crate::domain::errors::DomainError;
use crate::domain::ids::{AppCredentialId, AppId};
use crate::postgres::PgAppRepository;
use crate::test_support::prelude::*;
use chrono::{TimeZone, Utc};
use scylla_core::application::app::{AppCredentialRepository, AppRepository};
use sqlx::PgPool;

const TEST_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHQ$aGFzaGhhc2g";

fn credential(app_id: &AppId, label: &str) -> AppCredential {
    let long_ago = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    AppCredential::from_persistence(
        AppCredentialId::generate(),
        app_id.clone(),
        AppSecretLabel::new(label).unwrap(),
        AppSecretHash::new(TEST_HASH).unwrap(),
        true,
        long_ago,
        long_ago,
    )
}

async fn app_with_secret(pool: &PgPool) -> (App, AppCredential) {
    let org = seed_org(pool, "Acme").await;
    let app = App::create(org.id().clone(), AppName::new("ci-bot").unwrap()).unwrap();
    let first = credential(app.id(), "first");
    PgAppRepository::new(pool.clone())
        .create_app(&app, &first)
        .await
        .unwrap();
    (app, first)
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_new_label_is_stored_and_bumps_the_update_time(pool: PgPool) {
    let (_, first) = app_with_secret(&pool).await;
    let repo = PgAppCredentialRepository::new(pool.clone());

    repo.set_label(
        first.id(),
        &AppSecretLabel::new("Build-01.example.com").unwrap(),
    )
    .await
    .unwrap();

    let found = repo.find_by_id(first.id()).await.unwrap();
    assert_eq!(found.label().as_str(), "Build-01.example.com");
    assert!(found.updated_at() > first.updated_at());
    assert_eq!(found.secret_hash().as_str(), first.secret_hash().as_str());
}

#[sqlx::test(migrations = "../../migrations")]
async fn a_label_of_another_secret_of_the_app_conflicts(pool: PgPool) {
    let (app, first) = app_with_secret(&pool).await;
    let repo = PgAppCredentialRepository::new(pool.clone());
    repo.create(&credential(app.id(), "second")).await.unwrap();

    let err = repo
        .set_label(first.id(), &AppSecretLabel::new("second").unwrap())
        .await
        .unwrap_err();
    repo.set_label(first.id(), &AppSecretLabel::new("first").unwrap())
        .await
        .expect("its own label is not a conflict");

    assert!(
        matches!(&err, DomainError::Conflict(m) if m == "App secret label already exists on this app"),
        "{err:?}"
    );
    assert_eq!(
        repo.find_by_id(first.id()).await.unwrap().label().as_str(),
        "first"
    );
}
