//! The tests of `SecretService`: the data, the rules of the names and of the values, the pages,
//! and no value in an answer.

use prost::Message;
use scylla_domain::projects::secrets::SecretId;
use scylla_proto::projects::v1::{
    CreateSecretResponse, ListSecretsResponse, Secret, UpdateSecretResponse,
};
use scylla_testkit::{text, TestApp};
use sqlx::PgPool;

use crate::identity::accounts::assert_refused;
use crate::{app, assert_invalid_field};

/// Opens the sealed value of a secret in the database with the key of the server.
async fn stored_value(app: &TestApp, secret_id: &str) -> String {
    let id = SecretId::parse(secret_id).unwrap();
    let sealed = sqlx::query_scalar!("SELECT sealed_value FROM secrets WHERE id = $1", id.to_db())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    let sealer = app.sealer.as_ref().unwrap();
    String::from_utf8(sealer.open_with(&sealed, &id.to_bytes()).unwrap()).unwrap()
}

/// Returns `true` if the bytes of a message hold the text.
fn holds(message: &impl Message, text: &str) -> bool {
    let bytes = message.encode_to_vec();
    bytes
        .windows(text.len())
        .any(|window| window == text.as_bytes())
}

/// A created secret comes back with its metadata, its name trimmed, and its value sealed.
#[sqlx::test(migrations = false)]
async fn create_secret_returns_the_metadata(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();

    // when
    let response = app
        .create_secret(user, text(&project.id), " DB_PASSWORD ", "s3cret value")
        .await
        .unwrap();

    // then
    let secret = response.secret.unwrap();
    assert_eq!(secret.name, "DB_PASSWORD");
    assert_eq!(text(&secret.project_id), text(&project.id));
    assert_eq!(text(&secret.created_by), user.to_string());
    assert_eq!(secret.created_at, secret.updated_at);
    assert_eq!(stored_value(&app, text(&secret.id)).await, "s3cret value");
}

/// No answer holds a value: the messages have no value field, and their bytes do not hold it.
#[sqlx::test(migrations = false)]
async fn no_answer_holds_the_value(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let project = text(&project.id);

    // when
    let created = app
        .create_secret(user, project, "TOKEN", "first-marker")
        .await
        .unwrap();
    let secret_id = text(&created.secret.as_ref().unwrap().id).to_owned();
    let updated = app
        .update_secret(user, &secret_id, "second-marker")
        .await
        .unwrap();
    let listed = app.list_secrets(user, project, 10, "").await.unwrap();

    // then
    let CreateSecretResponse { secret } = created.clone();
    let UpdateSecretResponse { secret: changed } = updated.clone();
    let ListSecretsResponse { secrets, page: _ } = listed.clone();
    for secret in [secret, changed].into_iter().flatten().chain(secrets) {
        let Secret {
            id: _,
            project_id: _,
            name: _,
            created_by: _,
            created_at: _,
            updated_at: _,
        } = secret;
    }
    for marker in ["first-marker", "second-marker"] {
        assert!(!holds(&created, marker) && !holds(&updated, marker));
        assert!(!holds(&listed, marker));
    }
}

/// A name has 1 to 128 letters, digits, dots, dashes or underscores.
#[sqlx::test(migrations = false)]
async fn create_secret_checks_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let project = text(&project.id);

    // when
    let longest = app
        .create_secret(user, project, &"x".repeat(128), "v")
        .await;
    let too_long = app
        .create_secret(user, project, &"x".repeat(129), "v")
        .await;
    let empty = app.create_secret(user, project, "  ", "v").await;
    let space = app.create_secret(user, project, "MY TOKEN", "v").await;
    let slash = app.create_secret(user, project, "a/b", "v").await;

    // then
    assert!(longest.is_ok());
    for refused in [too_long, empty, space, slash] {
        assert_invalid_field(&refused.unwrap_err(), "name");
    }
}

/// A name is unique in its project, in its exact case. Another project can use it.
#[sqlx::test(migrations = false)]
async fn create_secret_refuses_a_name_in_use(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let alpha = app.create_project(user, "alpha").await.unwrap();
    let beta = app.create_project(user, "beta").await.unwrap();
    app.create_secret(user, text(&alpha.id), "TOKEN", "one")
        .await
        .unwrap();

    // when
    let again = app
        .create_secret(user, text(&alpha.id), "TOKEN", "two")
        .await;
    let other_case = app
        .create_secret(user, text(&alpha.id), "token", "two")
        .await;
    let other_project = app
        .create_secret(user, text(&beta.id), "TOKEN", "two")
        .await;

    // then
    assert_refused(again, "SECRET_NAME_IN_USE");
    assert!(other_case.is_ok() && other_project.is_ok());
}

