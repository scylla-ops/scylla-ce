//! The tests of the logs of the server: the request id of each answer, one line for each call
//! with its refusal, no line for a preflight, and no secret in a line.

use scylla_core::api::ServeOptions;
use scylla_core::SignUpMode;
use scylla_domain::identity::actors::ActorId;
use scylla_domain::Id;
use scylla_proto::accounts::v1::account_service_client::AccountServiceClient;
use scylla_proto::accounts::v1::profile_service_client::ProfileServiceClient;
use scylla_proto::accounts::v1::{GetMeRequest, ListSignInMethodsRequest};
use scylla_testkit::{text, Logs, TestApp};
use sqlx::PgPool;
use tonic::Code;

use crate::assert_code;

/// The origin that the tests allow.
const ORIGIN: &str = "https://app.example";

/// The header of the request id.
const REQUEST_ID: &str = "x-request-id";

/// Starts the Community edition in open sign-up, for `ORIGIN`.
async fn app(pool: PgPool) -> TestApp {
    let mut options = ServeOptions {
        cors_origins: vec![ORIGIN.to_owned()],
        ..ServeOptions::default()
    };
    options.accounts.sign_up_mode = SignUpMode::Open;
    TestApp::spawn_with(pool, scylla_ce::edition(), options).await
}

/// Each answer carries a new request id: a gRPC answer, a gRPC error, a gRPC-Web answer that a
/// web app can read, and a browser route.
#[sqlx::test(migrations = false)]
async fn each_answer_carries_a_request_id(pool: PgPool) {
    // given
    let app = app(pool).await;
    let http = reqwest::Client::new();
    let url = |path: &str| format!("http://{}{path}", app.addr);

    // when
    let methods = AccountServiceClient::new(app.channel.clone())
        .list_sign_in_methods(ListSignInMethodsRequest {})
        .await
        .unwrap();
    let error = ProfileServiceClient::new(app.channel.clone())
        .get_me(GetMeRequest {})
        .await
        .unwrap_err();
    let web = http
        .post(url("/grpc.health.v1.Health/Check"))
        .header("content-type", "application/grpc-web+proto")
        .header("origin", ORIGIN)
        .body(vec![0u8, 0, 0, 0, 0])
        .send()
        .await
        .unwrap();
    let no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let browser = no_redirect
        .get(url("/auth/unknown/start"))
        .send()
        .await
        .unwrap();

    // then
    assert_eq!(error.code(), Code::Unauthenticated);
    let header = |value: Option<&[u8]>| String::from_utf8(value.unwrap().to_vec()).unwrap();
    let ids = [
        header(methods.metadata().get(REQUEST_ID).map(|v| v.as_bytes())),
        header(error.metadata().get(REQUEST_ID).map(|v| v.as_bytes())),
        header(web.headers().get(REQUEST_ID).map(|v| v.as_bytes())),
        header(browser.headers().get(REQUEST_ID).map(|v| v.as_bytes())),
    ];
    assert!(ids.iter().all(|id| Id::<()>::parse(id).is_ok()), "{ids:?}");
    assert!(ids
        .iter()
        .enumerate()
        .all(|(i, id)| !ids[i + 1..].contains(id)));
    let exposed = web.headers().get("access-control-expose-headers").unwrap();
    assert!(
        exposed.to_str().unwrap().contains(REQUEST_ID),
        "{exposed:?}"
    );
}

/// A refused check gives one line for its call, with the permission and the object on its second
/// line, and no line of its own.
#[sqlx::test(migrations = false)]
async fn get_workspace_denial_logs_one_line_with_its_permission(pool: PgPool) {
    // given
    let app = app(pool).await;
    let (alice, bob) = (app.user().await, app.user().await);
    let workspace = app.workspace_of(bob).await.to_string();
    let logs = Logs::pretty();

    // when
    let denied = app.get_workspace(alice, &workspace).await;

    // then
    assert_code(denied, Code::PermissionDenied);
    let lines = logs.lines();
    let calls: Vec<_> = (0..lines.len())
        .filter(|&index| lines[index].contains("WorkspaceService.GetWorkspace"))
        .collect();
    assert_eq!(calls.len(), 1, "{lines:#?}");
    let (line, details) = (&lines[calls[0]], &lines[calls[0] + 1]);
    assert!(line.contains("  DENIED  "), "{line}");
    let alice = alice.to_string();
    assert!(
        line.contains(&format!("user ..{}", &alice[alice.len() - 8..])),
        "{line}"
    );
    assert!(line.contains("127.0.0.1"), "{line}");
    let object = format!("object=workspace:..{}", &workspace[workspace.len() - 8..]);
    assert!(details.starts_with("    "), "{details}");
    assert!(details.contains("permission=get_workspace"), "{details}");
    assert!(details.contains(&object), "{details}");
    assert!(!logs.text().contains("access refused"), "{lines:#?}");
    assert_eq!(lines.len(), 2, "{lines:#?}");
}

