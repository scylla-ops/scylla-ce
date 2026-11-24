//! The tests of the link with a password at the callback: a verified email of an account waits for
//! the password, and no token and no email goes in a URL or a log.

use scylla_core::SignUpMode;
use scylla_proto::accounts::v1::account_service_client::AccountServiceClient;
use scylla_proto::accounts::v1::{
    GetPendingLinkRequest, GetPendingLinkResponse, LinkPendingIdentityRequest,
    LinkPendingIdentityResponse,
};
use scylla_testkit::{Logs, TestApp};
use sqlx::PgPool;
use tonic::{Code, Request, Response, Status};

use crate::identity::accounts::{app_in, assert_refused, PASSWORD};
use crate::identity::browser::Browser;
use crate::identity::idp::{person, Idp, CLIENT_ID};

/// The web app of the tests, the default of the settings.
pub(super) const APP_URL: &str = "http://127.0.0.1:3000";

/// The email of the person at the fake provider.
pub(super) const EMAIL: &str = "alice@idp.example";

/// One day, in seconds.
const ONE_DAY: f64 = 24.0 * 60.0 * 60.0;

/// Starts an app with the provider `idp`, which allows sign-up.
pub(super) async fn app_with_idp(pool: PgPool) -> (TestApp, Idp) {
    let app = app_in(pool, SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, true)
        .await;
    (app, idp)
}

/// Puts the cookies of a browser and the web origin on a request.
pub(super) fn from_browser<T>(browser: &Browser, message: T) -> Request<T> {
    let mut request = Request::new(message);
    let cookies: Vec<_> = browser
        .cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect();
    let metadata = request.metadata_mut();
    metadata.insert("cookie", cookies.join("; ").parse().unwrap());
    metadata.insert("origin", APP_URL.parse().unwrap());
    request
}

/// Calls `GetPendingLink` from a browser.
pub(super) async fn pending(
    app: &TestApp,
    browser: &Browser,
) -> Result<GetPendingLinkResponse, Status> {
    let request = from_browser(browser, GetPendingLinkRequest {});
    let response = AccountServiceClient::new(app.channel.clone())
        .get_pending_link(request)
        .await?;
    Ok(response.into_inner())
}

/// Calls `LinkPendingIdentity` from a browser.
pub(super) async fn link(
    app: &TestApp,
    browser: &Browser,
    password: &str,
) -> Result<Response<LinkPendingIdentityResponse>, Status> {
    let message = LinkPendingIdentityRequest {
        password: password.to_owned(),
    };
    AccountServiceClient::new(app.channel.clone())
        .link_pending_identity(from_browser(browser, message))
        .await
}

