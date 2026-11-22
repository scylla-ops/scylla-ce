//! The tests of the retention: a pass deletes the closed sessions, the used or ended links of the
//! mails, the ended invitations and the proof of the terms of the deleted accounts after their time
//! in the settings, and keeps the others.

use scylla_core::features::platform::retention::{self, RetentionSettings};
use scylla_domain::identity::actors::ActorId;
use scylla_proto::common::v1::PageRequest;
use sqlx::PgPool;
use uuid::Uuid;

use crate::app;
use crate::identity::accounts::PASSWORD;

/// A pass deletes the sessions that ended or that a person closed more than 90 days ago, and
/// keeps the open sessions and the sessions that ended less than 90 days ago.
#[sqlx::test(migrations = false)]
async fn a_pass_deletes_the_sessions_that_closed_before_their_time(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let email = "sessions@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    for _ in 0..4 {
        app.sign_in(email, PASSWORD).await.unwrap();
    }
    let ids = session_ids(&pool, user).await;
    sqlx::query!(
        "UPDATE sessions SET revoked_at = now() - interval '91 days' WHERE id = $1",
        ids[0]
    )
    .execute(&pool)
    .await
    .unwrap();
    for (id, days) in [(ids[1], 91), (ids[2], 89)] {
        sqlx::query!(
            "UPDATE sessions SET expires_at = now() - make_interval(days => $2) WHERE id = $1",
            id,
            days,
        )
        .execute(&pool)
        .await
        .unwrap();
    }

    // when
    retention::run(&pool, &RetentionSettings::default())
        .await
        .unwrap();

    // then
    assert_eq!(session_ids(&pool, user).await, ids[2..].to_vec());
}

/// The list of the users keeps the last sign-in of a user whose sessions the pass deleted.
#[sqlx::test(migrations = false)]
async fn list_users_keeps_the_last_sign_in_after_a_pass(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let admin = app.admin().await;
    let email = "last@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    app.sign_in(email, PASSWORD).await.unwrap();
    sqlx::query!(
        "UPDATE sessions SET revoked_at = now() - interval '91 days' WHERE user_id = $1",
        user.to_db()
    )
    .execute(&pool)
    .await
    .unwrap();

    // when
    retention::run(&pool, &RetentionSettings::default())
        .await
        .unwrap();
    let list = app.list_users(admin, email, PageRequest::default()).await;

    // then
    assert!(session_ids(&pool, user).await.is_empty());
    let users = list.unwrap().users;
    assert_eq!(users.len(), 1, "{users:?}");
    assert!(users[0].last_sign_in_at.is_some(), "{users:?}");
}

/// A pass deletes the links of the mails that were used or that ended more than 30 days ago, and
/// keeps the open links and the links that ended less than 30 days ago.
#[sqlx::test(migrations = false)]
async fn a_pass_deletes_the_links_that_ended_before_their_time(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let email = "links@example.com";
    let user = app.user_with(email, None).await;
    let rows = [
        ("used", 1, Some(-31)),
        ("expired", -31, None),
        ("recent", -29, None),
        ("open", 1, None),
    ];
    for (name, expires, used) in rows {
        sqlx::query!(
            "INSERT INTO one_time_tokens (hash, purpose, user_id, email, expires_at, used_at)
             VALUES ($1, 'reset_password', $2, $3, now() + make_interval(days => $4),
                     now() + make_interval(days => $5))",
            name.as_bytes(),
            user.to_db(),
            email,
            expires,
            used,
        )
        .execute(&pool)
        .await
        .unwrap();
    }

    // when
    retention::run(&pool, &RetentionSettings::default())
        .await
        .unwrap();

    // then
    let left = sqlx::query_scalar!(
        "SELECT hash FROM one_time_tokens WHERE user_id = $1 ORDER BY hash",
        user.to_db()
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(left, [b"open".to_vec(), b"recent".to_vec()]);
}

/// A pass deletes the invitations that were accepted or that ended more than 90 days ago, and
/// keeps the open invitations and the invitations that ended less than 90 days ago.
#[sqlx::test(migrations = false)]
async fn a_pass_deletes_the_invitations_that_ended_before_their_time(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let mut ids = Vec::new();
    for email in ["accepted", "expired", "recent", "open"] {
        let email = format!("{email}@example.com");
        let invitation = app.create_invitation(owner, workspace, &email, "member");
        let invitation = invitation.await.unwrap();
        ids.push(Uuid::parse_str(&invitation.id.unwrap().value).unwrap());
    }
    sqlx::query!(
        "UPDATE invitations SET accepted_by = invited_by, accepted_at = now() - interval '91 days'
         WHERE id = $1",
        ids[0]
    )
    .execute(&pool)
    .await
    .unwrap();
    for (id, days) in [(ids[1], 91), (ids[2], 89)] {
        sqlx::query!(
            "UPDATE invitations SET expires_at = now() - make_interval(days => $2) WHERE id = $1",
            id,
            days,
        )
        .execute(&pool)
        .await
        .unwrap();
    }

    // when
    retention::run(&pool, &RetentionSettings::default())
        .await
        .unwrap();

    // then
    let left = sqlx::query_scalar!(
        "SELECT email FROM invitations WHERE workspace_id = $1 ORDER BY email",
        workspace.to_db()
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(left, ["open@example.com", "recent@example.com"]);
}

/// A pass deletes the proof of the terms of the accounts deleted more than 5 years ago, and keeps
/// the proof of the accounts deleted less than 5 years ago and the acceptances of the live
/// accounts.
#[sqlx::test(migrations = false)]
async fn a_pass_deletes_the_terms_proofs_after_five_years(pool: PgPool) {
    // given
    let app = app(pool.clone()).await;
    let old = app.user().await;
    let recent = app.user().await;
    let live = app.user().await;
    for (user, deleted) in [(old, Some(1828)), (recent, Some(1826)), (live, None)] {
        sqlx::query!(
            "INSERT INTO terms_acceptances (user_id, kind, version, method, accepted_at,
                                            account_deleted_at)
             VALUES ($1, 'terms', '1.0', 'acceptance_step', now() - interval '3000 days',
                     now() - make_interval(days => $2))",
            user.to_db(),
            deleted,
        )
        .execute(&pool)
        .await
        .unwrap();
    }

    // when
    retention::run(&pool, &RetentionSettings::default())
        .await
        .unwrap();

    // then
    let left = sqlx::query_scalar!("SELECT user_id FROM terms_acceptances ORDER BY user_id")
        .fetch_all(&pool)
        .await
        .unwrap();
    let mut kept = vec![recent.to_db(), live.to_db()];
    kept.sort();
    assert_eq!(left, kept);
}

/// Returns the ids of the sessions of a user, oldest first.
async fn session_ids(pool: &PgPool, user: ActorId) -> Vec<Uuid> {
    sqlx::query_scalar!(
        "SELECT id FROM sessions WHERE user_id = $1 ORDER BY created_at, id",
        user.to_db()
    )
    .fetch_all(pool)
    .await
    .unwrap()
}
