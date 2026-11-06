//! The tests of the delivery: a fact stays until the task reads it, and it makes one row.

use std::sync::Arc;

use scylla_domain::tenancy::invitations;
use scylla_extension::{async_trait, BeforeCreate, CreateContext, HookError, Rejection, Tx};
use scylla_testkit::TestApp;
use sqlx::PgPool;

use super::kinds_of;
use crate::app;
use crate::identity::accounts::PASSWORD;

/// Returns the number of facts that wait.
async fn pending(pool: &PgPool) -> i64 {
    sqlx::query_scalar!(
        r#"SELECT count(*) AS "count!" FROM notification_events WHERE dispatched_at IS NULL"#
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A user with the verified email of an invitation gets one row, with the live invitation.
#[sqlx::test(migrations = false)]
async fn an_invitation_reaches_a_user_with_this_verified_email(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "ana@example.com";
    let ana = app.user_with(email, None).await;

    // when
    app.create_invitation(owner, workspace, email, "member")
        .await
        .unwrap();
    let inbox = app.inbox(ana).await;

    // then
    assert_eq!(inbox.len(), 1);
    let row = &inbox[0];
    assert_eq!(row.kind, "invitation.received");
    assert_eq!(row.reason, "subject");
    assert!(row.needs_action);
    assert_eq!(row.actor_name, "Test User");
    assert_eq!(row.category, "access");
    assert_eq!(row.params.get("role").map(String::as_str), Some("member"));
    assert_eq!(row.workspace_name, "Personal");
    let invitation = row.invitation.as_ref().expect("the live invitation");
    assert_eq!(invitation.role, "member");
    assert!(invitation.state.is_some());
}

/// A person that verifies its email after the invitation gets the row at the verification, and an
/// unverified email gets no row before it.
#[sqlx::test(migrations = false)]
async fn an_invitation_reaches_a_user_who_verifies_later(pool: PgPool) {
    // given
    let app = crate::identity::accounts::app_in(pool.clone(), scylla_core::SignUpMode::Open).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "late@example.com";
    app.sign_up(email, PASSWORD, "").await.unwrap();
    let late = user_of(&pool, email).await;
    app.create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();

    // when
    let before = app.inbox(late).await.len();
    let link = app.mails.token(email, "verify-email");
    app.verify_email(&link).await.unwrap();
    let after = kinds_of(&app, late).await;

    // then
    assert_eq!(before, 0);
    assert_eq!(after, ["invitation.received"]);
}

/// A new user with the verified email gets the row of an invitation that came before the account.
#[sqlx::test(migrations = false)]
async fn a_new_user_gets_the_row_of_an_older_invitation(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "new@example.com";
    app.create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();

    // when
    let new = app.user_with(email, None).await;

    // then
    assert_eq!(kinds_of(&app, new).await, ["invitation.received"]);
}

/// A fact stays in its table when no task reads it, and the task of a new server reads it.
#[sqlx::test(migrations = false)]
async fn an_event_survives_a_restart(pool: PgPool) {
    // given
    let first = app(pool.clone()).await;
    let owner = first.user().await;
    let workspace = first.workspace_of(owner).await;
    let email = "restart@example.com";
    let user = first.user_with(email, None).await;
    first
        .create_invitation(owner, workspace, email, "viewer")
        .await
        .unwrap();
    let waiting = pending(&pool).await;

    // when
    let second = app(pool.clone()).await;
    let kinds = kinds_of(&second, user).await;

    // then
    assert!(waiting >= 1, "{waiting}");
    assert_eq!(kinds, ["invitation.received"]);
    assert_eq!(pending(&pool).await, 0);
}

/// A hook that refuses.
struct Refuse;

#[async_trait]
impl BeforeCreate for Refuse {
    async fn before_create(&self, _tx: &mut Tx<'_>, _ctx: &CreateContext) -> Result<(), HookError> {
        Err(Rejection::new("REFUSED", "no").into())
    }
}

/// A refused write leaves no fact: the fact is in the transaction of the write.
#[sqlx::test(migrations = false)]
async fn a_rollback_leaves_no_event(pool: PgPool) {
    // given
    let edition = scylla_core::edition("test")
        .before_create(invitations::KIND, Arc::new(Refuse))
        .build();
    let app = TestApp::spawn(pool.clone(), edition).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;

    // when
    let created = app
        .create_invitation(owner, workspace, "no@example.com", "viewer")
        .await;

    // then
    assert!(created.is_err());
    assert_eq!(pending(&pool).await, 0);
}

/// Two facts of one group, and a second pass, make one row.
#[sqlx::test(migrations = false)]
async fn the_same_fact_makes_one_row(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let email = "one@example.com";
    let invitation = app.create_invitation(owner, workspace, email, "viewer");
    invitation.await.unwrap();
    let user = app.user_with(email, None).await;

    // when
    let first = app.inbox(user).await;
    let second = app.inbox(user).await;

    // then
    assert_eq!(first.len(), 1);
    assert_eq!(second.len(), 1);
    assert_eq!(first[0].id, second[0].id);
}

/// Returns the user of an email.
async fn user_of(pool: &PgPool, email: &str) -> scylla_domain::identity::actors::ActorId {
    let id = sqlx::query_scalar!("SELECT user_id FROM user_emails WHERE email = $1", email)
        .fetch_one(pool)
        .await
        .unwrap();
    scylla_domain::identity::actors::ActorId::from_db(id)
}
