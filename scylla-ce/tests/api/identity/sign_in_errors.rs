//! The tests of a failed sign-in in a browser: the browser goes to the page `sign-in-error` of the
//! web app with a code, the URL has no data of the person, and no session opens.

use scylla_core::api::ServeOptions;
use scylla_core::SignUpMode;
use scylla_testkit::TestApp;
use sqlx::PgPool;

use crate::identity::accounts::{app_in, PASSWORD};
use crate::identity::browser::{Browser, Step};
use crate::identity::idp::{person, Idp, CLIENT_ID};

/// The web app of the tests, the default of the settings.
const APP_URL: &str = "http://127.0.0.1:3000";

/// Returns the address of the page for a code of the provider `idp`.
fn page(code: &str) -> String {
    format!("{APP_URL}/sign-in-error?code={code}&provider=idp")
}

/// Starts an app in open sign-up with the provider `idp`.
async fn app_with_idp(pool: PgPool) -> (TestApp, Idp) {
    let app = app_in(pool, SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, true)
        .await;
    (app, idp)
}

/// Opens the start route of a provider.
async fn start(app: &TestApp, browser: &mut Browser, slug: &str, query: &str) -> Step {
    let url = format!("http://{}/auth/{slug}/start?{query}", app.addr);
    browser.get(&url).await
}

/// Checks that a step goes to a page, and that the browser has no session and no sign-in left.
fn assert_goes_to(step: &Step, browser: &Browser, page: &str) {
    assert_eq!((step.status, step.location.as_str()), (303, page));
    assert!(!browser.cookies.contains_key("scylla_session"));
    assert!(!browser.cookies.contains_key("scylla_flow"));
}

/// A person who stops at the provider gets `access_denied`.
#[sqlx::test(migrations = false)]
async fn a_stop_at_the_provider_gives_access_denied(pool: PgPool) {
    // given
    let (app, idp) = app_with_idp(pool).await;
    idp.refuse_sign_in("access_denied");
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;

    // then
    assert_goes_to(&step, &browser, &page("access_denied"));
}

/// An error that Scylla does not know gives `provider_refused`: the text of the provider does not
/// go in the URL of the web app.
#[sqlx::test(migrations = false)]
async fn an_unknown_error_of_the_provider_goes_no_further(pool: PgPool) {
    // given
    let (app, idp) = app_with_idp(pool).await;
    idp.refuse_sign_in("call-555-1234");
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;

    // then
    assert_goes_to(&step, &browser, &page("provider_refused"));
}

/// A provider that refuses the redirect URL at the exchange gives `redirect_uri_mismatch`, and
/// another refusal `provider_refused`.
#[sqlx::test(migrations = false)]
async fn a_refused_exchange_gives_its_code(pool: PgPool) {
    // given
    let (app, idp) = app_with_idp(pool).await;
    let (mut first, mut second) = (Browser::new(), Browser::new());

    // when
    idp.refuse_exchange("redirect_uri_mismatch");
    let mismatch = first.sign_in(&app, "idp", "").await;
    idp.refuse_exchange("invalid_grant");
    let other = second.sign_in(&app, "idp", "").await;

    // then
    assert_goes_to(&mismatch, &first, &page("redirect_uri_mismatch"));
    assert_goes_to(&other, &second, &page("provider_refused"));
}

/// A callback with an unknown state gives `state_expired`, and a state works one time.
#[sqlx::test(migrations = false)]
async fn a_state_works_one_time(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool).await;
    let mut browser = Browser::new();
    let start = browser
        .get(&format!("http://{}/auth/idp/start", app.addr))
        .await;
    let provider = browser.get(&start.location).await;
    let flow = browser.cookies["scylla_flow"].clone();

    // when
    let unknown = Browser::new()
        .get(&format!(
            "http://{}/auth/idp/callback?code=x&state=unknown",
            app.addr
        ))
        .await;
    let first = browser.get(&provider.location).await;
    let mut again = Browser::new();
    again.cookies.insert("scylla_flow".to_owned(), flow);
    let replay = again.get(&provider.location).await;

    // then
    assert_eq!(unknown.location, page("state_expired"));
    assert_eq!(first.location, APP_URL);
    assert_eq!(replay.location, page("state_expired"));
}

