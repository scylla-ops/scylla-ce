//! The tests of `TestDelivery`: the provider sends a delivery and the link goes back to the
//! webhook mode, no delivery arrives and the link polls, and a GitLab link with no webhook.

use scylla_proto::integrations::v1::DeliveryMode;
use scylla_testkit::wait_for_requests;
use sqlx::PgPool;

use super::{
    app_with_fake, github_push, gitlab_push, linked, post_app, post_gitlab, SHA, SIGNING_TOKEN,
};

/// GitLab sends the test push of the webhook: the test tells that it arrived, and a link that
/// polls because GitLab disabled the webhook goes back to the webhook mode.
#[sqlx::test(migrations = false)]
async fn test_delivery_waits_for_the_gitlab_test_push(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    let reason = Some("WEBHOOK_DISABLED");
    app.set_link_mode(&linked.project, "polling", reason).await;
    fake.answer_gitlab_hook_test(7, 9);
    let route = "/api/v4/projects/7/hooks/9/test/push_events";

    // when
    let gitlab = async {
        wait_for_requests(&fake, "POST", route, 1).await;
        let push = gitlab_push(SHA, "main");
        post_gitlab(&app, &linked.project, "k-1", &push, SIGNING_TOKEN, 0).await
    };
    let (tested, sent) = tokio::join!(app.test_delivery(linked.owner, &linked.project), gitlab);

    // then
    let tested = tested.unwrap();
    assert!(tested.delivered, "{}", tested.error);
    assert_eq!(sent.0, 202);
    let link = tested.link.unwrap();
    assert_eq!(link.delivery_mode(), DeliveryMode::Webhook);
    assert!(link.state_reason.is_empty());
    let request = &fake.requests_to("POST", route)[0];
    assert_eq!(request.header("private-token"), Some("glpat-events"));
}

/// GitHub sends again the last delivery of the installation of the link, not the one of another
/// installation. A delivery that the server had gets 200, and the test still tells that it
/// arrived.
#[sqlx::test(migrations = false)]
async fn test_delivery_sends_a_github_delivery_again(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    post_app(&app, "push", "guid-4", &github_push(SHA, "main")).await;
    app.wait_for_delivery("guid-4").await;
    fake.answer_github_deliveries(&[(5, 99), (4, 42)]);
    let route = "/app/hook/deliveries/4/attempts";

    // when
    let github = async {
        wait_for_requests(&fake, "POST", route, 1).await;
        post_app(&app, "push", "guid-4", &github_push(SHA, "main")).await
    };
    let (tested, sent) = tokio::join!(app.test_delivery(linked.owner, &linked.project), github);

    // then
    let tested = tested.unwrap();
    assert!(tested.delivered, "{}", tested.error);
    assert_eq!(sent.0, 200);
    assert!(fake
        .requests_to("POST", "/app/hook/deliveries/5/attempts")
        .is_empty());
    let listed = &fake.requests_to("GET", "/app/hook/deliveries")[0];
    assert!(listed
        .header("authorization")
        .is_some_and(|value| value.starts_with("Bearer ")));
}

/// No delivery arrives within 10 seconds: the test tells why, and the link and its GitHub
/// connection move to the polling mode with `WEBHOOKS_NOT_ARRIVING`.
#[sqlx::test(migrations = false)]
async fn test_delivery_moves_a_silent_link_to_polling(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "github").await;
    fake.answer_github_deliveries(&[(4, 42)]);

    // when
    let tested = app.test_delivery(linked.owner, &linked.project).await;

    // then
    let tested = tested.unwrap();
    assert!(!tested.delivered);
    assert!(
        tested.error.contains("cannot reach this server"),
        "{}",
        tested.error
    );
    let link = tested.link.unwrap();
    assert_eq!(link.delivery_mode(), DeliveryMode::Polling);
    assert_eq!(link.state_reason, "WEBHOOKS_NOT_ARRIVING");
    let connection = app.connection_mode(&linked.connection).await;
    assert_eq!(connection.delivery_mode, "polling");
}

/// A GitLab link with no webhook cannot test it: the test tells why, with no call to GitLab.
#[sqlx::test(migrations = false)]
async fn test_delivery_needs_a_gitlab_webhook(pool: PgPool) {
    // given
    let (app, fake) = app_with_fake(pool).await;
    let linked = linked(&app, "gitlab").await;
    app.set_link_hook_off(&linked.project).await;

    // when
    let tested = app.test_delivery(linked.owner, &linked.project).await;

    // then
    let tested = tested.unwrap();
    assert!(!tested.delivered);
    assert!(
        tested.error.starts_with("the project has no webhook"),
        "{}",
        tested.error
    );
    assert!(fake.requests().is_empty());
}
