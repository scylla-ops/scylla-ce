//! The tests of the tokens: signature, expiry, revocation and attenuation.

use std::time::Duration;

use biscuit_auth::{BlockBuilder, UnverifiedBiscuit};
use scylla_core::features::actors;
use scylla_proto::projects::v1::{CreateProjectRequest, ListProjectsRequest};
use scylla_testkit::{test_access, with_token};
use sqlx::PgPool;
use tonic::Code;

use crate::{app, assert_code};

/// Makes a creation request for the caller.
fn create() -> CreateProjectRequest {
    CreateProjectRequest {
        owner_id: None,
        name: "alpha".to_owned(),
    }
}

/// A token signed by another key, an expired token and a text that is not a token are refused.
#[sqlx::test(migrations = false)]
async fn a_bad_token_is_refused(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let user = app.user().await;
    let actor = actors::find(&app.pool, user).await.unwrap();
    let foreign = test_access(pool)
        .issue_for(&actor, Duration::from_secs(600))
        .unwrap();
    let expired = app.access.issue_for(&actor, Duration::ZERO).unwrap();
    let mut client = app.projects.clone();

    // when
    let by_foreign = client
        .create_project(with_token(&foreign.value, create()))
        .await;
    let by_expired = client
        .create_project(with_token(&expired.value, create()))
        .await;
    let by_garbage = client.create_project(with_token("nope", create())).await;

    // then
    assert_code(by_foreign, Code::Unauthenticated);
    assert_code(by_expired, Code::Unauthenticated);
    assert_code(by_garbage, Code::Unauthenticated);
}

/// A revoked token, and each token made from it, is refused.
#[sqlx::test(migrations = false)]
async fn a_revoked_token_is_refused(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let token = app.token(user).await;
    let attenuated = attenuate(&token, r#"check if operation("projects.create");"#);
    app.access.revoke(&token).await.unwrap();
    let mut client = app.projects.clone();

    // when
    let by_token = client.create_project(with_token(&token, create())).await;
    let by_attenuated = client
        .create_project(with_token(&attenuated, create()))
        .await;

    // then
    assert_code(by_token, Code::Unauthenticated);
    assert_code(by_attenuated, Code::Unauthenticated);
}

/// A holder restricts its token offline: the restricted token allows only what is left.
#[sqlx::test(migrations = false)]
async fn an_attenuated_token_allows_only_its_operations(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let token = app.token(user).await;
    let read_only = attenuate(&token, r#"check if operation("projects.list");"#);
    let list = ListProjectsRequest {
        owner_id: None,
        page: None,
    };
    let mut client = app.projects.clone();

    // when
    let created = client
        .create_project(with_token(&read_only, create()))
        .await;
    let listed = client.list_projects(with_token(&read_only, list)).await;

    // then
    assert_code(created, Code::PermissionDenied);
    assert!(listed.is_ok());
}

/// Adds a block of Datalog to a token, with no key, as any holder can.
fn attenuate(token: &str, code: &str) -> String {
    let block = BlockBuilder::new().code(code).unwrap();
    UnverifiedBiscuit::from_base64(token)
        .unwrap()
        .append(block)
        .unwrap()
        .to_base64()
        .unwrap()
}
