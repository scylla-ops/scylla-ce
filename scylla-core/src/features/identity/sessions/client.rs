//! The client of a sign-in: its user agent and its IP address. The IP address is the peer
//! address, or the last `X-Forwarded-For` address, which the trusted proxy adds, when the settings
//! trust this header.

use std::net::{IpAddr, SocketAddr};

use axum::http::{Extensions, HeaderMap};
use tonic::transport::server::{TcpConnectInfo, TlsConnectInfo};
use tonic::Request;

/// The longest user agent that a session keeps, in characters.
const MAX_USER_AGENT: usize = 200;

/// The browsers and the marks of their user agents. The order is important: Edge, Opera and
/// Samsung Internet also send `Chrome/` and `Safari/`.
const BROWSERS: [(&str, &[&str]); 6] = [
    ("Edge", &["Edg/", "EdgA/", "EdgiOS/", "Edge/"]),
    ("Opera", &["OPR/", "OPT/", "OPiOS/", "Opera"]),
    ("Samsung Internet", &["SamsungBrowser/"]),
    ("Firefox", &["Firefox/", "FxiOS/"]),
    ("Chrome", &["Chrome/", "CriOS/"]),
    ("Safari", &["Safari/"]),
];

/// The systems and the marks of their user agents. The order is important: iOS sends
/// `Mac OS X`, and Android and ChromeOS send `Linux`.
const SYSTEMS: [(&str, &[&str]); 6] = [
    ("iOS", &["iPhone", "iPad", "iPod"]),
    ("Android", &["Android"]),
    ("ChromeOS", &["CrOS"]),
    ("Windows", &["Windows"]),
    ("macOS", &["Macintosh", "Mac OS X"]),
    ("Linux", &["Linux", "X11"]),
];

/// The client of a sign-in, as its session keeps it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Client {
    /// The user agent, at most 200 characters.
    pub user_agent: Option<String>,
    /// The IP address.
    pub ip: Option<IpAddr>,
}

impl Client {
    /// Reads the client of a gRPC request.
    pub fn of_grpc<T>(request: &Request<T>, trust_forwarded: bool) -> Self {
        let metadata = request.metadata();
        let text = |name: &str| metadata.get(name).and_then(|value| value.to_str().ok());
        Self::new(
            text("user-agent"),
            text("x-forwarded-for"),
            request.remote_addr(),
            trust_forwarded,
        )
    }

    /// Reads the client of a browser request.
    pub fn of_http(headers: &HeaderMap, extensions: &Extensions, trust_forwarded: bool) -> Self {
        let text = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let peer = extensions
            .get::<TcpConnectInfo>()
            .and_then(TcpConnectInfo::remote_addr)
            .or_else(|| {
                extensions
                    .get::<TlsConnectInfo<TcpConnectInfo>>()
                    .and_then(|info| info.get_ref().remote_addr())
            });
        Self::new(
            text("user-agent"),
            text("x-forwarded-for"),
            peer,
            trust_forwarded,
        )
    }

    /// Names the device for a person, as the web app does: `Firefox on macOS`, the product of an
    /// API client, or `Unknown device`.
    pub fn device(&self) -> String {
        let agent = self.user_agent.as_deref().unwrap_or("").trim();
        let find = |rules: &[(&'static str, &[&str])]| {
            rules
                .iter()
                .find(|(_, marks)| marks.iter().any(|mark| agent.contains(mark)))
                .map(|(name, _)| *name)
        };
        match (find(&BROWSERS), find(&SYSTEMS)) {
            _ if agent.is_empty() => "Unknown device".to_owned(),
            (Some(browser), Some(system)) => format!("{browser} on {system}"),
            (Some(browser), None) => browser.to_owned(),
            (None, _) => agent
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or(agent)
                .to_owned(),
        }
    }

    /// Names the device and the IP address for a person: `Firefox on macOS · 203.0.113.42`.
    pub fn describe(&self) -> String {
        match self.ip {
            Some(ip) => format!("{} · {ip}", self.device()),
            None => self.device(),
        }
    }

    /// Makes a client from its headers and its peer address. A forwarded address that is not
    /// valid gives the peer address.
    fn new(
        user_agent: Option<&str>,
        forwarded: Option<&str>,
        peer: Option<SocketAddr>,
        trust_forwarded: bool,
    ) -> Self {
        let forwarded = forwarded
            .filter(|_| trust_forwarded)
            .and_then(|value| value.rsplit(',').next())
            .and_then(|last| last.trim().parse::<IpAddr>().ok());
        let peer = peer.map(|peer| peer.ip().to_canonical());
        Self {
            user_agent: user_agent.map(|agent| agent.chars().take(MAX_USER_AGENT).collect()),
            ip: forwarded.or(peer),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests of the client of a sign-in.

    use std::net::{IpAddr, SocketAddr};

    use super::Client;

    /// The peer address of the tests.
    const PEER: &str = "10.0.0.9:4000";

    /// The address of the peer, for a check.
    fn peer() -> Option<SocketAddr> {
        Some(PEER.parse().unwrap())
    }

    /// The last forwarded address, which the proxy adds, replaces the peer address only when the
    /// settings trust the header. The client can write the other addresses.
    #[test]
    fn new_reads_the_last_forwarded_address_when_trusted() {
        // given
        let forwarded = Some("198.51.100.9, 203.0.113.7");

        // when
        let trusted = Client::new(None, forwarded, peer(), true);
        let untrusted = Client::new(None, forwarded, peer(), false);

        // then
        assert_eq!(trusted.ip, Some("203.0.113.7".parse::<IpAddr>().unwrap()));
        assert_eq!(untrusted.ip, Some("10.0.0.9".parse::<IpAddr>().unwrap()));
    }

    /// A forwarded value that is not an address gives the peer address, and a long user agent
    /// is cut.
    #[test]
    fn new_falls_back_to_the_peer_and_cuts_the_user_agent() {
        // given
        let agent = "a".repeat(300);

        // when
        let client = Client::new(Some(&agent), Some("unknown"), peer(), true);

        // then
        assert_eq!(client.ip, Some("10.0.0.9".parse::<IpAddr>().unwrap()));
        assert_eq!(client.user_agent.map(|agent| agent.len()), Some(200));
    }

    /// A browser gets its name and its system, an Edge that also sends `Chrome/` is Edge, an
    /// API client gets its product, and no user agent is an unknown device.
    #[test]
    fn device_names_the_browser_and_the_system() {
        // given
        let firefox = "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.5; rv:132.0) Gecko/20100101 \
                       Firefox/132.0";
        let edge = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like \
                    Gecko) Chrome/131.0 Safari/537.36 Edg/131.0";
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 \
                      (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1";

        // when
        let device = |agent: Option<&str>| {
            let client = Client::new(agent, None, peer(), false);
            (client.device(), client.describe())
        };

        // then
        let firefox = device(Some(firefox));
        assert_eq!(firefox.0, "Firefox on macOS");
        assert_eq!(firefox.1, "Firefox on macOS · 10.0.0.9");
        assert_eq!(device(Some(edge)).0, "Edge on Windows");
        assert_eq!(device(Some(iphone)).0, "Safari on iOS");
        assert_eq!(device(Some("grpc-python/1.66.0")).0, "grpc-python");
        assert_eq!(device(None).0, "Unknown device");
    }
}
