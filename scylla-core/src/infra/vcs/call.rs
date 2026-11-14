//! A call to a Git provider: its credentials, its query, its JSON body, the ETag of a read, the
//! errors of its answer, and the pages of a list (the `Link` header of the offset and the keyset
//! pages).

use reqwest::header::{HeaderMap, HeaderValue, ETAG, IF_NONE_MATCH, LINK};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use time::OffsetDateTime;

use super::{ApiClient, Auth, Hidden, ProviderError};

/// The most pages that `list` reads.
const MAX_PAGES: usize = 100;

/// A call that is not sent yet.
pub struct Call<'a> {
    /// The client of the API.
    client: &'a ApiClient,
    /// The HTTP method.
    method: Method,
    /// A path below the base URL, or a full URL of the same origin.
    target: String,
    /// The credentials.
    auth: Auth<'a>,
    /// The query parameters, added to the query of the target.
    query: Vec<(String, String)>,
    /// The JSON body.
    body: Option<Value>,
    /// The ETag of the last read, for `If-None-Match`.
    etag: Option<String>,
}

/// The answer of a call.
#[derive(Debug)]
pub enum Answer<T> {
    /// A new body.
    Fresh {
        /// The body.
        body: T,
        /// The ETag of the body, for the next read.
        etag: Option<String>,
        /// The URL of the next page of a list.
        next: Option<String>,
    },
    /// The body did not change since the ETag of the call (304).
    NotModified,
}

impl<'a> Call<'a> {
    /// Starts a call with no credentials.
    pub(super) fn new(client: &'a ApiClient, method: Method, target: &str) -> Self {
        Self {
            client,
            method,
            target: target.to_owned(),
            auth: Auth::None,
            query: Vec::new(),
            body: None,
            etag: None,
        }
    }

    /// Sets the credentials.
    pub fn auth(mut self, auth: Auth<'a>) -> Self {
        self.auth = auth;
        self
    }

    /// Adds a query parameter.
    pub fn query(mut self, name: &str, value: impl ToString) -> Self {
        self.query.push((name.to_owned(), value.to_string()));
        self
    }

    /// Asks for keyset pages of GitLab, in the order of a column.
    pub fn keyset(self, order_by: &str) -> Self {
        self.query("pagination", "keyset")
            .query("order_by", order_by)
            .query("sort", "asc")
    }

    /// Sets the JSON body.
    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    /// Sends `If-None-Match` with the ETag of the last read, if there is one.
    pub fn if_none_match(mut self, etag: Option<&str>) -> Self {
        self.etag = etag.map(str::to_owned);
        self
    }

    /// Sends the call, and reads the JSON body of a success. An empty body reads as `null`.
    pub async fn send<T: DeserializeOwned>(self) -> Result<Answer<T>, ProviderError> {
        // Make the request: the URL, the headers of the API, the credentials and the body.
        let mut url = self.client.url(&self.target)?;
        if !self.query.is_empty() {
            url.query_pairs_mut().extend_pairs(&self.query);
        }
        let mut request = self
            .client
            .http
            .request(self.method, url)
            .headers(self.client.headers());
        request = match self.auth {
            Auth::None => request,
            Auth::Bearer(token) => request.bearer_auth(token.expose()),
            Auth::PrivateToken(token) => request.header("private-token", sensitive(token)?),
        };
        if let Some(etag) = &self.etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        if let Some(body) = &self.body {
            request = request.json(body);
        }

        // Send it, and map a failure.
        let response = request.send().await.map_err(ProviderError::of_transport)?;
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Ok(Answer::NotModified);
        }
        let headers = response.headers().clone();
        let bytes = response
            .bytes()
            .await
            .map_err(ProviderError::of_transport)?;
        if !status.is_success() {
            let now = OffsetDateTime::now_utc();
            return Err(ProviderError::of_answer(status, &headers, &bytes, now));
        }

        // Read the body, its ETag and the link of the next page.
        let json = if bytes.is_empty() {
            &b"null"[..]
        } else {
            &bytes[..]
        };
        let body = serde_json::from_slice(json)
            .map_err(|error| ProviderError::Invalid(format!("the answer is not valid: {error}")))?;
        let etag = headers
            .get(ETAG)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        Ok(Answer::Fresh {
            body,
            etag,
            next: next_link(&headers),
        })
    }

    /// Sends the call, and reads the body of a success.
    pub async fn read<T: DeserializeOwned>(self) -> Result<T, ProviderError> {
        self.send().await?.into_body()
    }

    /// Reads each page of a list, with the links of the next pages. The items are the body, or
    /// the array at `field` of the body (GitHub `/installation/repositories`).
    pub async fn list<T: DeserializeOwned>(
        self,
        field: Option<&str>,
    ) -> Result<Vec<T>, ProviderError> {
        let (client, auth) = (self.client, self.auth);
        let mut items = Vec::new();
        let mut call = self;
        for _ in 0..MAX_PAGES {
            // Read a page, and its items.
            let Answer::Fresh { body, next, .. } = call.send::<Value>().await? else {
                return Err(not_modified());
            };
            let page = match field {
                Some(field) => body.get(field).cloned().unwrap_or(Value::Null),
                None => body,
            };
            let page: Vec<T> = serde_json::from_value(page)
                .map_err(|error| ProviderError::Invalid(format!("a page is not valid: {error}")))?;
            items.extend(page);

            // Go to the next page, or end at the last page.
            match next {
                Some(next) => call = client.get(&next).auth(auth),
                None => return Ok(items),
            }
        }
        Err(ProviderError::Invalid(format!(
            "the list has more than {MAX_PAGES} pages"
        )))
    }
}

impl<T> Answer<T> {
    /// Returns the body, or an error for an answer with no body (304).
    pub fn into_body(self) -> Result<T, ProviderError> {
        match self {
            Self::Fresh { body, .. } => Ok(body),
            Self::NotModified => Err(not_modified()),
        }
    }
}

/// Returns the URL of the next page in the `Link` headers: `<url>; rel="next"`.
pub fn next_link(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(LINK)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .find_map(|part| {
            let (target, params) = part.trim().split_once(';')?;
            let target = target.trim().strip_prefix('<')?.strip_suffix('>')?;
            let next = params
                .split(';')
                .filter_map(|param| param.trim().strip_prefix("rel="))
                .any(|rel| {
                    rel.trim_matches('"')
                        .split_whitespace()
                        .any(|rel| rel == "next")
                });
            next.then(|| target.to_owned())
        })
}

/// Makes a header value of a token that the HTTP stack never logs.
fn sensitive(token: &Hidden) -> Result<HeaderValue, ProviderError> {
    let mut value = HeaderValue::from_str(token.expose())
        .map_err(|_| ProviderError::Invalid("a token is not a valid header".to_owned()))?;
    value.set_sensitive(true);
    Ok(value)
}

/// The error of an answer with no body where a body is necessary.
fn not_modified() -> ProviderError {
    ProviderError::Invalid("the answer has no body: not modified".to_owned())
}