/// A refusal of a rule shows its reason on the second line of its call.
#[sqlx::test(migrations = false)]
async fn sign_up_refusal_logs_its_reason(pool: PgPool) {
    // given
    let app = TestApp::spawn(pool, scylla_ce::edition()).await;
    let logs = Logs::pretty();

    // when
    let refused = app.sign_up("new@example.com", "a long password", "").await;

    // then
    assert_code(refused, Code::FailedPrecondition);
    let lines = logs.lines();
    assert!(
        lines[0].contains("  REFUSED   AccountService.SignUp "),
        "{lines:#?}"
    );
    assert!(lines[0].contains("anonymous"), "{lines:#?}");
    assert_eq!(lines[1].trim(), "reason=INVITATION_REQUIRED", "{lines:#?}");
}

/// A CORS preflight gets its answer and logs nothing.
#[sqlx::test(migrations = false)]
async fn preflight_logs_nothing(pool: PgPool) {
    // given
    let app = app(pool).await;
    let url = format!(
        "http://{}/scylla.projects.v1.ProjectService/ListProjects",
        app.addr
    );
    let logs = Logs::pretty();

    // when
    let preflight = reqwest::Client::new()
        .request(reqwest::Method::OPTIONS, &url)
        .header("origin", ORIGIN)
        .header("access-control-request-method", "POST")
        .send()
        .await
        .unwrap();

    // then
    assert!(preflight
        .headers()
        .contains_key("access-control-allow-origin"));
    assert_eq!(logs.text(), "");
}

/// The sign-up, the sign-in, the reset and the secrets log their events with no password, no
/// token, no link, no secret value and no full email, in JSON.
#[sqlx::test(migrations = false)]
async fn account_and_secret_events_hide_the_secrets_in_json(pool: PgPool) {
    // given
    let app = app(pool).await;
    let logs = Logs::json();

    // when
    let secrets = account_and_secret_calls(&app).await;

    // then
    let text = logs.text();
    for secret in &secrets {
        assert!(
            !text.contains(secret.as_str()),
            "{secret} is in the logs:\n{text}"
        );
    }
    for event in EVENTS {
        assert!(
            text.contains(&format!("\"event\":\"{event}\"")),
            "no {event}:\n{text}"
        );
    }
    assert!(text.contains("\"to\":\"l***@example.com\""), "{text}");
}

/// The same calls log no secret in the pretty format either.
#[sqlx::test(migrations = false)]
async fn account_and_secret_events_hide_the_secrets_in_pretty(pool: PgPool) {
    // given
    let app = app(pool).await;
    let logs = Logs::pretty();

    // when
    let secrets = account_and_secret_calls(&app).await;

    // then
    let text = logs.text();
    for secret in &secrets {
        assert!(
            !text.contains(secret.as_str()),
            "{secret} is in the logs:\n{text}"
        );
    }
    assert!(
        text.contains("reset-password -> l***@example.com"),
        "{text}"
    );
    assert!(text.contains("secret.updated"), "{text}");
    assert!(
        text.contains("  OK        AccountService.SignIn "),
        "{text}"
    );
    assert!(!text.contains("CANCELLED"), "{text}");
}

/// The business events of `account_and_secret_calls`.
const EVENTS: [&str; 9] = [
    "account.created",
    "mail.sent",
    "account.verified",
    "account.signed_in",
    "account.signin_failed",
    "password.reset_requested",
    "password.changed",
    "secret.created",
    "secret.updated",
];

/// Signs up, verifies the email, signs in, fails a sign-in, resets the password, and creates and
/// changes a secret. Returns each secret of these calls: passwords, tokens, links, the values and
/// the email.
async fn account_and_secret_calls(app: &TestApp) -> Vec<String> {
    // Sign up, verify the email, and sign in.
    let email = format!("logs-{}@example.com", ActorId::generate());
    let (old, new, wrong) = ("first password", "second password", "wrong password");
    app.sign_up(&email, old, "").await.unwrap();
    let verify = app.mails.token(&email, "verify-email");
    app.verify_email(&verify).await.unwrap();
    let first = app.sign_in(&email, old).await.unwrap();
    assert!(app.sign_in(&email, wrong).await.is_err());

    // Reset the password with the link of the mail, and sign in again.
    app.request_reset(&email).await.unwrap();
    app.mails.wait(&email, 2).await;
    let reset = app.mails.token(&email, "reset-password");
    app.reset_password(&reset, new).await.unwrap();
    let second = app.sign_in(&email, new).await.unwrap();

    // Create a secret and give it a new value.
    let user = app.user().await;
    let project = app.create_project(user, "Logs").await.unwrap();
    let (one, two) = ("first-secret-value", "second-secret-value");
    let created = app.create_secret(user, text(&project.id), "TOKEN", one);
    let created = created.await.unwrap().secret.unwrap();
    app.update_secret(user, text(&created.id), two)
        .await
        .unwrap();

    // Return the secrets, and each access and refresh token.
    let mut secrets = vec![email, old.into(), new.into(), wrong.into(), verify, reset];
    for signed_in in [first, second] {
        secrets.push(signed_in.token.unwrap().value);
        secrets.push(signed_in.refresh_token);
    }
    secrets.extend([one.into(), two.into(), "#token=".into(), "Bearer".into()]);
    secrets
}
