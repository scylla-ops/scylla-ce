//! The browser routes of a sign-in with an identity provider: `/auth/<slug>/start` sends the
//! browser to the provider, and `/auth/<slug>/callback` opens the session in a cookie.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use scylla_domain::identity::actors::ActorId;
use scylla_extension::AccessError;
use serde::Deserialize;

use super::service::{Outcome, Providers};
use crate::features::identity::sessions::{self, cookie};
use crate::Error;

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
    let link_user = match query.link {
        true => match signed_in(&providers, &headers).await {
            Some(user) => Some(user),
            None => return failure(&unauthenticated("sign in first to link a provider")),
        },
        false => None,
    };
    let secure = providers.ctx.settings.secure_cookies();
    match providers
        .start(&slug, query.return_to.as_deref(), link_user)
        .await
    {
        Ok((url, browser)) => redirect(
            &url,
            [cookie::set(cookie::FLOW, &browser, FLOW_TTL, secure)],
        ),
        Err(error) => failure(&error),
    }
}

/// Ends a sign-in at the callback of the provider: the session goes in the session cookie, and
/// the browser goes back to the web app.
async fn callback(
    State(providers): State<Providers>,
    Path(slug): Path<String>,
    Query(query): Query<CallbackQuery>,
    headers: HeaderMap,
) -> Response {
    // Refuse a callback with an error or with no code.
    if query.error.is_some() {
        return failure(&unauthenticated(
            "the identity provider refused the sign-in",
        ));
    }
    let (Some(code), Some(state)) = (query.code, query.state) else {
        return failure(&Error::invalid("code", "is required"));
    };

    // End the sign-in for this browser.
    let browser = cookie::read(cookies(&headers), cookie::FLOW);
    let signed_in = signed_in(&providers, &headers).await;
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok());
    let callback = providers.callback(&slug, &code, &state, browser.as_deref(), signed_in, agent);
    let settings = &providers.ctx.settings;
    let clear = cookie::clear(cookie::FLOW, settings.secure_cookies());
    match callback.await {
        Ok(Outcome::SignedIn { opened, return_to }) => {
            let session = cookie::set(
                cookie::SESSION,
                &opened.refresh,
                settings.session_ttl,
                settings.secure_cookies(),
            );
            redirect(&return_to, [clear, session])
        }
        Ok(Outcome::Linked { return_to }) => redirect(&return_to, [clear]),
        Err(error) => failure(&error),
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

/// Answers a refusal with its HTTP status and a short text. A server failure goes to the log.
fn failure(error: &Error) -> Response {
    let status = match error {
        Error::InvalidArgument { .. } => StatusCode::BAD_REQUEST,
        Error::Access(AccessError::Unauthenticated(_)) => StatusCode::UNAUTHORIZED,
        Error::Access(AccessError::Denied(_)) => StatusCode::FORBIDDEN,
        Error::Access(AccessError::Unavailable(_)) => StatusCode::SERVICE_UNAVAILABLE,
        Error::NotFound(_) => StatusCode::NOT_FOUND,
        Error::Rejected(_) => StatusCode::CONFLICT,
        error => {
            tracing::error!(%error, "a sign-in failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response();
        }
    };
    let text = match error {
        Error::Rejected(rejection) => format!("{}: {}", rejection.code, rejection.message),
        error => error.to_string(),
    };
    (status, text).into_response()
}

/// Makes an `Unauthenticated` error.
fn unauthenticated(reason: &str) -> Error {
    AccessError::Unauthenticated(reason.to_owned()).into()
}
