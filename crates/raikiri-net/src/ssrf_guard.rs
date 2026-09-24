//! Non-overridable IP-address safety floor for SSRF-hardened `NetworkProvider`
//! backends in this crate.
//!
//! Pure, I/O-free classification: no DNS resolution happens here. A
//! `NetworkProvider` backend resolves a hostname itself (see
//! `crate::http_resolver::SsrfSafeResolver`) and passes each candidate
//! `IpAddr` through [`is_globally_routable`] before connecting. This applies
//! *underneath* `raikiri_traits::ResourcePolicy` — a policy that returns
//! `true` from `is_host_allowed` never bypasses this floor.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Returns `true` only if `ip` is safe to connect to when fetching a
/// Consumer-supplied resource URL: not private, loopback, link-local, CGNAT,
/// documentation, benchmarking, reserved, multicast, broadcast, or an
/// IPv4-mapped / NAT64-embedded address whose embedded IPv4 address itself
/// fails this same check.
///
/// This is the crate's SSRF safety floor: it is independent of
/// `raikiri_traits::ResourcePolicy` and cannot be loosened by one — a
/// policy that allows a host does not bypass this check.
pub fn is_globally_routable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_v4_globally_routable(v4),
        IpAddr::V6(v6) => is_v6_globally_routable(v6),
    }
}

fn is_v4_globally_routable(ip: Ipv4Addr) -> bool {
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || is_shared_address_space(ip)
        || is_ietf_protocol_assignment(ip)
        || is_6to4_relay_anycast(ip)
        || is_benchmarking(ip)
        || is_reserved(ip))
}

/// 100.64.0.0/10 (RFC 6598, "Shared Address Space" / CGNAT). Not covered by
/// any stable `Ipv4Addr` method — `is_shared` is still feature-gated `ip`
/// (rust-lang/rust#27709) as of this crate's pinned stable toolchain
/// (1.91.0). Alibaba Cloud's metadata endpoint (100.100.100.200) lives here.
fn is_shared_address_space(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 100 && (o[1] & 0b1100_0000) == 0b0100_0000
}

/// 192.0.0.0/24 (RFC 6890, IETF protocol assignments).
fn is_ietf_protocol_assignment(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 192 && o[1] == 0 && o[2] == 0
}

/// 192.88.99.0/24 (RFC 7526, deprecated 6to4 relay anycast — still routable).
fn is_6to4_relay_anycast(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 192 && o[1] == 88 && o[2] == 99
}

/// 198.18.0.0/15 (RFC 2544, benchmarking).
fn is_benchmarking(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    o[0] == 198 && (o[1] == 18 || o[1] == 19)
}

/// 240.0.0.0/4 (RFC 1112, reserved for future use; includes the
/// 255.255.255.255 broadcast address as its top address).
fn is_reserved(ip: Ipv4Addr) -> bool {
    ip.octets()[0] >= 240
}

fn is_v6_globally_routable(ip: Ipv6Addr) -> bool {
    if ip.is_loopback()
        || ip.is_multicast()
        || ip.is_unspecified()
        || ip.is_unique_local()
        || ip.is_unicast_link_local()
        || is_documentation_v6(ip)
    {
        return false;
    }
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_v4_globally_routable(mapped);
    }
    if let Some(embedded) = nat64_embedded_v4(ip) {
        return is_v4_globally_routable(embedded);
    }
    true
}

/// 64:ff9b::/96 (RFC 6052 NAT64 well-known prefix). The low 32 bits carry
/// the embedded IPv4 address, which must be re-checked rather than
/// blanket-blocking the whole prefix (it also carries legitimate public
/// destinations for NAT64 clients).
fn nat64_embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let seg = ip.segments();
    if seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2..6] == [0, 0, 0, 0] {
        let o = ip.octets();
        Some(Ipv4Addr::new(o[12], o[13], o[14], o[15]))
    } else {
        None
    }
}

