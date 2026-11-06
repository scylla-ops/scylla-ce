//! The tests of the contract: an edition adds a kind, and its plugin records a fact.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use scylla_domain::identity::actors::ActorId;
use scylla_extension::{
    async_trait, Audience, AudienceContext, Collapse, Event, EventRow, NotificationError,
    NotificationKind, Notifier, Recipient,
};
use scylla_testkit::TestApp;
use sqlx::PgPool;

/// Sends a fact to the person of its object, and fails when asked.
#[derive(Default)]
struct ToPerson {
    /// `true` makes the next call fail.
    fail: AtomicBool,
}

#[async_trait]
impl Audience for ToPerson {
    async fn recipients(
        &self,
        _ctx: &AudienceContext<'_>,
        event: &EventRow,
    ) -> Result<Vec<Recipient>, NotificationError> {
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(NotificationError("no answer".to_owned()));
        }
        let user: Option<ActorId> = event.subject_id("user");
        Ok(user.map(Recipient::about).into_iter().collect())
    }
}

/// An edition with one more kind, and the notifier that its service got.
type WithKind = (TestApp, Arc<ToPerson>, Arc<dyn Notifier>);

/// Starts an edition with the kind `build.failed`, which counts the facts of a group.
async fn with_kind(pool: PgPool) -> WithKind {
    let audience = Arc::new(ToPerson::default());
    let kind =
        NotificationKind::new("build.failed", "access", audience.clone()).collapse(Collapse::Count);
    let holder: Arc<Mutex<Option<Arc<dyn Notifier>>>> = Arc::default();
    let keep = holder.clone();
    let edition = scylla_core::edition("test")
        .notification_kind(kind)
        .service(move |_routes, ctx| {
            *keep.lock().unwrap() = Some(ctx.notifier());
        })
        .build();
    let app = TestApp::spawn(pool, edition).await;
    let notifier = holder.lock().unwrap().clone();
    (app, audience, wait_for(notifier, &holder).await)
}

/// Returns the notifier of the service: the server adds its services after its start.
async fn wait_for(
    first: Option<Arc<dyn Notifier>>,
    holder: &Mutex<Option<Arc<dyn Notifier>>>,
) -> Arc<dyn Notifier> {
    let mut found = first;
    for _ in 0..100 {
        if let Some(notifier) = found {
            return notifier;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        found = holder.lock().unwrap().clone();
    }
    panic!("the server did not add its services");
}

/// Makes a fact of the kind about a person, in one group.
fn fact(user: ActorId) -> Event {
    Event::new("build.failed")
        .subject("user", user)
        .group("build:1")
}

/// Facts of one group make one row with a count, and a read row becomes unread.
#[sqlx::test(migrations = false)]
async fn three_facts_make_one_row_with_count_3(pool: PgPool) {
    // given
    let (app, _, notifier) = with_kind(pool).await;
    let user = app.user().await;
    notifier.emit_detached(fact(user)).await.unwrap();
    let first = app.inbox(user).await;
    let id = scylla_testkit::text(&first[0].id).to_owned();
    app.mark_notifications_read(user, &[&id], true)
        .await
        .unwrap();

    // when
    notifier.emit_detached(fact(user)).await.unwrap();
    notifier.emit_detached(fact(user)).await.unwrap();
    let inbox = app.inbox(user).await;

    // then
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].occurrences, 3);
    assert!(inbox[0].read_at.is_none());
    assert_eq!(inbox[0].id, first[0].id);
}

/// A second fact with one dedupe key is not recorded, in the transaction of a caller.
#[sqlx::test(migrations = false)]
async fn a_dedupe_key_stops_a_second_fact(pool: PgPool) {
    // given
    let (app, _, notifier) = with_kind(pool.clone()).await;
    let user = app.user().await;
    let mut tx = pool.begin().await.unwrap();

    // when
    let first = notifier
        .emit(&mut tx, fact(user).dedupe("one"))
        .await
        .unwrap();
    let second = notifier
        .emit(&mut tx, fact(user).dedupe("one"))
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let inbox = app.inbox(user).await;

    // then
    assert!(first);
    assert!(!second);
    assert_eq!(inbox[0].occurrences, 1);
}

/// An audience that fails delays the fact, and a later pass delivers it.
#[sqlx::test(migrations = false)]
async fn a_failing_audience_delays_the_event(pool: PgPool) {
    // given
    let (app, audience, notifier) = with_kind(pool.clone()).await;
    let user = app.user().await;
    notifier.emit_detached(fact(user)).await.unwrap();
    audience.fail.store(true, Ordering::SeqCst);

    // when
    let delayed = app.inbox(user).await.len();
    let waits =
        sqlx::query_scalar!("SELECT attempts FROM notification_events WHERE kind = 'build.failed'")
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query!("UPDATE notification_events SET next_attempt_at = now()")
        .execute(&pool)
        .await
        .unwrap();
    let delivered = app.inbox(user).await.len();

    // then
    assert_eq!(delayed, 0);
    assert_eq!(waits, 1);
    assert_eq!(delivered, 1);
}

/// A fact of a kind that the edition does not have goes out with no row, and blocks no other fact.
#[sqlx::test(migrations = false)]
async fn an_unknown_kind_is_dropped(pool: PgPool) {
    // given
    let (app, _, notifier) = with_kind(pool.clone()).await;
    let user = app.user().await;
    let unknown = Event::new("gone.away").subject("user", user);
    notifier.emit_detached(unknown).await.unwrap();
    notifier.emit_detached(fact(user)).await.unwrap();

    // when
    let inbox = app.inbox(user).await;

    // then
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].kind, "build.failed");
}

/// Parameters above the limit are refused.
#[sqlx::test(migrations = false)]
async fn large_parameters_are_refused(pool: PgPool) {
    // given
    let (app, _, notifier) = with_kind(pool).await;
    let user = app.user().await;
    let large = fact(user).param("text", "x".repeat(3000));

    // when
    let emitted = notifier.emit_detached(large).await;

    // then
    assert!(emitted.is_err());
}
