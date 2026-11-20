//! A programmable fake of a Git provider: an HTTP server that gives the answers of a test and
//! records each request. The answers of a route serve the requests in order, and the last one
//! serves each later request.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router};
use scylla_core::infra::vcs::VcsOptions;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::keys::test_github_app;

/// A fake Git provider on a free port of the local host.
pub struct FakeProvider {
    /// The URL of the fake, such as `http://127.0.0.1:41234`.
    url: String,
    /// The answers and the requests.
    state: Arc<Mutex<Fake>>,
}

/// A request that the fake got.
#[derive(Clone, Debug)]
pub struct Recorded {
    /// The method, such as `POST`.
    pub method: String,
    /// The path, with no query.
    pub path: String,
    /// The query with no `?`, or an empty text.
    pub query: String,
    /// The headers, with names in lowercase.
    pub headers: BTreeMap<String, String>,
    /// The JSON body, a text for a body that is not JSON, or `null`.
    pub body: Value,
}

/// An answer that the fake gives.
#[derive(Clone)]
struct Answer {
    /// The HTTP status.
    status: StatusCode,
    /// The JSON body, or `null` for no body.
    body: Value,
    /// The headers.
    headers: Vec<(String, String)>,
}

/// The routes and the requests of a fake.
#[derive(Default)]
struct Fake {
    /// The answers of each method and path, in order. A path with a `?` has its query.
    routes: Vec<(String, String, VecDeque<Answer>)>,
    /// The requests, in order.
    requests: Vec<Recorded>,
}

impl FakeProvider {
    /// Starts a fake on a free port.
    pub async fn start() -> Self {
        let state = Arc::new(Mutex::new(Fake::default()));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("local addr"));
        let router = Router::new()
            .fallback(answer)
            .with_state(Arc::clone(&state));
        tokio::spawn(async move { axum::serve(listener, router).await.expect("fake serves") });
        Self { url, state }
    }

    /// Returns the URL of the fake.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the settings of a server that calls this fake for GitHub (API and web) and for
    /// GitLab (`/api/v4`), with the test GitHub App if `with_app`.
    pub fn options(&self, with_app: bool) -> VcsOptions {
        VcsOptions {
            github_api_url: self.url.clone(),
            github_web_url: self.url.clone(),
            gitlab_url: self.url.clone(),
            github_app: with_app.then(test_github_app),
        }
    }

    /// Adds an answer to a method and a path. A path with a `?` matches this query only, and a
    /// path with no `?` matches each query. A `null` body gives no body.
    pub fn respond(
        &self,
        method: &str,
        path: &str,
        status: u16,
        body: Value,
        headers: &[(&str, &str)],
    ) {
        // Make the answer.
        let answer = Answer {
            status: StatusCode::from_u16(status).expect("status"),
            body,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
        };

        // Add it to the answers of the route, or make the route.
        let mut fake = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let method = method.to_uppercase();
        match fake
            .routes
            .iter_mut()
            .find(|(known, route, _)| *known == method && route == path)
        {
            Some((_, _, answers)) => answers.push_back(answer),
            None => fake
                .routes
                .push((method, path.to_owned(), VecDeque::from([answer]))),
        }
    }

    /// Returns each request, in order.
    pub fn requests(&self) -> Vec<Recorded> {
        let fake = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        fake.requests.clone()
    }

    /// Returns the requests to a method and a path, with each query.
    pub fn requests_to(&self, method: &str, path: &str) -> Vec<Recorded> {
        self.requests()
            .into_iter()
            .filter(|request| request.method.eq_ignore_ascii_case(method) && request.path == path)
            .collect()
    }
}

impl Recorded {
    /// Returns the value of a header.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(&name.to_lowercase()).map(String::as_str)
    }
}

impl Fake {
    /// Takes the answer of a request: a route of its path and its query first, then a route of
    /// its path.
    fn take(&mut self, request: &Recorded) -> Option<Answer> {
        let full = format!("{}?{}", request.path, request.query);
        let find = |path: &str| {
            self.routes
                .iter()
                .position(|(method, route, _)| *method == request.method && route == path)
        };
        let index = find(&full).or_else(|| find(&request.path))?;
        let answers = &mut self.routes[index].2;
        match answers.len() {
            0 | 1 => answers.front().cloned(),
            _ => answers.pop_front(),
        }
    }
}

impl IntoResponse for Answer {
    fn into_response(self) -> Response {
        let mut response = match self.body {
            Value::Null => self.status.into_response(),
            body => (self.status, Json(body)).into_response(),
        };
        for (name, value) in self.headers {
            let name = HeaderName::from_bytes(name.as_bytes()).expect("header name");
            let value = HeaderValue::from_str(&value).expect("header value");
            response.headers_mut().append(name, value);
        }
        response
    }
}

/// Records a request and gives its answer, or 404 when no route matches.
async fn answer(
    State(state): State<Arc<Mutex<Fake>>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // Record the request.
    let recorded = Recorded {
        method: method.as_str().to_owned(),
        path: uri.path().to_owned(),
        query: uri.query().unwrap_or_default().to_owned(),
        headers: headers
            .iter()
            .map(|(name, value)| {
                let value = String::from_utf8_lossy(value.as_bytes()).into_owned();
                (name.as_str().to_owned(), value)
            })
            .collect(),
        body: match body.is_empty() {
            true => Value::Null,
            false => serde_json::from_slice(&body)
                .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned())),
        },
    };

    // Take its answer.
    let mut fake = state.lock().unwrap_or_else(PoisonError::into_inner);
    let answer = fake.take(&recorded);
    fake.requests.push(recorded);
    drop(fake);
    match answer {
        Some(answer) => answer.into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({ "message": "Not Found" })),
        )
            .into_response(),
    }
}