/// Returns the number of identities of the tests.
pub(super) async fn identities(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM external_identities")
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Returns the value of a cookie in the `set-cookie` values of a response.
fn set_cookie<T>(response: &Response<T>, name: &str) -> Option<String> {
    response
        .metadata()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(|value| {
            let pair = value.split(';').next()?;
            let (key, value) = pair.split_once('=')?;
            (key == name).then(|| value.to_owned())
        })
}

/// A verified email of an account with a password opens no session and goes to the page of the
/// link. The right password links the identity with a mail, and opens a session of 30 days with the
/// two methods. The provider then signs in the same user.
#[sqlx::test(migrations = false)]
async fn the_password_of_the_account_links_the_identity(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    let user = app.user_with(EMAIL, Some(PASSWORD)).await;
    let mut browser = Browser::new();

    // when
    let step = browser
        .sign_in(&app, "idp", &format!("return_to={APP_URL}/acme"))
        .await;
    let view = pending(&app, &browser).await.unwrap();
    let before = identities(&pool).await;
    let linked = link(&app, &browser, PASSWORD).await.unwrap();

    // then
    assert_eq!(step.location, format!("{APP_URL}/sign-in-link"));
    assert!(!browser.cookies.contains_key("scylla_session"));
    assert!(!browser.cookies.contains_key("scylla_flow"));
    assert_eq!(view.email, EMAIL);
    assert_eq!(view.provider_name, "idp");
    assert_eq!(before, 0);
    assert_eq!(identities(&pool).await, 1);
    assert_eq!(linked.get_ref().return_to, format!("{APP_URL}/acme"));
    assert_eq!(set_cookie(&linked, "scylla_pending").as_deref(), Some(""));
    let refresh = set_cookie(&linked, "scylla_session").unwrap();
    let token = app.refresh(&refresh).await.unwrap().token.unwrap().value;
    let me = app.get_me(&token).await.unwrap();
    assert_eq!(me.id.unwrap().value, user.to_string());
    let (methods, life): (Vec<String>, f64) = sqlx::query_as(
        "SELECT methods, EXTRACT(EPOCH FROM expires_at - created_at)::float8
         FROM sessions WHERE user_id = $1",
    )
    .bind(user.to_db())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(methods, ["password", "idp"]);
    assert!(life > 29.0 * ONE_DAY, "{life}");
    let mails = app.mails.wait(EMAIL, 1).await;
    assert!(mails[0].subject.contains("new sign-in method"));
    assert!(mails[0].subject.contains("idp"));

    // The provider now signs in the same user, with no password.
    let mut later = Browser::new();
    let again = later.sign_in(&app, "idp", "").await;
    assert_eq!(again.location, APP_URL);
    let refresh = &later.cookies["scylla_session"];
    let token = app.refresh(refresh).await.unwrap().token.unwrap().value;
    let me = app.get_me(&token).await.unwrap();
    assert_eq!(me.id.unwrap().value, user.to_string());
}

/// The database keeps only the hash of the token, and the URL and the logs have no token, no email
/// and no password.
#[sqlx::test(migrations = false)]
async fn no_token_and_no_email_go_in_a_url_or_a_log(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let logs = Logs::pretty();
    let mut browser = Browser::new();

    // when
    let step = browser.sign_in(&app, "idp", "").await;
    let token = browser.cookies["scylla_pending"].clone();
    let stored: String =
        sqlx::query_scalar("SELECT encode(token_hash, 'escape') FROM pending_links")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_refused(
        link(&app, &browser, "not the password at all").await,
        "WRONG_PASSWORD",
    );
    link(&app, &browser, PASSWORD).await.unwrap();
    app.mails.wait(EMAIL, 1).await;

    // then
    assert!(!step.location.contains(EMAIL));
    assert!(!step.location.contains(&token));
    assert!(!stored.contains(&token));
    let text = logs.text();
    assert!(!text.contains(EMAIL), "{text}");
    assert!(!text.contains(&token), "{text}");
    assert!(!text.contains(PASSWORD), "{text}");
    assert!(!text.contains("not the password at all"), "{text}");
}

/// A link works one time: the second use finds nothing, also with the right password.
#[sqlx::test(migrations = false)]
async fn a_link_works_one_time(pool: PgPool) {
    // given
    let (app, _idp) = app_with_idp(pool.clone()).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let mut browser = Browser::new();
    browser.sign_in(&app, "idp", "").await;

    // when
    link(&app, &browser, PASSWORD).await.unwrap();
    let second = link(&app, &browser, PASSWORD).await;
    let read = pending(&app, &browser).await;

    // then
    assert_eq!(second.unwrap_err().code(), Code::NotFound);
    assert_eq!(read.unwrap_err().code(), Code::NotFound);
    assert_eq!(identities(&pool).await, 1);
}

/// A provider that does not allow new accounts still offers the link: it makes no account, and it
/// always needs the password. A person with no account keeps the refusal, and an account with no
/// password too.
#[sqlx::test(migrations = false)]
async fn a_provider_with_no_sign_up_offers_the_link_only(pool: PgPool) {
    // given
    let app = app_in(pool.clone(), SignUpMode::Open).await;
    let idp = Idp::start().await;
    app.add_oidc_provider("idp", idp.url(), CLIENT_ID, None, false)
        .await;
    let users = "SELECT count(*) FROM users";
    let (nobody, no_password) = ("nobody@idp.example", "nopass@idp.example");
    app.user_with(no_password, None).await;
    app.user_with(EMAIL, Some(PASSWORD)).await;
    let before: i64 = sqlx::query_scalar(users).fetch_one(&pool).await.unwrap();

    // when
    idp.sign_in_as(person("nobody", nobody, true));
    let no_account = Browser::new().sign_in(&app, "idp", "").await;
    idp.sign_in_as(person("unverified", nobody, false));
    let unverified = Browser::new().sign_in(&app, "idp", "").await;
    idp.sign_in_as(person("nopass", no_password, true));
    let without_password = Browser::new().sign_in(&app, "idp", "").await;
    idp.sign_in_as(person("alice", EMAIL, true));
    let mut browser = Browser::new();
    let step = browser.sign_in(&app, "idp", "").await;
    let linked = link(&app, &browser, PASSWORD).await;

    // then
    assert_eq!(
        no_account.location,
        format!("{APP_URL}/sign-in-error?code=signup_disabled&provider=idp")
    );
    assert_eq!(
        unverified.location,
        format!("{APP_URL}/sign-in-error?code=signup_disabled&provider=idp")
    );
    assert_eq!(
        without_password.location,
        format!("{APP_URL}/sign-in-error?code=email_in_use&provider=idp")
    );
    assert_eq!(step.location, format!("{APP_URL}/sign-in-link"));
    assert!(linked.is_ok(), "{linked:?}");
    assert_eq!(identities(&pool).await, 1);
    let after: i64 = sqlx::query_scalar(users).fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);
}