/// 2001:db8::/32 (RFC 3849) and 3fff::/20 (RFC 9637), documentation ranges.
/// Not covered by a stable `Ipv6Addr` method — `is_documentation` is still
/// feature-gated `ip` (rust-lang/rust#27709).
fn is_documentation_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    (seg[0] == 0x2001 && seg[1] == 0x0db8) || (seg[0] == 0x3fff && (seg[1] & 0xF000) == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(s: &str) -> IpAddr {
        s.parse::<Ipv4Addr>().unwrap().into()
    }
    fn v6(s: &str) -> IpAddr {
        s.parse::<Ipv6Addr>().unwrap().into()
    }

    #[test]
    fn blocks_rfc1918_private_ranges() {
        assert!(!is_globally_routable(v4("10.0.0.1")));
        assert!(!is_globally_routable(v4("172.16.5.5")));
        assert!(!is_globally_routable(v4("192.168.1.1")));
    }

    #[test]
    fn blocks_loopback() {
        assert!(!is_globally_routable(v4("127.0.0.1")));
        assert!(!is_globally_routable(v6("::1")));
    }

    #[test]
    fn blocks_link_local_including_cloud_metadata() {
        // AWS/GCP/Azure/OCI/DigitalOcean metadata endpoint.
        assert!(!is_globally_routable(v4("169.254.169.254")));
    }

    #[test]
    fn blocks_cgnat_shared_address_space_including_alibaba_metadata() {
        // 100.64.0.0/10 (RFC 6598) is neither RFC1918-private nor
        // link-local; a naive check misses it. Alibaba Cloud's metadata
        // endpoint lives here.
        assert!(!is_globally_routable(v4("100.100.100.200")));
        assert!(!is_globally_routable(v4("100.64.0.0")));
        assert!(!is_globally_routable(v4("100.127.255.255")));
        // Just outside the range on both sides must stay unaffected by this
        // specific check (still checked by other rules below, e.g. 100.63.x
        // and 100.128.x are ordinary public space).
        assert!(is_globally_routable(v4("100.63.255.255")));
        assert!(is_globally_routable(v4("100.128.0.0")));
    }

    #[test]
    fn blocks_ietf_protocol_assignment_and_6to4_relay_and_benchmarking_and_reserved() {
        assert!(!is_globally_routable(v4("192.0.0.8"))); // 192.0.0.0/24
        assert!(!is_globally_routable(v4("192.88.99.1"))); // 6to4 relay anycast
        assert!(!is_globally_routable(v4("198.18.0.1"))); // benchmarking
        assert!(!is_globally_routable(v4("198.19.255.255"))); // benchmarking, high end
        assert!(!is_globally_routable(v4("240.0.0.1"))); // reserved
        assert!(!is_globally_routable(v4("255.255.255.255"))); // broadcast
    }

    #[test]
    fn blocks_multicast_and_documentation() {
        assert!(!is_globally_routable(v4("224.0.0.1")));
        assert!(!is_globally_routable(v4("192.0.2.1"))); // TEST-NET-1
        assert!(!is_globally_routable(v4("198.51.100.1"))); // TEST-NET-2
        assert!(!is_globally_routable(v4("203.0.113.1"))); // TEST-NET-3
    }

    #[test]
    fn allows_ordinary_public_v4_addresses() {
        assert!(is_globally_routable(v4("1.1.1.1")));
        assert!(is_globally_routable(v4("8.8.8.8")));
    }

    #[test]
    fn blocks_ipv6_link_local_and_unique_local_including_aws_ipv6_metadata() {
        assert!(!is_globally_routable(v6("fe80::1")));
        // fc00::/7 unique-local; AWS's IPv6 metadata endpoint lives here.
        assert!(!is_globally_routable(v6("fc00::1")));
        assert!(!is_globally_routable(v6("fd00:ec2::254")));
    }

    #[test]
    fn blocks_ipv6_multicast_unspecified_and_documentation() {
        assert!(!is_globally_routable(v6("ff02::1")));
        assert!(!is_globally_routable(v6("::")));
        assert!(!is_globally_routable(v6("2001:db8::1")));
        assert!(!is_globally_routable(v6("3fff::1")));
    }

    #[test]
    fn unwraps_ipv4_mapped_ipv6_and_rechecks_the_embedded_address() {
        // ::ffff:127.0.0.1 — url::Url normalizes the *syntax* to
        // `[::ffff:7f00:1]` but does not flatten it to plain IPv4, so this
        // crate must unwrap it itself before checking.
        assert!(!is_globally_routable(v6("::ffff:127.0.0.1")));
        assert!(!is_globally_routable(v6("::ffff:169.254.169.254")));
        // A mapped *public* address must not be blanket-blocked just for
        // being in mapped form.
        assert!(is_globally_routable(v6("::ffff:8.8.8.8")));
    }

    #[test]
    fn unwraps_nat64_embedded_ipv4_and_rechecks_the_embedded_address() {
        // 64:ff9b::/96 (RFC 6052 well-known prefix): low 32 bits are the
        // embedded IPv4 address. 7f00:1 = 127.0.0.1.
        assert!(!is_globally_routable(v6("64:ff9b::7f00:1")));
        // 0808:0808 = 8.8.8.8 — a NAT64-embedded *public* address must not
        // be blanket-blocked just for using the NAT64 prefix.
        assert!(is_globally_routable(v6("64:ff9b::808:808")));
    }

    #[test]
    fn allows_ordinary_public_v6_addresses() {
        assert!(is_globally_routable(v6("2606:4700:4700::1111")));
    }
}
