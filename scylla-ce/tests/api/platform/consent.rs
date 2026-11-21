//! The tests of `ConsentService`: anyone records a choice of a browser, a withdrawal is a new
//! record of the same consent, a token of a user names the user, and the export and the deletion
//! of the account include the records of the user.

use scylla_domain::identity::actors::ActorId;
use scylla_domain::platform::consent::ConsentId;
use scylla_proto::platform::v1::{
    ConsentAction, ConsentChoice, ConsentSource, RecordConsentRequest,
};
use scylla_testkit::{id, text, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::identity::accounts::PASSWORD;
use crate::{app, assert_code, assert_invalid_field};

/// The SHA-256 of the text of a banner.
const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

/// Makes a record of the banner, with one choice for each `(purpose, granted)`.
fn choice(action: ConsentAction, choices: &[(&str, bool)]) -> RecordConsentRequest {
    RecordConsentRequest {
        consent_id: None,
        banner_version: "a1b2c3".to_owned(),
        banner_hash: HASH.to_owned(),
        choices: choices
            .iter()
            .map(|(purpose, granted)| ConsentChoice {
                purpose: (*purpose).to_owned(),
                granted: *granted,
            })
            .collect(),
        action: action as i32,
        source: ConsentSource::Banner as i32,
    }
}

/// Returns the user and the action of each record of a consent, oldest first.
async fn records(app: &TestApp, consent: &str) -> Vec<(Option<String>, String)> {
    let consent = ConsentId::parse(consent).expect("a consent id").to_db();
    sqlx::query!(
        "SELECT user_id, action FROM consent_records WHERE consent_id = $1
         ORDER BY recorded_at, id",
        consent,
    )
    .fetch_all(&app.pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| (row.user_id.map(|user| user.to_string()), row.action))
    .collect()
}

/// A browser with no token gets a new consent id. Its next choice keeps the id: a withdrawal is
/// a new record, and the first record stays.
#[sqlx::test(migrations = false)]
async fn record_consent_keeps_each_choice_of_a_browser(pool: PgPool) {
    // given
    let app = app(pool).await;
    let accept = choice(ConsentAction::AcceptAll, &[("analytics", true)]);

    // when
    let first = app.record_consent(None, accept).await.unwrap();
    let consent = text(&first.consent_id).to_owned();
    let withdraw = RecordConsentRequest {
        consent_id: Some(id(&consent)),
        source: ConsentSource::Preferences as i32,
        ..choice(ConsentAction::Withdraw, &[("analytics", false)])
    };
    let second = app.record_consent(None, withdraw).await.unwrap();

    // then
    assert_eq!(text(&second.consent_id), consent);
    assert!(first.recorded_at.is_some());
    let actions = vec![
        (None, "accept_all".to_owned()),
        (None, "withdraw".to_owned()),
    ];
    assert_eq!(records(&app, &consent).await, actions);
}

/// A token of a user names the user in the record. The export of the account holds the record,
/// and the deletion of the account deletes it.
#[sqlx::test(migrations = false)]
async fn record_consent_of_a_user_goes_with_the_account(pool: PgPool) {
    // given
    let app = app(pool).await;
    let email = "ada@example.com";
    let user = app.user_with(email, Some(PASSWORD)).await;
    let reject = choice(ConsentAction::RejectAll, &[("analytics", false)]);

    // when
    let recorded = app.record_consent(Some(user), reject).await.unwrap();
    let consent = text(&recorded.consent_id).to_owned();
    let token = app
        .sign_in(email, PASSWORD)
        .await
        .unwrap()
        .token
        .unwrap()
        .value;
    let export = app.export_account_data(&token).await.unwrap();
    let named = records(&app, &consent).await;
    app.delete_account(&token, PASSWORD).await.unwrap();

    // then
    assert_eq!(
        named,
        vec![(Some(user.to_string()), "reject_all".to_owned())]
    );
    let data: serde_json::Value = serde_json::from_str(&export.json).unwrap();
    assert_eq!(data["consents"][0]["consent_id"], consent);
    assert_eq!(data["consents"][0]["action"], "reject_all");
    assert_eq!(data["consents"][0]["choices"]["analytics"], false);
    assert!(records(&app, &consent).await.is_empty());
}

/// Only a user has an account: the token of an App gets `PERMISSION_DENIED`.
#[sqlx::test(migrations = false)]
async fn record_consent_refuses_the_token_of_an_app(pool: PgPool) {
    // given
    let app = app(pool).await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;
    let created = app.create_app(owner, workspace, false).await.unwrap();
    let ci = ActorId::parse(text(&created.app.unwrap().id)).expect("an actor id");
    let accept = choice(ConsentAction::AcceptAll, &[("analytics", true)]);

    // when
    let refused = app.record_consent(Some(ci), accept).await;

    // then
    assert_code(refused, Code::PermissionDenied);
}

/// A record needs a SHA-256, a version, an action, a source, and one valid choice for each
/// purpose that fits the action.
#[sqlx::test(migrations = false)]
async fn record_consent_checks_each_field(pool: PgPool) {
    // given
    let app = app(pool).await;
    let accept = || choice(ConsentAction::AcceptAll, &[("analytics", true)]);
    let cases = [
        (
            RecordConsentRequest {
                banner_hash: "ABC".to_owned(),
                ..accept()
            },
            "banner_hash",
        ),
        (
            RecordConsentRequest {
                banner_version: " ".to_owned(),
                ..accept()
            },
            "banner_version",
        ),
        (
            RecordConsentRequest {
                action: ConsentAction::Unspecified as i32,
                ..accept()
            },
            "action",
        ),
        (
            RecordConsentRequest {
                source: ConsentSource::Unspecified as i32,
                ..accept()
            },
            "source",
        ),
        (
            choice(ConsentAction::AcceptAll, &[("Analytics", true)]),
            "choices[0].purpose",
        ),
        (choice(ConsentAction::Custom, &[]), "choices"),
        (
            choice(ConsentAction::AcceptAll, &[("analytics", false)]),
            "choices",
        ),
        (
            choice(
                ConsentAction::Custom,
                &[("support", true), ("support", false)],
            ),
            "choices",
        ),
    ];

    for (request, field) in cases {
        // when
        let refused = app.record_consent(None, request).await;

        // then
        assert_invalid_field(&refused.unwrap_err(), field);
    }
}

/// An address gets 30 records in one minute, then `RESOURCE_EXHAUSTED`.
#[sqlx::test(migrations = false)]
async fn record_consent_limits_the_records_of_an_address(pool: PgPool) {
    // given
    let app = app(pool).await;
    let accept = || choice(ConsentAction::AcceptAll, &[("analytics", true)]);
    for _ in 0..30 {
        app.record_consent(None, accept()).await.unwrap();
    }

    // when
    let refused = app.record_consent(None, accept()).await;

    // then
    assert_code(refused, Code::ResourceExhausted);
}