/// A value of 65536 bytes passes and 65537 fails, on a creation and on a change.
#[sqlx::test(migrations = false)]
async fn a_value_has_at_most_65536_bytes(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let project = text(&project.id);
    let (largest, too_large) = ("x".repeat(65_536), "x".repeat(65_537));

    // when
    let created = app.create_secret(user, project, "BIG", &largest).await;
    let refused = app.create_secret(user, project, "BIGGER", &too_large).await;
    let secret_id = text(&created.as_ref().unwrap().secret.as_ref().unwrap().id).to_owned();
    let changed = app.update_secret(user, &secret_id, &too_large).await;

    // then
    assert!(created.is_ok());
    assert_eq!(stored_value(&app, &secret_id).await, largest);
    assert_invalid_field(&refused.unwrap_err(), "value");
    assert_invalid_field(&changed.unwrap_err(), "value");
}

/// A change gives a new value and a new `updated_at`. The name and the creation time stay.
#[sqlx::test(migrations = false)]
async fn update_secret_changes_the_value(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let created = app
        .create_secret(user, text(&project.id), "TOKEN", "old")
        .await
        .unwrap()
        .secret
        .unwrap();

    // when
    let changed = app
        .update_secret(user, text(&created.id), "new")
        .await
        .unwrap()
        .secret
        .unwrap();

    // then
    assert_eq!(
        (changed.name.as_str(), changed.created_at),
        ("TOKEN", created.created_at)
    );
    let (before, after) = (created.updated_at.unwrap(), changed.updated_at.unwrap());
    assert!((after.seconds, after.nanos) > (before.seconds, before.nanos));
    assert_eq!(stored_value(&app, text(&created.id)).await, "new");
}

/// A deleted secret leaves the list, and its name is free again, also if a pipeline names it.
#[sqlx::test(migrations = false)]
async fn delete_secret_frees_the_name(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let project = text(&project.id);
    let secret = app
        .create_secret(user, project, "TOKEN", "old")
        .await
        .unwrap()
        .secret
        .unwrap();

    // when
    app.delete_secret(user, text(&secret.id)).await.unwrap();
    let names = app.secret_names(user, project).await.unwrap();
    let again = app.create_secret(user, project, "TOKEN", "new").await;

    // then
    assert!(names.is_empty(), "{names:?}");
    assert!(again.is_ok());
}

/// The pages give each secret one time, in byte order of the names, and the last page has no
/// token. A token that is not a name is refused.
#[sqlx::test(migrations = false)]
async fn list_secrets_returns_pages_in_name_order(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    let project = text(&project.id);
    for name in ["b", "_x", "B", "c.1", "a", "-y", "A"] {
        app.create_secret(user, project, name, "v").await.unwrap();
    }

    // when
    let (mut listed, mut sizes, mut token) = (Vec::new(), Vec::new(), String::new());
    loop {
        let page = app.list_secrets(user, project, 3, &token).await.unwrap();
        sizes.push(page.secrets.len());
        listed.extend(page.secrets.into_iter().map(|secret| secret.name));
        token = page.page.unwrap().next_token;
        if token.is_empty() {
            break;
        }
    }
    let bad_token = app.list_secrets(user, project, 3, "not a name").await;

    // then
    assert_eq!(sizes, vec![3, 3, 1]);
    assert_eq!(listed, ["-y", "A", "B", "_x", "a", "b", "c.1"]);
    assert_invalid_field(&bad_token.unwrap_err(), "page.token");
}

/// A deleted project takes its secrets with it.
#[sqlx::test(migrations = false)]
async fn delete_project_deletes_its_secrets(pool: PgPool) {
    // given
    let app = app(pool).await;
    let user = app.user().await;
    let project = app.create_project(user, "alpha").await.unwrap();
    app.create_secret(user, text(&project.id), "TOKEN", "v")
        .await
        .unwrap();

    // when
    app.delete_project(user, text(&project.id)).await.unwrap();

    // then
    let count = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM secrets"#)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}
