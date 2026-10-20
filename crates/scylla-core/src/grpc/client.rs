//! The client of a call, as a new session records it.

use crate::domain::session::SessionClient;
use tonic::Request;
use tonic::metadata::MetadataMap;

/// The user agent is the `user-agent` header. With `trust_forwarded_headers`, the IP address is
/// the first entry of `x-forwarded-for`, else `x-real-ip`; a call that has neither gives the
/// peer address. Without it, the IP address is always the peer address of the connection: a
/// client can write these headers. The tonic transport server gives the peer address of a TCP
/// connection and of a TLS connection.
pub fn session_client<T>(request: &Request<T>, trust_forwarded_headers: bool) -> SessionClient {
    let metadata = request.metadata();
    let forwarded = if trust_forwarded_headers {
        header(metadata, "x-forwarded-for")
            .map(|list| list.split(',').next().unwrap_or_default().to_owned())
            .or_else(|| header(metadata, "x-real-ip").map(str::to_owned))
    } else {
        None
    };
    let ip_address = forwarded.or_else(|| request.remote_addr().map(|addr| addr.ip().to_string()));
    SessionClient::new(header(metadata, "user-agent"), ip_address.as_deref())
}

fn header<'a>(metadata: &'a MetadataMap, name: &str) -> Option<&'a str> {
    metadata.get(name).and_then(|value| value.to_str().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::session::USER_AGENT_MAX_CHARS;
    use std::net::{IpAddr, SocketAddr};
    use tonic::transport::server::TcpConnectInfo;

    const PEER: &str = "192.0.2.10";

    fn call(headers: &[(&'static str, &str)], peer: Option<&str>) -> Request<()> {
        let mut request = Request::new(());
        for (name, value) in headers {
            request.metadata_mut().insert(*name, value.parse().unwrap());
        }
        if let Some(peer) = peer {
            request.extensions_mut().insert(TcpConnectInfo {
                local_addr: None,
                remote_addr: Some(SocketAddr::new(peer.parse().unwrap(), 50_000)),
            });
        }
        request
    }

    fn ip(value: &str) -> Option<IpAddr> {
        Some(value.parse().unwrap())
    }

    #[test]
    fn an_untrusted_call_gives_the_peer_address_and_ignores_the_forwarded_headers() {
        let request = call(
            &[
                ("user-agent", "Mozilla/5.0 (X11; Linux x86_64)"),
                ("x-forwarded-for", "203.0.113.7"),
                ("x-real-ip", "203.0.113.8"),
            ],
            Some(PEER),
        );

        let client = session_client(&request, false);

        assert_eq!(client.user_agent(), Some("Mozilla/5.0 (X11; Linux x86_64)"));
        assert_eq!(client.ip_address(), ip(PEER));
    }

    #[test]
    fn a_trusted_call_gives_the_first_forwarded_entry() {
        let request = call(
            &[
                ("x-forwarded-for", " 203.0.113.7 , 10.0.0.2, 10.0.0.3"),
                ("x-real-ip", "203.0.113.8"),
            ],
            Some(PEER),
        );

        assert_eq!(
            session_client(&request, true).ip_address(),
            ip("203.0.113.7")
        );
    }

    #[test]
    fn a_trusted_call_without_a_forwarded_for_gives_the_real_ip_then_the_peer() {
        let real = call(&[("x-real-ip", "2001:db8::7")], Some(PEER));
        assert_eq!(session_client(&real, true).ip_address(), ip("2001:db8::7"));

        let direct = call(&[], Some(PEER));
        assert_eq!(session_client(&direct, true).ip_address(), ip(PEER));
    }

    #[test]
    fn a_forwarded_value_that_is_no_ip_address_gives_none() {
        for value in ["unknown", "203.0.113.7:443", ", 203.0.113.7", "not an ip"] {
            let request = call(&[("x-forwarded-for", value)], Some(PEER));
            assert_eq!(session_client(&request, true).ip_address(), None, "{value}");
        }
        let request = call(&[("x-real-ip", "999.0.0.1")], Some(PEER));
        assert_eq!(session_client(&request, true).ip_address(), None);
    }

    #[test]
    fn a_call_without_a_peer_or_a_header_gives_nothing() {
        let client = session_client(&call(&[], None), false);

        assert_eq!(client.user_agent(), None);
        assert_eq!(client.ip_address(), None);
    }

    #[test]
    fn a_long_user_agent_is_cut() {
        let long = "a".repeat(USER_AGENT_MAX_CHARS * 2);
        let request = call(&[("user-agent", &long)], None);

        let client = session_client(&request, false);

        assert_eq!(
            client.user_agent().map(str::len),
            Some(USER_AGENT_MAX_CHARS)
        );
    }
}