/// An identity with no verified email gives `email_unverified`.
#[sqlx::test(migrations = false)]
async fn no_verified_email_gives_email_unverified(pool: PgPool) {
    // given
    let (app, idp) = app_with_idp(pool).await;
    idp.sign_in_as(person("bob", "bob@idp.example", false));
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;

    // then
    assert_goes_to(&step, &browser, &page("email_unverified"));
}

/// A provider that does not allow new people gives `signup_disabled`.
#[sqlx::test(migrations = false)]
async fn no_new_account_gives_signup_disabled(pool: PgPool) {
    // given
    let app = app_in(pool, SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, false)
        .await;
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;

    // then
    assert_goes_to(&step, &browser, &page("signup_disabled"));
}

/// The identity of another user gives `identity_in_use`, on the page for a link.
#[sqlx::test(migrations = false)]
async fn the_identity_of_another_user_gives_identity_in_use(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool).await;
    let mut first = Browser::new();
    first.sign_in(&app, "idp", "").await;
    app.user_with("bob@example.com", Some(PASSWORD)).await;
    let session = app.sign_in("bob@example.com", PASSWORD).await.unwrap();
    let mut bob = Browser::new();
    bob.cookies
        .insert("scylla_session".to_owned(), session.refresh_token);

    // when
    let step = bob.sign_in(&app, "idp", "link=true").await;

    // then
    assert_eq!(
        step.location,
        format!("{}&link=true", page("identity_in_use"))
    );
}

/// A disabled account gives `account_inactive`, and it opens no session.
#[sqlx::test(migrations = false)]
async fn a_disabled_account_gives_account_inactive(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool).await;
    let admin = app.admin().await;
    let mut first = Browser::new();
    first.sign_in(&app, "idp", "").await;
    let refresh = &first.cookies["scylla_session"];
    let token = app.refresh(refresh).await.unwrap().token.unwrap().value;
    let user = app.get_me(&token).await.unwrap().id.unwrap().value;
    app.set_user_status(admin, &user, "disabled").await.unwrap();
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;

    // then
    assert_goes_to(&step, &browser, &page("account_inactive"));
}

/// A link with no session gives `session_required`.
#[sqlx::test(migrations = false)]
async fn a_link_needs_a_session(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool).await;
    let mut browser = Browser::new();

    // when
    let step = start(&app, &mut browser, "idp", "link=true").await;

    // then
    assert_goes_to(
        &step,
        &browser,
        &format!("{}&link=true", page("session_required")),
    );
}

/// A provider that is off or unknown gives `provider_off`, and a provider that does not answer
/// `provider_unavailable`.
#[sqlx::test(migrations = false)]
async fn a_provider_that_cannot_start_gives_its_code(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool).await;
    app.add_oidc_provider("gone", "http://127.0.0.1:1", CLIENT_ID, None, true)
        .await;

    // when
    let mut browser = Browser::new();
    let unknown = start(&app, &mut browser, "nobody", "").await;
    let gone = start(&app, &mut browser, "gone", "").await;

    // then
    assert_eq!(
        unknown.location,
        format!("{APP_URL}/sign-in-error?code=provider_off&provider=nobody")
    );
    assert_eq!(
        gone.location,
        format!("{APP_URL}/sign-in-error?code=provider_unavailable&provider=gone")
    );
}

/// The page is on the web origin of the start, and never on an origin that the URL names.
#[sqlx::test(migrations = false)]
async fn the_page_is_on_the_web_origin_of_the_start(pool: PgPool) {
    // given
    const WEB: &str = "https://web.example";
    let mut options = ServeOptions::default();
    options.accounts.web_origins = vec![WEB.to_owned()];
    let app = TestApp::spawn_with(pool, scylla_ce::edition(), options).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, true)
        .await;
    idp.refuse_sign_in("access_denied");

    // when
    let refused = Browser::new()
        .sign_in(&app, "idp", &format!("return_to={WEB}/acme"))
        .await;
    let mut browser = Browser::new();
    let outside = start(
        &app,
        &mut browser,
        "nobody",
        "return_to=https://evil.example/x",
    )
    .await;

    // then
    assert_eq!(
        refused.location,
        format!("{WEB}/sign-in-error?code=access_denied&provider=idp")
    );
    assert_eq!(
        outside.location,
        format!("{APP_URL}/sign-in-error?code=provider_off&provider=nobody")
    );
}
