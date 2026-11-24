//! The pages of the feed, the count of the rows since a time, and the bad requests.

use scylla_proto::activity::v1::ListActivityRequest;
use scylla_proto::common::v1::PageRequest;
use scylla_testkit::id;
use sqlx::PgPool;

use super::place::Place;
use crate::{app, assert_invalid_field};

/// Returns a request of the first page of a workspace.
fn request(workspace: &str) -> ListActivityRequest {
    ListActivityRequest {
        workspace_id: Some(id(workspace)),
        ..ListActivityRequest::default()
    }
}

/// Returns a page request.
fn page(size: i32, token: &str) -> Option<PageRequest> {
    Some(PageRequest {
        size,
        token: token.to_owned(),
    })
}

/// The pages give each row one time, newest first. `since_count` counts the rows at a time or
/// later, and is 0 with no time.
#[sqlx::test(migrations = false)]
async fn list_activity_gives_pages_and_a_count(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = Place::new(&app).await;
    for name in ["A", "B", "C"] {
        place.secret(&app, name).await;
    }
    let owner = place.owner;

    // when
    let first = ListActivityRequest {
        page: page(2, ""),
        ..request(&place.workspace)
    };
    let first = app.list_activity(owner, first).await.unwrap();
    let token = first.page.clone().unwrap().next_token;
    let second = ListActivityRequest {
        page: page(2, &token),
        ..request(&place.workspace)
    };
    let second = app.list_activity(owner, second).await.unwrap();
    let since = ListActivityRequest {
        since: first.activities[1].at,
        ..request(&place.workspace)
    };
    let counted = app.list_activity(owner, since).await.unwrap();

    // then
    let names = |rows: &[scylla_proto::activity::v1::Activity]| -> Vec<String> {
        rows.iter().map(|row| row.params["name"].clone()).collect()
    };
    assert_eq!(names(&first.activities), ["C", "B"]);
    assert_eq!(names(&second.activities), ["A"]);
    assert!(second.page.unwrap().next_token.is_empty());
    assert_eq!((first.since_count, counted.since_count), (0, 2));
    assert_eq!(counted.activities.len(), 3);
}

/// No workspace, a negative page size and a bad page token are request errors.
#[sqlx::test(migrations = false)]
async fn list_activity_refuses_bad_values(pool: PgPool) {
    // given
    let app = app(pool).await;
    let place = Place::new(&app).await;
    let owner = place.owner;
    let negative = ListActivityRequest {
        page: page(-1, ""),
        ..request(&place.workspace)
    };
    let bad_token = ListActivityRequest {
        page: page(2, "not a token"),
        ..request(&place.workspace)
    };

    // when
    let no_workspace = app
        .list_activity(owner, ListActivityRequest::default())
        .await;
    let negative = app.list_activity(owner, negative).await;
    let bad_token = app.list_activity(owner, bad_token).await;

    // then
    assert_invalid_field(&no_workspace.unwrap_err(), "workspace_id");
    assert_invalid_field(&negative.unwrap_err(), "page.size");
    assert_invalid_field(&bad_token.unwrap_err(), "page.token");
}
