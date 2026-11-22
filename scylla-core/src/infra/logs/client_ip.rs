//! The rule of the client IP address in the log of each call: the full address, its network, or
//! no address.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// What the log of each call keeps of the client IP address, from `SCYLLA_LOG_CLIENT_IP`. The
/// sessions keep the full address.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum LogClientIp {
    /// The full address.
    Full,
    /// The network of the address: the first 24 bits of an IPv4 address, and the first 48 bits
    /// of an IPv6 address. The other bits are zero.
    #[default]
    Truncated,
    /// No address.
    Off,
}

impl LogClientIp {
    /// Returns the address that the log shows, or `None`.
    pub fn show(self, ip: IpAddr) -> Option<IpAddr> {
        match (self, ip.to_canonical()) {
            (Self::Full, _) => Some(ip),
            (Self::Off, _) => None,
            (Self::Truncated, IpAddr::V4(ip)) => {
                let [a, b, c, _] = ip.octets();
                Some(IpAddr::V4(Ipv4Addr::new(a, b, c, 0)))
            }
            (Self::Truncated, IpAddr::V6(ip)) => {
                let [a, b, c, ..] = ip.segments();
                Some(IpAddr::V6(Ipv6Addr::new(a, b, c, 0, 0, 0, 0, 0)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests of the rule of the client IP address.

    use std::net::IpAddr;

    use super::LogClientIp;

    /// Reads an address of a test.
    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    /// The full rule keeps the address as it is.
    #[test]
    fn full_keeps_the_address() {
        // given
        let address = ip("203.0.113.42");

        // when
        let shown = LogClientIp::Full.show(address);

        // then
        assert_eq!(shown, Some(address));
    }

    /// The truncated rule keeps the first 24 bits of an IPv4 address and the first 48 bits of an
    /// IPv6 address. An IPv4 address in an IPv6 address is an IPv4 address.
    #[test]
    fn truncated_keeps_the_network_of_the_address() {
        // given
        let addresses = [
            "203.0.113.42",
            "2001:db8:85a3:8d3:1319:8a2e:370:7348",
            "::ffff:198.51.100.7",
        ];

        // when
        let shown: Vec<_> = addresses
            .iter()
            .map(|text| LogClientIp::Truncated.show(ip(text)))
            .collect();

        // then
        let expected = [ip("203.0.113.0"), ip("2001:db8:85a3::"), ip("198.51.100.0")];
        assert_eq!(shown, expected.map(Some));
    }

    /// The off rule shows no address.
    #[test]
    fn off_shows_no_address() {
        // given
        let address = ip("2001:db8::1");

        // when
        let shown = LogClientIp::Off.show(address);

        // then
        assert_eq!(shown, None);
    }
}
