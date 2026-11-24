//! A browser for the tests: it keeps the cookies of the server and does not follow redirects.

use std::collections::HashMap;

use axum::http::header;
use scylla_testkit::TestApp;

/// The answer of one step of a browser.
pub(crate) struct Step {
    /// The HTTP status.
    pub(crate) status: u16,
    /// The `location` header of a redirect.
    pub(crate) location: String,
    /// The body.
    pub(crate) body: String,
}

/// A browser that keeps the cookies of the server and does not follow redirects.
pub(crate) struct Browser {
    /// The HTTP client.
    http: reqwest::Client,
    /// The cookies, by name.
    pub(crate) cookies: HashMap<String, String>,
}

impl Browser {
    /// Makes a browser with no cookie.
    pub(crate) fn new() -> Self {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        Self {
            http,
            cookies: HashMap::new(),
        }
    }

    /// Opens a URL with the cookies, and keeps the new cookies.
    pub(crate) async fn get(&mut self, url: &str) -> Step {
        let cookie: Vec<_> = self
            .cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        let response = self
            .http
            .get(url)
            .header("cookie", cookie.join("; "))
            .send()
            .await
            .unwrap();
        for value in response.headers().get_all(header::SET_COOKIE) {
            let pair = value.to_str().unwrap().split(';').next().unwrap();
            let (name, value) = pair.split_once('=').unwrap();
            match value.is_empty() {
                true => self.cookies.remove(name),
                false => self.cookies.insert(name.to_owned(), value.to_owned()),
            };
        }
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|value| value.to_str().unwrap().to_owned())
            .unwrap_or_default();
        Step {
            status,
            location,
            body: response.text().await.unwrap(),
        }
    }

    /// Signs in at a provider of the app, through the start, the provider and the callback, and
    /// returns the last step. `query` goes to the start route.
    pub(crate) async fn sign_in(&mut self, app: &TestApp, slug: &str, query: &str) -> Step {
        let start = self
            .get(&format!("http://{}/auth/{slug}/start?{query}", app.addr))
            .await;
        if start.status != 303 {
            return start;
        }
        let provider = self.get(&start.location).await;
        self.get(&provider.location).await
    }
}
