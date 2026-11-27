//! The download: the check of the URL and of each address of its host, then one request with no
//! redirect and no credential, to the checked addresses only.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, USER_AGENT};
use reqwest::redirect::Policy;
use url::{Host, Url};

use super::address::is_public;

/// The time of a whole download, with the connection.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Where a download can go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// HTTPS to a public address only.
    Public,
    /// Also HTTP, and a loopback or private address: for the tests and a local server only.
    Local,
}

/// Why a download failed.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refusal {
    /// The address is not a URL with a host.
    #[error("the address is not a URL with a host")]
    NotAUrl,
    /// The URL does not use HTTPS.
    #[error("the URL does not use HTTPS")]
    NotHttps,
    /// The URL holds a user name or a password.
    #[error("the URL holds a credential")]
    Credential,
    /// The host has an address that is not public.
    #[error("the host has an address that is not public")]
    NotPublic,
    /// The host has no address.
    #[error("the host has no address")]
    Unresolved,
    /// The server sent a redirect, which the download does not follow.
    #[error("the server sent a redirect")]
    Redirect,
    /// The server answered with a status that is not a success.
    #[error("the server answered with the status {0}")]
    Status(u16),
    /// The content type is not one of the expected types.
    #[error("the content type is not an expected type")]
    ContentType,
    /// The file has more bytes than the limit.
    #[error("the file has more than {0} bytes")]
    TooLarge(usize),
    /// The request failed: no connection, a timeout or a TLS error.
    #[error("the request failed: {0}")]
    Failed(String),
}

/// Downloads a file of one of the content types `types`, with at most `max` bytes. With
/// `Reach::Public`, each address of the host must be public, and the request goes to these
/// addresses only, so a second resolution cannot give another one. The request has no cookie, no
/// token and no proxy, and it follows no redirect.
pub(crate) async fn download(
    url: &str,
    reach: Reach,
    types: &[&str],
    max: usize,
) -> Result<Vec<u8>, Refusal> {
    // Check the URL, then each address of its host.
    let url = Url::parse(url).map_err(|_| Refusal::NotAUrl)?;
    let local = reach == Reach::Local;
    match url.scheme() {
        "https" => {}
        "http" if local => {}
        _ => return Err(Refusal::NotHttps),
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(Refusal::Credential);
    }
    let addrs = resolve(&url).await?;
    if !local && !addrs.iter().all(|addr| is_public(addr.ip())) {
        return Err(Refusal::NotPublic);
    }

    // Send the request to the checked addresses.
    crate::infra::tls::install();
    let mut builder = reqwest::Client::builder()
        .redirect(Policy::none())
        .no_proxy()
        .timeout(TIMEOUT);
    if let Some(Host::Domain(domain)) = url.host() {
        builder = builder.resolve_to_addrs(domain, &addrs);
    }
    let client = builder.build().map_err(failed)?;
    let request = client.get(url).header(USER_AGENT, "scylla");
    let mut response = request.send().await.map_err(failed)?;
    let status = response.status();
    if status.is_redirection() {
        return Err(Refusal::Redirect);
    }
    if !status.is_success() {
        return Err(Refusal::Status(status.as_u16()));
    }

    // Check the content type and the announced size, then read the body up to the limit.
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(|value| value.trim().to_ascii_lowercase());
    if !content_type.is_some_and(|value| types.contains(&value.as_str())) {
        return Err(Refusal::ContentType);
    }
    let limit = u64::try_from(max).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit)
    {
        return Err(Refusal::TooLarge(max));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(failed)? {
        if body.len() + chunk.len() > max {
            return Err(Refusal::TooLarge(max));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Returns the addresses of the host of a URL, with the port of the URL.
async fn resolve(url: &Url) -> Result<Vec<SocketAddr>, Refusal> {
    let port = url.port_or_known_default().ok_or(Refusal::NotAUrl)?;
    let ip = match url.host().ok_or(Refusal::NotAUrl)? {
        Host::Ipv4(ip) => IpAddr::V4(ip),
        Host::Ipv6(ip) => IpAddr::V6(ip),
        Host::Domain(domain) => {
            let found = tokio::net::lookup_host((domain, port)).await;
            let addrs: Vec<SocketAddr> = found.map_err(|_| Refusal::Unresolved)?.collect();
            return match addrs.is_empty() {
                true => Err(Refusal::Unresolved),
                false => Ok(addrs),
            };
        }
    };
    Ok(vec![SocketAddr::new(ip, port)])
}

/// The refusal of a request that failed. It does not hold the URL.
fn failed(error: reqwest::Error) -> Refusal {
    Refusal::Failed(error.without_url().to_string())
}
