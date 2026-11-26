//! The addresses that a download can reach: only the public addresses of the internet. A loopback,
//! private, link-local, shared or reserved address can be a service of the network of the server,
//! as the metadata service of a cloud (`169.254.169.254`, `fd00:ec2::254`).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Returns `true` if an address is a public address of the internet. An IPv6 address that holds
/// an IPv4 address (mapped, compatible, NAT64, 6to4) is public only if that IPv4 address is.
pub(super) fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    }
}

/// Returns `true` if an IPv4 address is public.
fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    let reserved = a == 0
        || (a == 100 && (64..128).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b == 18 || b == 19))
        || a >= 240;
    !(reserved
        || ip.is_loopback()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_documentation())
}

/// Returns `true` if an IPv6 address is public.
fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = embedded_v4(ip) {
        return is_public_v4(v4);
    }
    let [first, second, ..] = ip.segments();
    let local =
        (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80 || (first & 0xffc0) == 0xfec0;
    let special = (first == 0x2001 && (second == 0 || second == 0x0db8)) || first == 0x0100;
    !(local || special || ip.is_unspecified() || ip.is_loopback() || ip.is_multicast())
}

/// Returns the IPv4 address in an IPv6 address: mapped (`::ffff:a.b.c.d`), compatible
/// (`::a.b.c.d`), NAT64 (`64:ff9b::a.b.c.d`) or 6to4 (`2002:aabb:ccdd::`).
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = ip.segments();
    let [hi, lo] = [segments[6], segments[7]].map(u16::to_be_bytes);
    let last = Ipv4Addr::new(hi[0], hi[1], lo[0], lo[1]);
    match segments {
        [0, 0, 0, 0, 0, 0xffff, ..] => Some(last),
        [0, 0, 0, 0, 0, 0, ..] if !ip.is_unspecified() && !ip.is_loopback() => Some(last),
        [0x0064, 0xff9b, 0, 0, 0, 0, ..] => Some(last),
        [0x2002, hi, lo, ..] => {
            let [a, b] = hi.to_be_bytes();
            let [c, d] = lo.to_be_bytes();
            Some(Ipv4Addr::new(a, b, c, d))
        }
        _ => None,
    }
}
