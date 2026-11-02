//! The tests of the API key authentication.

use scylla_core::features::api_keys;
use scylla_domain::api_keys::ApiKeyId;
use scylla_proto::projects::v1::CreateProjectRequest;
use scylla_testkit::{text, with_key};
use sqlx::PgPool;
use tonic::{Code, Request};

use crate::app;

/// Makes a creation request for the caller.
fn create(name: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        owner_id: None,
        name: name.to_owned(),
    }
}

/// A request with the key of an actor acts as that actor.
#[sqlx::test(migrations = false)]
async fn create_project_accepts_an_api_key(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;
    let key = app.api_key(actor).await;

    // when
    let response = app
        .projects
        .clone()
        .create_project(with_key(&key, create("alpha")))
        .await
        .unwrap();

    // then
    let project = response.into_inner().project.unwrap();
    assert_eq!(text(&project.created_by), actor.to_string());
}

/// A revoked key, an unknown key and a header that is not `Bearer` are refused.
#[sqlx::test(migrations = false)]
async fn create_project_refuses_a_bad_api_key(pool: PgPool) {
    // given
    let app = app(pool).await;
    let actor = app.actor().await;
    let key = api_keys::create(&app.pool, actor).await.unwrap();
    api_keys::revoke(&app.pool, key.id).await.unwrap();
    let mut basic = Request::new(create("alpha"));
    basic
        .metadata_mut()
        .insert(api_keys::HEADER, "Basic abc".parse().unwrap());
    let mut client = app.projects.clone();

    // when
    let revoked = client
        .create_project(with_key(&key.secret, create("a")))
        .await;
    let unknown = client
        .create_project(with_key("scy_unknown", create("b")))
        .await;
    let not_bearer = client.create_project(basic).await;

    // then
    assert_eq!(revoked.unwrap_err().code(), Code::Unauthenticated);
    assert_eq!(unknown.unwrap_err().code(), Code::Unauthenticated);
    assert_eq!(not_bearer.unwrap_err().code(), Code::Unauthenticated);
}

/// A revoke of an unknown key gives `NotFound`.
#[sqlx::test(migrations = false)]
async fn revoke_refuses_an_unknown_key(pool: PgPool) {
    // given
    let app = app(pool).await;

    // when
    let result = api_keys::revoke(&app.pool, ApiKeyId::generate()).await;

    // then
    assert!(matches!(result, Err(scylla_core::Error::NotFound("key"))));
}
