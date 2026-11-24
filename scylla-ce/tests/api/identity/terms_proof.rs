//! The tests of the proof of the terms of service: the acceptances of a deleted account stay, with
//! the time of the deletion and the hash of the email of the account.

use scylla_domain::identity::actors::ActorId;
use scylla_testkit::TestApp;
use sha2::{Digest, Sha256};
use sqlx::PgPool;

use crate::app;
use crate::identity::accounts::PASSWORD;
use crate::identity::terms::set_terms;

/// One acceptance in the table: the version, the method, the date as text, whether the account
/// was deleted, and the hash of its email.
type Proof = (String, String, String, bool, Option<Vec<u8>>);

/// Reads the acceptances of a user in the table, by version.
async fn proofs(app: &TestApp, user: ActorId) -> Vec<Proof> {
    sqlx::query_as(
        "SELECT version, method, accepted_at::text, account_deleted_at IS NOT NULL, email_hash
         FROM terms_acceptances
         WHERE user_id = $1
         ORDER BY version",
    )
    .bind(user.to_db())
    .fetch_all(&app.pool)
    .await
    .unwrap()
}

/// `DeleteAccount` keeps the acceptances of the terms as the proof (the account id, the version,
/// the method and the date) with the time of the deletion and the SHA-256 hash of the primary
/// email in lowercase, and does not change the acceptances of the other users.
#[sqlx::test(migrations = false)]
async fn delete_account_keeps_the_proof_of_the_terms(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    set_terms(&app, admin, Some(("1.0", true))).await;
    let user = app.user_with("Ada@Example.com", Some(PASSWORD)).await;
    let token = app.token(user).await;
    app.accept_terms(&token, "1.0").await.unwrap();
    set_terms(&app, admin, Some(("2.0", true))).await;
    app.accept_terms(&token, "2.0").await.unwrap();
    let other = app.user().await;
    app.accept_terms(&app.token(other).await, "2.0")
        .await
        .unwrap();
    let before = proofs(&app, user).await;
    let other_before = proofs(&app, other).await;

    // when
    app.delete_account(&token, PASSWORD).await.unwrap();

    // then
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = $1")
        .bind(user.to_db())
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(users, 0);
    let hash = Sha256::digest(b"ada@example.com").to_vec();
    let expected: Vec<Proof> = before
        .into_iter()
        .map(|(version, method, date, _, _)| (version, method, date, true, Some(hash.clone())))
        .collect();
    let after = proofs(&app, user).await;
    assert_eq!(after, expected);
    let kept: Vec<(&str, &str)> = after
        .iter()
        .map(|row| (row.0.as_str(), row.1.as_str()))
        .collect();
    assert_eq!(kept, [("1.0", "acceptance_step"), ("2.0", "re_acceptance")]);
    assert_eq!(proofs(&app, other).await, other_before);
    assert!(other_before.iter().all(|row| !row.3 && row.4.is_none()));
}
