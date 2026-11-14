//! The tests of the webhooks of the GitHub App: the signature before any write, a ping, a
//! delivery kept one time, and the arrival of a webhook on a link in polling mode.

use serde_json::json;
use sqlx::PgPool;

use super::{app_with_fake, github_push, linked, post_app, post_github, GITHUB_ROUTE, SHA};

/// A delivery with a bad signature or of an unknown App is refused, and the server writes
/// nothing: no delivery, and no time of a webhook on the link or on its connection.
#[sqlx::test(migrations = false)]
async fn a_github_webhook_with_a_bad_signature_writes_nothing(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let push = github_push(SHA, "main");
    let unknown_app = "/integrations/github/00000000-0000-0000-0000-000000000000/webhook";

    // when
    let forged = post_github(&app, GITHUB_ROUTE, "push", "d-1", &push, "not the secret").await;
    let unknown = post_github(&app, unknown_app, "push", "d-2", &push, "x").await;
    let bad_route = "/integrations/github/7/webhook";
    let bad_id = post_github(&app, bad_route, "push", "d-3", &push, "x").await;

    // then
    assert_eq!((forged.0, unknown.0, bad_id.0), (401, 404, 404));
    assert!(app.deliveries().await.is_empty());
    assert!(!app.link_mode(&linked.project).await.has_event);
    assert!(!app.connection_mode(&linked.connection).await.has_event);
}

/// A ping gets 200 and is not kept. A push gets 202, and its work keeps the head of the branch.
/// A second delivery with the same id gets 200 and is ignored.
#[sqlx::test(migrations = false)]
async fn a_github_delivery_is_kept_one_time(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let push = github_push(SHA, "main");

    // when
    let ping = post_app(&app, "ping", "d-0", &json!({ "zen": "Keep it simple." })).await;
    let first = post_app(&app, "push", "d-1", &push).await;
    let second = post_app(&app, "push", "d-1", &push).await;
    let done = app.wait_for_delivery("d-1").await;

    // then
    assert_eq!((ping.0, first.0, second.0), (200, 202, 200));
    assert_eq!(second.1, json!({ "duplicate": true }));
    assert_eq!(app.deliveries().await.len(), 1);
    assert_eq!(
        (
            done.provider.as_str(),
            done.event.as_str(),
            done.error.as_str()
        ),
        ("github", "push", "")
    );
    let heads = app.poll_heads(&linked.project).await;
    assert_eq!(heads, vec![("refs/heads/main".to_owned(), SHA.to_owned())]);
    assert!(app.link_mode(&linked.project).await.has_event);
    assert!(app.connection_mode(&linked.connection).await.has_event);
}

/// A webhook of the repository moves a link that polls because the webhooks did not arrive back
/// to the webhook mode. A body that is not JSON gets 400.
#[sqlx::test(migrations = false)]
async fn a_github_webhook_ends_the_polling_mode_of_a_link(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    let reason = Some("WEBHOOKS_NOT_ARRIVING");
    app.set_link_mode(&linked.project, "polling", reason).await;

    // when
    let pushed = post_app(&app, "push", "d-1", &github_push(SHA, "main")).await;
    let broken = post_app(&app, "push", "d-2", &json!("not an event")).await;

    // then
    assert_eq!((pushed.0, broken.0), (202, 400));
    let mode = app.link_mode(&linked.project).await;
    assert_eq!(
        (mode.delivery_mode.as_str(), mode.reason.as_str()),
        ("webhook", "")
    );
}
