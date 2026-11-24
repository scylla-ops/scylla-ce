//! The tests of the link of a GitLab project: the webhook of a Maintainer with its sealed
//! signing token, the polling mode of a Developer, and the delete of the webhook at the unlink.

use scylla_proto::integrations::v1::{ConnectionState, DeliveryMode, VcsProvider};
use serde_json::{json, Value};
use sqlx::PgPool;

use super::{app_with_fake, assert_reason, project, GITLAB_TOKEN};

/// Story 8.2, step 3: the link of a project where the service account is Maintainer makes the
/// webhook of the project for the pushes, the tags and the merge requests, signed with a new
/// signing token. The server keeps the token sealed with the id of the project.
#[sqlx::test(migrations = false)]
async fn link_repository_makes_the_gitlab_webhook(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "gitlab").await;
    fake.answer_gitlab_project(7, "bank/payments", 40);
    fake.answer_gitlab_hook(7, 555);

    // when
    let linked = app
        .link_repository(project.owner, &project.id, &project.connection, "7")
        .await
        .unwrap();

    // then
    assert_eq!(linked.provider, VcsProvider::Gitlab as i32);
    assert_eq!(linked.full_name, "bank/payments");
    assert_eq!(linked.delivery_mode, DeliveryMode::Webhook as i32);
    assert_eq!(linked.state, ConnectionState::Active as i32);
    assert!(linked.state_reason.is_empty());
    let made = &fake.requests_to("POST", "/api/v4/projects/7/hooks")[0];
    assert_eq!(made.header("private-token"), Some(GITLAB_TOKEN));
    let url = made.body["url"].as_str().unwrap_or_default();
    let route = format!("/integrations/gitlab/{}/webhook", project.id);
    assert!(url.ends_with(&route), "{url}");
    for event in ["push_events", "tag_push_events", "merge_requests_events"] {
        assert_eq!(made.body[event], Value::Bool(true), "{event}");
    }
    let signing = made.body["signing_token"].as_str().unwrap_or_default();
    assert!(signing.starts_with("whsec_"), "{signing}");
    let (hook, sealed) = app.hook_of(&project.id).await.expect("a webhook");
    assert_eq!(hook, "555");
    let id = uuid::Uuid::parse_str(&project.id).unwrap();
    let sealer = app.sealer.as_ref().expect("a key");
    let opened = sealer.open_with(&sealed, id.as_bytes()).unwrap();
    assert_eq!(String::from_utf8(opened).unwrap(), signing);
    assert!(!format!("{linked:?}").contains(signing));
}

/// A service account that is only Developer cannot make the webhook: the link polls, with the
/// reason `ROLE_TOO_LOW`, and the server asks GitLab for no webhook.
#[sqlx::test(migrations = false)]
async fn link_repository_polls_for_a_gitlab_developer(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "gitlab").await;
    fake.answer_gitlab_project(7, "bank/payments", 30);

    // when
    let linked = app
        .link_repository(project.owner, &project.id, &project.connection, "7")
        .await
        .unwrap();

    // then
    assert_eq!(linked.delivery_mode, DeliveryMode::Polling as i32);
    assert_eq!(linked.state, ConnectionState::Active as i32);
    assert_eq!(linked.state_reason, "ROLE_TOO_LOW");
    assert!(fake
        .requests_to("POST", "/api/v4/projects/7/hooks")
        .is_empty());
    assert!(app.hook_of(&project.id).await.is_none());
}

/// GitLab refuses the webhook of a Maintainer for a URL that it cannot call: the link polls,
/// with no reason. A refused role gives `ROLE_TOO_LOW`.
#[sqlx::test(migrations = false)]
async fn link_repository_polls_when_gitlab_refuses_the_webhook(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let first = project(&app, "gitlab").await;
    let second = project(&app, "gitlab").await;
    fake.answer_gitlab_project(7, "bank/payments", 40);
    let blocked = json!({ "message": "Invalid url given" });
    fake.respond("POST", "/api/v4/projects/7/hooks", 422, blocked, &[]);
    let forbidden = json!({ "message": "403 Forbidden" });
    fake.respond("POST", "/api/v4/projects/7/hooks", 403, forbidden, &[]);

    // when
    let blocked = app
        .link_repository(first.owner, &first.id, &first.connection, "7")
        .await
        .unwrap();
    let refused = app
        .link_repository(second.owner, &second.id, &second.connection, "7")
        .await
        .unwrap();

    // then
    assert_eq!(blocked.delivery_mode, DeliveryMode::Polling as i32);
    assert!(blocked.state_reason.is_empty());
    assert_eq!(refused.delivery_mode, DeliveryMode::Polling as i32);
    assert_eq!(refused.state_reason, "ROLE_TOO_LOW");
}

/// A project that the service account does not see, or where it is less than Developer, gives
/// `REPOSITORY_NOT_VISIBLE`, and no link.
#[sqlx::test(migrations = false)]
async fn link_repository_refuses_a_gitlab_project_with_no_access(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "gitlab").await;
    fake.answer_gitlab_project(8, "bank/ledger", 20);

    // when
    let unknown = app
        .link_repository(project.owner, &project.id, &project.connection, "7")
        .await;
    let reporter = app
        .link_repository(project.owner, &project.id, &project.connection, "8")
        .await;

    // then
    assert_reason(unknown, "REPOSITORY_NOT_VISIBLE");
    assert_reason(reporter, "REPOSITORY_NOT_VISIBLE");
    let read = app.get_repository_link(project.owner, &project.id).await;
    assert_eq!(read.unwrap(), None);
}

/// An unlink deletes the link and the webhook that Scylla made. The project can link again, and
/// a second unlink gives `NO_REPOSITORY_LINK`.
#[sqlx::test(migrations = false)]
async fn unlink_repository_deletes_the_gitlab_webhook(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let project = project(&app, "gitlab").await;
    fake.answer_gitlab_project(7, "bank/payments", 40);
    fake.answer_gitlab_hook(7, 555);
    app.link_repository(project.owner, &project.id, &project.connection, "7")
        .await
        .unwrap();

    // when
    app.unlink_repository(project.owner, &project.id)
        .await
        .unwrap();
    let read = app.get_repository_link(project.owner, &project.id).await;
    let again = app.unlink_repository(project.owner, &project.id).await;
    let relinked = app
        .link_repository(project.owner, &project.id, &project.connection, "7")
        .await;

    // then
    assert_eq!(read.unwrap(), None);
    assert_reason(again, "NO_REPOSITORY_LINK");
    assert!(relinked.is_ok());
    let deleted = fake.requests_to("DELETE", "/api/v4/projects/7/hooks/555");
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].header("private-token"), Some(GITLAB_TOKEN));
}
