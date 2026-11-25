//! The tests of the terms of service: the acceptance at the sign-up with a password, the state in
//! `GetMe`, the acceptance of a new version with `AcceptTerms`, the export and the deletion.

use scylla_core::SignUpMode;
use scylla_domain::identity::actors::ActorId;
use scylla_proto::platform::v1::legal_document::Content;
use scylla_proto::platform::v1::{LegalDocument, LegalDocumentKind, UpdateLegalSettingsRequest};
use scylla_testkit::TestApp;
use serde_json::Value;
use sqlx::PgPool;
use tonic::{Code, Status};
use tonic_types::StatusExt;

use crate::identity::accounts::{app_in, assert_refused, PASSWORD};
use crate::{app, assert_code};

/// Sets the terms of the instance: a version, with or without an acceptance of each user.
/// `None` turns the terms off.
pub(crate) async fn set_terms(app: &TestApp, admin: ActorId, terms: Option<(&str, bool)>) {
    let document = LegalDocument {
        kind: LegalDocumentKind::Terms as i32,
        content: terms.map(|_| Content::Markdown("# Terms\n".to_owned())),
        version: terms.map_or("", |(version, _)| version).to_owned(),
        acceptance_required: terms.is_some_and(|(_, required)| required),
        ..LegalDocument::default()
    };
    let request = UpdateLegalSettingsRequest {
        documents: vec![document],
        ..UpdateLegalSettingsRequest::default()
    };
    app.update_legal_settings(admin, request).await.unwrap();
}

/// Checks that `result` is `TERMS_NOT_ACCEPTED`, with the current version in its metadata.
fn assert_not_accepted<T: std::fmt::Debug>(result: Result<T, Status>, current: &str) {
    let status = result.expect_err("the call must fail");
    let info = status.get_details_error_info().expect("ErrorInfo detail");
    assert_eq!(
        info.metadata.get("version").map(String::as_str),
        Some(current)
    );
    assert_refused(Err::<(), _>(status), "TERMS_NOT_ACCEPTED");
}

/// Reads the acceptances of the terms in the export of a user.
async fn exported(app: &TestApp, token: &str) -> Vec<Value> {
    let export = app.export_account_data(token).await.unwrap();
    let data: Value = serde_json::from_str(&export.json).unwrap();
    data["terms_acceptances"].as_array().unwrap().clone()
}

/// Terms that are off, or that need no acceptance, ask nothing of a user.
#[sqlx::test(migrations = false)]
async fn get_me_asks_nothing_when_the_terms_need_no_acceptance(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let token = app.token(app.user().await).await;

    // when
    let off = app.me(&token).await.unwrap();
    set_terms(&app, admin, Some(("1.0", false))).await;
    let informative = app.me(&token).await.unwrap();

    // then
    for me in [off, informative] {
        assert!(!me.terms_acceptance_required);
        assert_eq!(me.accepted_terms, None);
        assert!(me.user.is_some());
    }
}

/// When the terms need an acceptance, a sign-up that does not accept their current version is
/// refused, names the current version, and makes no account.
#[sqlx::test(migrations = false)]
async fn sign_up_refuses_a_missing_or_old_version_of_the_terms(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let admin = app.admin().await;
    set_terms(&app, admin, Some(("2.0", true))).await;
    let email = "new@example.com";

    // when
    let missing = app.sign_up_accepting(email, PASSWORD, None).await;
    let old = app.sign_up_accepting(email, PASSWORD, Some("1.0")).await;
    let current = app.sign_up_accepting(email, PASSWORD, Some("2.0")).await;

    // then
    assert_not_accepted(missing, "2.0");
    assert_not_accepted(old, "2.0");
    assert!(current.is_ok(), "{current:?}");
    assert_eq!(app.mails.wait(email, 1).await.len(), 1);
}

/// A sign-up keeps the version that the person accepted, with the method of the form, and the
/// user then has nothing to accept.
#[sqlx::test(migrations = false)]
async fn sign_up_keeps_the_accepted_version(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let admin = app.admin().await;
    set_terms(&app, admin, Some(("2.0", true))).await;
    let email = "new@example.com";
    app.sign_up_accepting(email, PASSWORD, Some("2.0"))
        .await
        .unwrap();
    app.verify_email(&app.mails.token(email, "verify-email").await)
        .await
        .unwrap();
    let signed_in = app.sign_in(email, PASSWORD).await.unwrap();
    let token = signed_in.token.unwrap().value;

    // when
    let me = app.me(&token).await.unwrap();
    let rows = exported(&app, &token).await;

    // then
    assert!(!me.terms_acceptance_required);
    let accepted = me.accepted_terms.unwrap();
    assert_eq!(accepted.version, "2.0");
    assert!(accepted.accepted_at.is_some());
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["document"], "terms");
    assert_eq!(rows[0]["version"], "2.0");
    assert_eq!(rows[0]["method"], "sign_up_form");
    assert!(rows[0]["accepted_at"].is_string());
}

