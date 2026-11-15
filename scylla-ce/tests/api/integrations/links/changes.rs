//! The tests of the changes of a link: the reports and the time between two polls, with their
//! bounds.

use scylla_proto::integrations::v1::UpdateRepositoryLinkRequest;
use sqlx::PgPool;

use super::{app_with_fake, assert_reason, insert_link, project};
use crate::assert_invalid_field;

/// An update changes the fields that it sets, and keeps the others.
#[sqlx::test(migrations = false)]
async fn update_repository_link_changes_the_fields_that_it_sets(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let project = project(&app, "github").await;
    insert_link(&app, &project, "octo/app").await;
    let changes = UpdateRepositoryLinkRequest {
        reports_enabled: Some(false),
        poll_interval_seconds: Some(900),
        ..UpdateRepositoryLinkRequest::default()
    };
    let one = UpdateRepositoryLinkRequest {
        poll_interval_seconds: Some(60),
        ..UpdateRepositoryLinkRequest::default()
    };

    // when
    let changed = app
        .update_repository_link(project.owner, &project.id, changes)
        .await
        .unwrap();
    let kept = app
        .update_repository_link(project.owner, &project.id, one)
        .await
        .unwrap();

    // then
    assert!(!changed.reports_enabled);
    assert_eq!(changed.poll_interval_seconds, 900);
    assert!(!kept.reports_enabled);
    assert_eq!(kept.poll_interval_seconds, 60);
    let read = app.get_repository_link(project.owner, &project.id).await;
    assert_eq!(read.unwrap(), Some(kept));
}

/// A time between two polls out of 60 to 900 seconds is a bad request. A project with no link
/// gives `NO_REPOSITORY_LINK`.
#[sqlx::test(migrations = false)]
async fn update_repository_link_checks_the_request(pool: PgPool) {
    // given
    let (app, _fake) = app_with_fake(pool).await;
    let gitlab = project(&app, "gitlab").await;
    insert_link(&app, &gitlab, "group/app").await;
    let unlinked = project(&app, "github").await;
    let poll = |seconds| UpdateRepositoryLinkRequest {
        poll_interval_seconds: Some(seconds),
        ..UpdateRepositoryLinkRequest::default()
    };
    let update = |changes| app.update_repository_link(gitlab.owner, &gitlab.id, changes);

    // when
    let short = update(poll(59)).await.unwrap_err();
    let long = update(poll(901)).await.unwrap_err();
    let none = app
        .update_repository_link(unlinked.owner, &unlinked.id, poll(120))
        .await;

    // then
    assert_invalid_field(&short, "poll_interval_seconds");
    assert_invalid_field(&long, "poll_interval_seconds");
    assert_reason(none, "NO_REPOSITORY_LINK");
}
