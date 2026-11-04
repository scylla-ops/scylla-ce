//! The tests of the Apps: creation with a join code, the join, the exchange of a signed assertion
//! for a token, the keys, the reads and the delete.

use scylla_domain::tenancy::workspaces::Workspace;
use scylla_proto::projects::v1::ListProjectsRequest;
use scylla_testkit::{new_key, text, with_token, TestApp};
use sqlx::PgPool;
use tonic::{Code, Status};

use crate::{app, assert_code, assert_invalid_field, UNKNOWN_ID};

/// Returns `Ok` if a token can call a route that each actor can call.
async fn try_token(app: &TestApp, token: &str) -> Result<(), Status> {
    let list = ListProjectsRequest {
        workspace_id: None,
        page: None,
    };
    app.projects
        .clone()
        .list_projects(with_token(token, list))
        .await
        .map(|_| ())
}

/// An App joins with its code and its own key pair, then signs an assertion and gets a token.
#[sqlx::test(migrations = false)]
async fn join_app_gives_a_key_that_signs_in(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();

    // when
    let key = app.join_app(&created.join_code).await.unwrap();
    let token = app.exchange(key.assertion(60)).await.unwrap();

    // then
    assert!(try_token(&app, &token).await.is_ok());
    let read = app.get_app(owner, &key.app).await.unwrap();
    let keys: Vec<_> = read.keys.iter().map(|key| text(&key.id)).collect();
    assert_eq!(keys, vec![key.key_id.as_str()]);
    let public_key = key.key.verifying_key().to_bytes().to_vec();
    assert_eq!(read.keys[0].public_key, public_key);
    let created = created.app.unwrap();
    assert!(created.runner && !created.connected);
    assert_eq!(text(&created.workspace_id), workspace.to_string());
}

/// A code works one time, a new code stops the open one, and an unknown code is refused.
#[sqlx::test(migrations = false)]
async fn join_app_takes_a_code_one_time(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, true).await.unwrap();
    let id = text(&created.app.unwrap().id).to_owned();
    let first = app.join_app(&created.join_code).await;
    let stopped = app.create_join_code(owner, &id).await.unwrap();
    let new = app.create_join_code(owner, &id).await.unwrap();

    // when
    let again = app.join_app(&created.join_code).await;
    let replaced = app.join_app(&stopped).await;
    let unknown = app.join_app("not-a-code").await;
    let fresh = app.join_app(&new).await;

    // then
    assert!(first.is_ok() && fresh.is_ok());
    assert_code(again, Code::Unauthenticated);
    assert_code(replaced, Code::Unauthenticated);
    assert_code(unknown, Code::Unauthenticated);
    assert_eq!(app.get_app(owner, &id).await.unwrap().keys.len(), 2);
}

/// The owners and the admins of a workspace create its Apps. A member and another user cannot.
#[sqlx::test(migrations = false)]
async fn create_app_needs_an_admin_of_the_workspace(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let (admin, member, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    let roles = [
        Workspace::ADMIN.of(workspace, admin),
        Workspace::MEMBER.of(workspace, member),
    ];
    app.grant(&roles).await;

    // when
    let by_owner = app.create_app(owner, workspace, true).await;
    let by_admin = app.create_app(admin, workspace, false).await;
    let by_member = app.create_app(member, workspace, true).await;
    let by_stranger = app.create_app(stranger, workspace, true).await;

    // then
    assert!(by_owner.is_ok() && by_admin.is_ok());
    assert_code(by_member, Code::PermissionDenied);
    assert_code(by_stranger, Code::PermissionDenied);
}

/// The readers of a workspace read and list its Apps. A stranger and an unknown App get
/// `PERMISSION_DENIED`. The list filters the agents and gives pages.
#[sqlx::test(migrations = false)]
async fn get_app_and_list_apps_need_a_reader(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, viewer, stranger) = (app.user().await, app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::VIEWER.of(workspace, viewer)]).await;
    let agent = app.create_app(owner, workspace, true).await.unwrap();
    app.create_app(owner, workspace, false).await.unwrap();
    app.create_app(owner, workspace, true).await.unwrap();
    let id = text(&agent.app.unwrap().id).to_owned();

    // when
    let read = app.get_app(viewer, &id).await;
    let hidden = app.get_app(stranger, &id).await;
    let unknown = app.get_app(owner, UNKNOWN_ID).await;
    let agents = app
        .list_apps(viewer, workspace, Some(true), 0, "")
        .await
        .unwrap();
    let first = app.list_apps(viewer, workspace, None, 2, "").await.unwrap();
    let token = first.page.clone().unwrap().next_token;
    let second = app
        .list_apps(viewer, workspace, None, 2, &token)
        .await
        .unwrap();
    let refused = app.list_apps(stranger, workspace, None, 0, "").await;

    // then
    assert!(read.is_ok());
    assert_code(hidden, Code::PermissionDenied);
    assert_code(unknown, Code::PermissionDenied);
    assert_eq!(agents.apps.len(), 2);
    assert!(agents.apps.iter().all(|app| app.runner));
    assert_eq!((first.apps.len(), second.apps.len()), (2, 1));
    assert_eq!(text(&first.apps[0].id), id);
    assert_code(refused, Code::PermissionDenied);
}

