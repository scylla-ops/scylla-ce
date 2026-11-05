//! The browser routes of a sign-in with an identity provider: `/auth/<slug>/start` sends the
//! browser to the provider, and `/auth/<slug>/callback` opens the session in a cookie. A failure
//! sends the browser to the page `sign-in-error` of the web app, with a code.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, Extensions, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use scylla_domain::identity::actors::ActorId;
use serde::Deserialize;

use super::failure::{code_of_provider, report};
use super::service::{Outcome, Providers};
use crate::features::identity::sessions::{self, cookie, Client};

/// The lifetime of the cookie of a sign-in.
const FLOW_TTL: Duration = Duration::from_secs(10 * 60);

/// The query of `/auth/<slug>/start`.
#[derive(Deserialize)]
struct StartQuery {
    /// The page of the web app after the sign-in.
    return_to: Option<String>,
    /// `true` to link the provider to the signed-in user.
    #[serde(default)]
    link: bool,
}

/// The query of `/auth/<slug>/callback`.
#[derive(Deserialize)]
struct CallbackQuery {
    /// The code to exchange.
    code: Option<String>,
    /// The state of the sign-in.
    state: Option<String>,
    /// The error of the provider, if it refused.
    error: Option<String>,
}

/// Makes the routes.
pub fn router(providers: Providers) -> Router {
    Router::new()
        .route("/auth/{slug}/start", get(start))
        .route("/auth/{slug}/callback", get(callback))
        .with_state(providers)
}

/// Starts a sign-in, or a link for the user of the session cookie, and sends the browser to the
/// provider.
async fn start(
    State(providers): State<Providers>,
    Path(slug): Path<String>,
    Query(query): Query<StartQuery>,
    headers: HeaderMap,
) -> Response {
    let settings = &providers.ctx.settings;
    let web = settings.web_root_of(query.return_to.as_deref().unwrap_or_default());
    let link_user = match query.link {
        true => match signed_in(&providers, &headers).await {
            Some(user) => Some(user),
            None => return error_page(web, "session_required", &slug, true, []),
        },
        false => None,
    };
    let secure = settings.secure_cookies();
    match providers
        .start(&slug, query.return_to.as_deref(), link_user)
        .await
    {
        Ok((url, browser)) => redirect(
            &url,
            [cookie::set(cookie::FLOW, &browser, FLOW_TTL, secure)],
        ),
        Err(error) => error_page(web, report(&error), &slug, query.link, []),
    }
}

/// Ends a sign-in at the callback of the provider: the session goes in the session cookie, and
/// the browser goes back to the web app.
async fn callback(
    State(providers): State<Providers>,
    Path(slug): Path<String>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
    extensions: Extensions,
) -> Response {
    // Take the sign-in of the state, for this browser.
    let settings = &providers.ctx.settings;
    let clear = cookie::clear(cookie::FLOW, settings.secure_cookies());
    let browser = cookie::read(cookies(&headers), cookie::FLOW);
    let flow = match providers
        .take_flow(query.state.as_deref(), browser.as_deref())
        .await
    {
        Ok(flow) => flow,
        Err(error) => return error_page(&settings.app_url, report(&error), &slug, false, [clear]),
    };
    let web = settings.web_root_of(&flow.return_to);
    let link = flow.link_user.is_some();

    // Refuse a callback with an error of the provider, or with no code.
    if let Some(error) = &query.error {
        let code = code_of_provider(error);
        tracing::info!(
            event = "request.refused",
            code,
            "the provider refused a sign-in"
        );
        return error_page(web, code, &slug, link, [clear]);
    }
    let Some(code) = query.code else {
        return error_page(web, "provider_refused", &slug, link, [clear]);
    };

    // End the sign-in.
    let signed_in = signed_in(&providers, &headers).await;
    let client = Client::of_http(&headers, &extensions, settings.trust_forwarded);
    match providers
        .finish(&slug, &code, &flow, signed_in, &client)
        .await
    {
        Ok(Outcome::SignedIn(opened)) => {
            let session = cookie::set(
                cookie::SESSION,
                &opened.refresh,
                settings.session_ttl,
                settings.secure_cookies(),
            );
            redirect(&flow.return_to, [clear, session])
        }
        Ok(Outcome::Linked) => redirect(&flow.return_to, [clear]),
        Err(error) => error_page(web, report(&error), &slug, link, [clear]),
    }
}

/// Returns the user of the session cookie, if it is valid.
async fn signed_in(providers: &Providers, headers: &HeaderMap) -> Option<ActorId> {
    let refresh = cookie::read(cookies(headers), cookie::SESSION)?;
    sessions::user_of(&providers.ctx.pool, &refresh)
        .await
        .ok()
        .flatten()
}

/// Returns the values of the `cookie` headers.
fn cookies(headers: &HeaderMap) -> impl Iterator<Item = &str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
}

/// Sends the browser to a URL, with cookies.
fn redirect(to: &str, cookies: impl IntoIterator<Item = String>) -> Response {
    let mut response = StatusCode::SEE_OTHER.into_response();
    let headers = response.headers_mut();
    if let Ok(location) = HeaderValue::from_str(to) {
        headers.insert(header::LOCATION, location);
    }
    for value in cookies {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.append(header::SET_COOKIE, value);
        }
    }
    response
}

/// Sends the browser to the page of the web app for a failed sign-in. The URL has the code and the
/// slug only: a person's data never goes in it.
fn error_page(
    web: &str,
    code: &str,
    slug: &str,
    link: bool,
    cookies: impl IntoIterator<Item = String>,
) -> Response {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query
        .append_pair("code", code)
        .append_pair("provider", slug);
    if link {
        query.append_pair("link", "true");
    }
    redirect(&format!("{web}/sign-in-error?{}", query.finish()), cookies)
}
