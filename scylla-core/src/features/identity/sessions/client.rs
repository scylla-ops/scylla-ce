//! The client of a sign-in: its user agent and its IP address. The IP address is the peer
//! address, or the last `X-Forwarded-For` address, which the trusted proxy adds, when the settings
//! trust this header.

use std::net::{IpAddr, SocketAddr};

use axum::http::{Extensions, HeaderMap};
use tonic::transport::server::{TcpConnectInfo, TlsConnectInfo};
use tonic::Request;

/// The longest user agent that a session keeps, in characters.
const MAX_USER_AGENT: usize = 200;

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
}
