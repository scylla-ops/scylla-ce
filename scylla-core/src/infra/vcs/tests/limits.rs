//! Unit tests of an answer of a provider: the limits in its headers, its errors, the link of its
//! next page, and the hosts that a call can reach.

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::StatusCode;
use time::{Duration, OffsetDateTime};

use crate::infra::vcs::{next_link, ApiClient, Flavor, ProviderError};

/// Makes headers from names and values.
fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for (name, value) in pairs {
        let value = HeaderValue::from_str(value).expect("header value");
        headers.append(HeaderName::from_static(name), value);
    }
    headers
}

/// Returns the error of an answer at a fixed time.
fn error_of(status: u16, pairs: &[(&'static str, &str)], body: &str) -> ProviderError {
    let status = StatusCode::from_u16(status).expect("status");
    ProviderError::of_answer(status, &headers(pairs), body.as_bytes(), now())
}

/// A fixed time.
fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1_800_000_000).expect("time")
}

/// `retry-after` gives the end of the limit, before the other headers.
#[test]
fn of_answer_waits_the_seconds_of_retry_after() {
    // when
    let error = error_of(
        429,
        &[("retry-after", "30"), ("x-ratelimit-remaining", "0")],
        "",
    );

    // then
    assert_eq!(
        error,
        ProviderError::RateLimited(now() + Duration::seconds(30))
    );
    assert!(error.is_retryable());
}

/// With no call left, the limit ends at `x-ratelimit-reset` (GitHub).
#[test]
fn of_answer_waits_for_the_reset_of_github() {
    // when
    let error = error_of(
        403,
        &[
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", "1800000600"),
        ],
        r#"{"message": "API rate limit exceeded"}"#,
    );

    // then
    let reset = OffsetDateTime::from_unix_timestamp(1_800_000_600).expect("time");
    assert_eq!(error, ProviderError::RateLimited(reset));
}

/// With no call left, the limit ends at `ratelimit-reset` (GitLab).
#[test]
fn of_answer_waits_for_the_reset_of_gitlab() {
    // when
    let error = error_of(
        429,
        &[
            ("ratelimit-remaining", "0"),
            ("ratelimit-reset", "1800000120"),
        ],
        "",
    );

    // then
    let reset = OffsetDateTime::from_unix_timestamp(1_800_000_120).expect("time");
    assert_eq!(error, ProviderError::RateLimited(reset));
}

/// A secondary limit of GitHub with no time in the headers waits one minute.
#[test]
fn of_answer_waits_one_minute_after_a_secondary_limit() {
    // when
    let error = error_of(
        403,
        &[("x-ratelimit-remaining", "4000")],
        r#"{"message": "You have exceeded a secondary rate limit."}"#,
    );

    // then
    assert_eq!(
        error,
        ProviderError::RateLimited(now() + Duration::minutes(1))
    );
}

/// A reset in the past waits one minute.
#[test]
fn of_answer_waits_one_minute_after_a_past_reset() {
    // when
    let error = error_of(
        403,
        &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1")],
        "",
    );

    // then
    assert_eq!(
        error,
        ProviderError::RateLimited(now() + Duration::minutes(1))
    );
}

/// The other statuses give their errors, and only a transient failure can be sent again.
#[test]
fn of_answer_maps_the_other_statuses() {
    // when
    let unauthorized = error_of(401, &[], r#"{"message": "Bad credentials"}"#);
    let forbidden = error_of(403, &[("x-ratelimit-remaining", "10")], "{}");
    let not_found = error_of(404, &[], "");
    let gone = error_of(410, &[], "");
    let invalid = error_of(422, &[], r#"{"message": "Validation Failed"}"#);
    let gitlab = error_of(400, &[], r#"{"message": {"name": ["is taken"]}}"#);
    let transient = error_of(502, &[], "<html>bad gateway</html>");

    // then
    assert_eq!(unauthorized, ProviderError::Unauthorized);
    assert_eq!(forbidden, ProviderError::Forbidden);
    assert_eq!(not_found, ProviderError::NotFound);
    assert_eq!(gone, ProviderError::NotFound);
    assert_eq!(
        invalid,
        ProviderError::Invalid("Validation Failed".to_owned())
    );
    assert_eq!(
        gitlab,
        ProviderError::Invalid(r#"{"name":["is taken"]}"#.to_owned())
    );
    assert_eq!(transient, ProviderError::Transient("status 502".to_owned()));
    assert!(transient.is_retryable());
    assert!(!forbidden.is_retryable());
}

/// The next page is the link with `rel="next"`, from GitHub and from a GitLab keyset page, and
/// the last page has none.
#[test]
fn next_link_reads_the_link_header() {
    // given
    let github = headers(&[(
        "link",
        "<https://api.github.com/installation/repositories?page=2>; rel=\"next\", \
         <https://api.github.com/installation/repositories?page=5>; rel=\"last\"",
    )]);
    let gitlab = headers(&[(
        "link",
        "<https://gitlab.com/api/v4/projects?id_after=42&pagination=keyset>; rel=\"next\"",
    )]);
    let last = headers(&[(
        "link",
        "<https://api.github.com/x?page=1>; rel=\"first\", \
         <https://api.github.com/x?page=4>; rel=\"prev\"",
    )]);

    // when
    let pages = [next_link(&github), next_link(&gitlab), next_link(&last)];

    // then
    assert_eq!(
        pages,
        [
            Some("https://api.github.com/installation/repositories?page=2".to_owned()),
            Some("https://gitlab.com/api/v4/projects?id_after=42&pagination=keyset".to_owned()),
            None,
        ]
    );
}

/// A call goes below the base URL, or to a link of the same origin, and never to another host.
#[test]
fn url_keeps_the_calls_on_the_host_of_the_api() {
    // given
    crate::infra::tls::install();
    let gitlab = ApiClient::new(Flavor::Json, "https://gitlab.com/api/v4/").expect("client");

    // when
    let path = gitlab.url("/projects/1").expect("path");
    let link = gitlab.url("https://gitlab.com/api/v4/projects?page=2");
    let other = gitlab.url("https://evil.example/api/v4/projects?page=2");

    // then
    assert_eq!(path.as_str(), "https://gitlab.com/api/v4/projects/1");
    assert!(link.is_ok());
    assert!(matches!(other, Err(ProviderError::Invalid(_))));
}
