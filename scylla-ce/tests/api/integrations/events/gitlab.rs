//! The tests of the GitLab webhook of a linked project: the signing token and the time window
//! before any write, a delivery kept one time, and a merge request.

use serde_json::json;
use sqlx::PgPool;

use super::{app_with_fake, gitlab_push, linked, post_gitlab, SHA, SIGNING_TOKEN};

/// Another commit.
const OTHER: &str = "fedcba9876543210fedcba9876543210fedcba98";

/// A delivery signed with the signing token of the link gets 202, and its work keeps the head of
/// the branch. A second delivery with the same key gets 200. A bad token, an old delivery and an
/// unknown project write nothing.
#[sqlx::test(migrations = false)]
async fn a_gitlab_webhook_checks_its_signing_token(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    let push = gitlab_push(SHA, "main");
    let other = "whsec_YW5vdGhlciBrZXk=";
    let unknown = "00000000-0000-0000-0000-000000000000";

    // when
    let forged = post_gitlab(&app, &linked.project, "k-0", &push, other, 0).await;
    let old = post_gitlab(&app, &linked.project, "k-0", &push, SIGNING_TOKEN, 600).await;
    let nobody = post_gitlab(&app, unknown, "k-0", &push, SIGNING_TOKEN, 0).await;
    let written = app.deliveries().await;
    let first = post_gitlab(&app, &linked.project, "k-1", &push, SIGNING_TOKEN, 0).await;
    let second = post_gitlab(&app, &linked.project, "k-1", &push, SIGNING_TOKEN, 0).await;
    let done = app.wait_for_delivery("k-1").await;

    // then
    assert_eq!((forged.0, old.0, nobody.0), (401, 401, 404));
    assert!(written.is_empty());
    assert_eq!((first.0, second.0), (202, 200));
    assert_eq!(second.1, json!({ "duplicate": true }));
    assert_eq!(
        (
            done.provider.as_str(),
            done.event.as_str(),
            done.error.as_str()
        ),
        ("gitlab", "push", "")
    );
    let heads = app.poll_heads(&linked.project).await;
    assert_eq!(heads, vec![("refs/heads/main".to_owned(), SHA.to_owned())]);
    assert!(app.link_mode(&linked.project).await.has_event);
}

/// A merge request of the project is kept with its action. A delivery of another GitLab project
/// is kept, but it changes no head.
#[sqlx::test(migrations = false)]
async fn a_gitlab_merge_request_is_kept_with_its_action(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    let request = |project: u64, sha: &str| {
        json!({
            "object_kind": "merge_request",
            "user": { "username": "dev", "avatar_url": null },
            "project": { "id": project, "path_with_namespace": "group/app", "web_url": "" },
            "object_attributes": {
                "iid": 17, "title": "Add login", "url": "https://gitlab.com/group/app/-/merge_requests/17",
                "source_branch": "login", "target_branch": "main",
                "source_project_id": project, "target_project_id": project,
                "action": "open", "draft": false,
                "last_commit": { "id": sha, "message": "Add login" },
            },
        })
    };

    // when
    post_gitlab(
        &app,
        &linked.project,
        "k-1",
        &request(7, SHA),
        SIGNING_TOKEN,
        0,
    )
    .await;
    let done = app.wait_for_delivery("k-1").await;
    post_gitlab(
        &app,
        &linked.project,
        "k-2",
        &request(8, OTHER),
        SIGNING_TOKEN,
        0,
    )
    .await;
    app.wait_for_delivery("k-2").await;

    // then
    assert_eq!(
        (done.event.as_str(), done.action.as_str()),
        ("merge_request", "open")
    );
    let heads = app.poll_heads(&linked.project).await;
    let head = ("refs/merge-requests/17/head".to_owned(), SHA.to_owned());
    assert_eq!(heads, vec![head]);
}