/// Terms that need no acceptance let a sign-up accept nothing, and keep no acceptance.
#[sqlx::test(migrations = false)]
async fn sign_up_needs_no_version_when_the_terms_need_no_acceptance(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let admin = app.admin().await;
    set_terms(&app, admin, Some(("1.0", false))).await;

    // when
    let result = app
        .sign_up_accepting("new@example.com", PASSWORD, None)
        .await;

    // then
    assert!(result.is_ok(), "{result:?}");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM terms_acceptances")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0);
}

/// Each user must accept each new version that needs an acceptance: after a sign-up with a
/// provider, and after a change of the version. The export keeps each acceptance and its method.
#[sqlx::test(migrations = false)]
async fn get_me_asks_each_user_to_accept_a_new_version(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let token = app.token(app.user().await).await;
    set_terms(&app, admin, Some(("1.0", true))).await;

    // when
    let first = app.me(&token).await.unwrap();
    app.accept_terms(&token, "1.0").await.unwrap();
    let accepted = app.me(&token).await.unwrap();
    set_terms(&app, admin, Some(("2.0", true))).await;
    let changed = app.me(&token).await.unwrap();
    app.accept_terms(&token, "2.0").await.unwrap();
    let again = app.me(&token).await.unwrap();

    // then
    assert!(first.terms_acceptance_required);
    assert_eq!(first.accepted_terms, None);
    assert!(!accepted.terms_acceptance_required);
    assert!(changed.terms_acceptance_required);
    assert_eq!(changed.accepted_terms.unwrap().version, "1.0");
    assert!(!again.terms_acceptance_required);
    assert_eq!(again.accepted_terms.unwrap().version, "2.0");
    let rows = exported(&app, &token).await;
    let methods: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| {
            (
                row["version"].as_str().unwrap(),
                row["method"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        methods,
        [("1.0", "acceptance_step"), ("2.0", "re_acceptance")]
    );
}

/// `AcceptTerms` refuses a version that is not the current version, and terms that are off, and
/// names the current version.
#[sqlx::test(migrations = false)]
async fn accept_terms_refuses_a_version_that_is_not_current(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let token = app.token(app.user().await).await;

    // when
    let off = app.accept_terms(&token, "").await;
    set_terms(&app, admin, Some(("2.0", true))).await;
    let old = app.accept_terms(&token, "1.0").await;

    // then
    assert_not_accepted(off, "");
    assert_not_accepted(old, "2.0");
    assert!(app.me(&token).await.unwrap().terms_acceptance_required);
}

/// A second acceptance of one version keeps the time of the first one.
#[sqlx::test(migrations = false)]
async fn accept_terms_keeps_the_first_acceptance(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    let token = app.token(app.user().await).await;
    set_terms(&app, admin, Some(("1.0", true))).await;
    let first = app.accept_terms(&token, "1.0").await.unwrap();

    // when
    let second = app.accept_terms(&token, "1.0").await.unwrap();

    // then
    assert_eq!(second, first);
    assert_eq!(exported(&app, &token).await.len(), 1);
}

/// `AcceptTerms` needs a valid token of an account that exists.
#[sqlx::test(migrations = false)]
async fn accept_terms_needs_an_account(pool: PgPool) {
    // given
    let app = app(pool).await;
    let admin = app.admin().await;
    set_terms(&app, admin, Some(("1.0", true))).await;
    let user = app.user_with("ada@example.com", Some(PASSWORD)).await;
    let token = app.token(user).await;
    app.accept_terms(&token, "1.0").await.unwrap();

    // when
    let anonymous = app.accept_terms("not-a-token", "1.0").await;
    app.delete_account(&token, PASSWORD).await.unwrap();
    let deleted = app.accept_terms(&token, "1.0").await;

    // then
    assert_code(anonymous, Code::Unauthenticated);
    assert_code(deleted, Code::Unauthenticated);
}