/// Only the admins of the workspace change the keys and the codes of an App, and delete it.
#[sqlx::test(migrations = false)]
async fn the_key_routes_need_an_admin(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (owner, member) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(owner).await;
    app.grant(&[Workspace::MEMBER.of(workspace, member)]).await;
    let (key, _) = app.app_in(owner, workspace, true).await;

    // when
    let code = app.create_join_code(member, &key.app).await;
    let added = app.add_app_key(member, &key.app, &new_key(), "").await;
    let revoked = app.revoke_app_key(member, &key.app, &key.key_id).await;
    let deleted = app.delete_app(member, &key.app).await;

    // then
    assert_code(code, Code::PermissionDenied);
    assert_code(added, Code::PermissionDenied);
    assert_code(revoked, Code::PermissionDenied);
    assert_code(deleted, Code::PermissionDenied);
}

/// A revoked key stops its tokens at once and signs no new assertion. Another key of the App
/// still works.
#[sqlx::test(migrations = false)]
async fn revoke_app_key_stops_its_tokens(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let (key, token) = app.app_in(owner, workspace, true).await;
    let spare = new_key();
    let added = app.add_app_key(owner, &key.app, &spare, "").await.unwrap();
    let rotated = scylla_testkit::TestKey {
        app: key.app.clone(),
        key_id: text(&added.id).to_owned(),
        key: spare,
    };

    // when
    app.revoke_app_key(owner, &key.app, &key.key_id)
        .await
        .unwrap();

    // then
    assert_code(try_token(&app, &token).await, Code::Unauthenticated);
    assert_code(app.exchange(key.assertion(60)).await, Code::Unauthenticated);
    let new = app.exchange(rotated.assertion(60)).await.unwrap();
    assert!(try_token(&app, &new).await.is_ok());
    let keys = app.get_app(owner, &key.app).await.unwrap().keys;
    assert!(keys[0].revoked_at.is_some() && keys[1].revoked_at.is_none());
}

/// A deleted App loses its tokens and its roles, and its workspace can go.
#[sqlx::test(migrations = false)]
async fn delete_app_stops_its_tokens_and_its_roles(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.create_workspace(owner, "machines").await.unwrap();
    let workspace =
        scylla_domain::tenancy::workspaces::WorkspaceId::parse(text(&workspace.id)).unwrap();
    let (key, token) = app.app_in(owner, workspace, true).await;
    app.grant(&[Workspace::MEMBER.of(workspace, key.actor())])
        .await;

    // when
    app.delete_app(owner, &key.app).await.unwrap();

    // then
    assert_code(try_token(&app, &token).await, Code::Unauthenticated);
    assert_code(app.get_app(owner, &key.app).await, Code::PermissionDenied);
    let id = workspace.to_string();
    let access = app.access_of(owner, "workspace", &id, 0).await.unwrap();
    assert!(access.iter().all(|(actor, _)| *actor != key.app));
    assert!(app.delete_workspace(owner, &id).await.is_ok());
}

/// A replayed, expired, too long, or wrongly signed assertion is refused.
#[sqlx::test(migrations = false)]
async fn exchange_app_assertion_refuses_a_bad_assertion(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let (key, _) = app.app_in(user, workspace, true).await;
    let (other, _) = app.app_in(user, workspace, true).await;
    let replayed = key.assertion(60);
    app.exchange(replayed.clone()).await.unwrap();
    let mut forged = key.assertion(60);
    forged.signature = other.assertion(60).signature;

    // when
    let replay = app.exchange(replayed).await;
    let expired = app.exchange(key.assertion(-1)).await;
    let too_long = app.exchange(key.assertion(3600)).await;
    let short_nonce = app.exchange(key.assertion_with(60, "short")).await;
    let wrong_key = app.exchange(forged).await;

    // then
    assert_code(replay, Code::Unauthenticated);
    assert_code(expired, Code::Unauthenticated);
    assert_code(too_long, Code::Unauthenticated);
    assert_code(short_nonce, Code::Unauthenticated);
    assert_code(wrong_key, Code::Unauthenticated);
}

/// A key that is not an Ed25519 public key is refused.
#[sqlx::test(migrations = false)]
async fn join_app_checks_the_public_key(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let workspace = app.workspace_of(user).await;
    let created = app.create_app(user, workspace, true).await.unwrap();
    let request = scylla_proto::auth::v1::JoinAppRequest {
        join_code: created.join_code,
        public_key: vec![1, 2, 3],
        label: String::new(),
    };

    // when
    let result =
        scylla_proto::auth::v1::token_service_client::TokenServiceClient::new(app.channel.clone())
            .join_app(request)
            .await;

    // then
    assert_invalid_field(&result.unwrap_err(), "public_key");
}
