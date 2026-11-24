//! The generic REST client of the Git providers: JSON, the user agent, the API version of
//! GitHub, the timeouts, and the URL of a call. A credential never goes to another host.

use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use reqwest::Method;
use url::Url;

use super::{Call, Hidden, ProviderError};

/// The user agent of each call.
const USER_AGENT: &str = concat!("scylla/", env!("CARGO_PKG_VERSION"));

/// The time to open a connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The time of a whole call.
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// The version of the GitHub REST API.
const GITHUB_API_VERSION: &str = "2022-11-28";

/// The headers of an API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    /// The GitHub REST API: `application/vnd.github+json` and `X-GitHub-Api-Version`.
    GitHub,
    /// A plain JSON API: the GitLab REST API, and the web routes of GitHub.
    Json,
}

/// The credentials of a call. The client never logs them.
#[derive(Clone, Copy)]
pub enum Auth<'a> {
    /// No credentials.
    None,
    /// `Authorization: Bearer`: the JWT of a GitHub App, an installation token, a user token.
    Bearer(&'a Hidden),
    /// `PRIVATE-TOKEN`: the token of a GitLab service account.
    PrivateToken(&'a Hidden),
}

/// A REST client of one API.
#[derive(Clone, Debug)]
pub struct ApiClient {
    /// The HTTP client, with the user agent and the timeouts.
    pub(super) http: reqwest::Client,
    /// The base URL of the API.
    base: Url,
    /// The headers of the API.
    flavor: Flavor,
}

impl ApiClient {
    /// Makes a client of the API at `base`.
    pub fn new(flavor: Flavor, base: &str) -> Result<Self, String> {
        let base = Url::parse(base.trim_end_matches('/'))
            .map_err(|error| format!("{base:?} is not a URL: {error}"))?;
        let http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(CALL_TIMEOUT)
            .build()
            .map_err(|error| error.to_string())?;
        Ok(Self { http, base, flavor })
    }

    /// Returns the base URL, with no `/` at the end.
    pub fn base(&self) -> &str {
        self.base.as_str().trim_end_matches('/')
    }

    /// Starts a call to a path of the API (`/repos/o/r`), or to a full URL of the same origin,
    /// such as the link of a next page.
    pub fn call(&self, method: Method, target: &str) -> Call<'_> {
        Call::new(self, method, target)
    }

    /// Starts a `GET` call.
    pub fn get(&self, target: &str) -> Call<'_> {
        self.call(Method::GET, target)
    }

    /// Returns the URL of a target. A URL of another origin is refused: a credential never goes
    /// to another host.
    pub(super) fn url(&self, target: &str) -> Result<Url, ProviderError> {
        let invalid = |reason: &str| ProviderError::Invalid(reason.to_owned());
        if target.starts_with("http://") || target.starts_with("https://") {
            let url = Url::parse(target).map_err(|_| invalid("a link is not a URL"))?;
            if url.origin() != self.base.origin() {
                return Err(invalid("a link goes to another host"));
            }
            return Ok(url);
        }
        let path = target.trim_start_matches('/');
        Url::parse(&format!("{}/{path}", self.base())).map_err(|_| invalid("a path is not valid"))
    }

    /// Returns the headers of the API.
    pub(super) fn headers(&self) -> HeaderMap {
        let mut headers = HeaderMap::new();
        match self.flavor {
            Flavor::GitHub => {
                let accept = HeaderValue::from_static("application/vnd.github+json");
                headers.insert(ACCEPT, accept);
                let version = HeaderValue::from_static(GITHUB_API_VERSION);
                headers.insert("x-github-api-version", version);
            }
            Flavor::Json => {
                headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
            }
        }
        headers
    }
}
