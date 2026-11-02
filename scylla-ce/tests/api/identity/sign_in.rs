//! The tests of the sign-in of a browser with an identity provider: the start, the provider,
//! the callback, the session cookie, the link of a provider, and GitHub.

use scylla_core::SignUpMode;
use scylla_domain::identity::actors::ActorId;
use scylla_testkit::TestApp;
use sqlx::PgPool;

use crate::app;
use crate::identity::accounts::{app_in, assert_refused, PASSWORD};
use crate::identity::browser::Browser;
use crate::identity::idp::{person, Idp, CLIENT_ID};

/// The web app of the tests, the default of the settings.
const APP_URL: &str = "http://127.0.0.1:3000";

/// Returns the access token of the session cookie of a browser.
async fn token_of(app: &TestApp, browser: &Browser) -> String {
    let refresh = &browser.cookies["scylla_session"];
    app.refresh(refresh).await.unwrap().token.unwrap().value
}

/// A browser signs up with an OIDC provider: the callback opens a session cookie and goes back
/// to the web app. The last way to sign in cannot be removed.
#[sqlx::test(migrations = false)]
async fn a_browser_signs_up_with_a_provider(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, Some("secret"), true)
        .await;
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;

    // then
    assert_eq!((step.status, step.location.as_str()), (303, APP_URL));
    assert!(!browser.cookies.contains_key("scylla_flow"));
    let token = token_of(&app, &browser).await;
    let me = app.get_me(&token).await.unwrap();
    assert_eq!(me.email, "alice@idp.example");
    let unlink = app.unlink_identity(&token, "idp", "alice").await;
    assert_refused(unlink, "LAST_SIGN_IN_METHOD");
}

/// A return address in the web app stays, and another one becomes the web app.
#[sqlx::test(migrations = false)]
async fn a_sign_in_goes_back_to_the_web_app_only(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, true)
        .await;

    // when
    let inside = Browser::new()
        .sign_in(&app, "idp", &format!("return_to={APP_URL}/projects"))
        .await;
    let outside = Browser::new()
        .sign_in(&app, "idp", "return_to=https://evil.example")
        .await;

    // then
    assert_eq!(inside.location, format!("{APP_URL}/projects"));
    assert_eq!(outside.location, APP_URL);
}

/// A known email is never linked automatically. The person signs in, then links the provider,
/// and the provider then signs in the same user.
#[sqlx::test(migrations = false)]
async fn a_known_email_is_linked_only_by_its_user(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, true)
        .await;
    let user = app.user_with("alice@idp.example", Some(PASSWORD)).await;
    let session = app.sign_in("alice@idp.example", PASSWORD).await.unwrap();

    // when
    let refused = Browser::new().sign_in(&app, "idp", "").await;
    let mut signed_in = Browser::new();
    signed_in
        .cookies
        .insert("scylla_session".to_owned(), session.refresh_token.clone());
    let linked = signed_in.sign_in(&app, "idp", "link=true").await;
    let mut later = Browser::new();
    let again = later.sign_in(&app, "idp", "").await;

    // then
    assert_eq!(refused.status, 409);
    assert!(refused.body.starts_with("EMAIL_IN_USE"), "{}", refused.body);
    assert_eq!(linked.status, 303);
    assert_eq!(again.status, 303);
    let me = app.get_me(&token_of(&app, &later).await).await.unwrap();
    assert_eq!(me.id.unwrap().value, user.to_string());
}

/// The callback needs the browser that started the sign-in.
#[sqlx::test(migrations = false)]
async fn the_callback_needs_the_same_browser(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, true)
        .await;
    let mut starter = Browser::new();
    let start = starter
        .get(&format!("http://{}/auth/idp/start", app.addr))
        .await;

    // when
    let mut other = Browser::new();
    let provider = other.get(&start.location).await;
    let callback = other.get(&provider.location).await;

    // then
    assert_eq!(callback.status, 401);
    assert!(!other.cookies.contains_key("scylla_session"));
}

/// In the mode `invite_only`, a new person needs an open invitation for its email, and then
/// joins the workspace of the invitation.
#[sqlx::test(migrations = false)]
async fn a_provider_sign_up_needs_an_invitation_when_invite_only(pool: PgPool) {
    // given
    let app = app(pool).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, true)
        .await;
    let owner = app.user().await;
    let workspace = app.workspace_of(owner).await;

    // when
    let refused = Browser::new().sign_in(&app, "idp", "").await;
    app.create_invitation(owner, workspace, "alice@idp.example", "member")
        .await
        .unwrap();
    let mut browser = Browser::new();
    let accepted = browser.sign_in(&app, "idp", "").await;

    // then
    assert!(
        refused.body.starts_with("INVITATION_REQUIRED"),
        "{}",
        refused.body
    );
    assert_eq!(accepted.status, 303);
    let me = app.get_me(&token_of(&app, &browser).await).await.unwrap();
    let alice = ActorId::parse(&me.id.unwrap().value).unwrap();
    let created = app.create_project_in(alice, workspace, "alice").await;
    assert!(created.is_ok(), "{created:?}");
}

/// A GitHub sign-in reads the user and its primary verified email from the API.
#[sqlx::test(migrations = false)]
async fn a_browser_signs_up_with_github(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_github_provider("github", idp.url(), "gh-client", "gh-secret")
        .await;
    idp.sign_in_as(person("octocat", "octo@example.com", true));
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "github", "").await;

    // then
    assert_eq!(step.status, 303, "{}", step.body);
    let me = app.get_me(&token_of(&app, &browser).await).await.unwrap();
    assert_eq!(
        (me.email.as_str(), me.display_name.as_str()),
        ("octo@example.com", "octocat")
    );
}
