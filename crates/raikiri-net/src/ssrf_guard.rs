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
/// Consumer-supplied resource URL: not "this network" (0.0.0.0/8), private,
/// loopback, link-local, CGNAT,
/// documentation, benchmarking, reserved, multicast, broadcast, in the
/// local-use NAT64 prefix `64:ff9b:1::/48`, an IPv4-mapped /
/// well-known-prefix NAT64 address whose embedded IPv4 address itself fails
/// this same check, or — for IPv6 — outside IANA's global unicast
/// allocation (`2000::/3`) at all (which by itself excludes every
/// IPv4-compatible/-translated and deprecated site-local form, named or
/// not), or within the deprecated 6to4 (`2002::/16`) or Teredo
/// (`2001::/32`) transition prefixes.
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
    !(is_this_network(ip)
        || ip.is_private()
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

/// 0.0.0.0/8 (RFC 1122 §3.2.1.3 / RFC 6890, "this network"). Not a valid
/// destination, but on Linux a connect to any address in this range reaches
/// sockets listening on the local host, so it is a loopback bypass unless
/// blocked. `Ipv4Addr::is_unspecified` only matches the single address
/// 0.0.0.0, not the whole /8, so the range is checked here explicitly.
fn is_this_network(ip: Ipv4Addr) -> bool {
    ip.octets()[0] == 0
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
        || is_nat64_local_use(ip)
        || is_6to4(ip)
        || is_teredo(ip)
    {
        return false;
    }
    if let Some(mapped) = ip.to_ipv4_mapped() {
        return is_v4_globally_routable(mapped);
    }
    if let Some(embedded) = nat64_embedded_v4(ip) {
        return is_v4_globally_routable(embedded);
    }
    is_global_unicast(ip)
}

/// `2000::/3`, IANA's IPv6 Global Unicast allocation — the *only* range
/// this floor accepts an otherwise-unhandled address from. Requiring this
/// affirmatively, rather than only excluding each known non-global form by
/// name, closes every IPv4-compatible/-translated and deprecated
/// site-local form in one check: `::7f00:1` (deprecated IPv4-compatible,
/// RFC 4291), `::ffff:0:7f00:1` (IPv4-translated, RFC 2765), `100::1`
/// (discard-only, RFC 6666), and `fec0::1` (deprecated site-local, RFC
/// 3879) all fall outside `2000::/3` and are rejected here without each
/// needing its own named check — including any such form nobody has
/// enumerated yet.
fn is_global_unicast(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xE000) == 0x2000
}

/// `2002::/16` (RFC 3056, 6to4). Deprecated and disabled by default in
/// every major OS for years; blocked outright rather than unwrapping its
/// embedded IPv4 address, since nothing should legitimately name a 6to4
/// address as a resource URL host.
fn is_6to4(ip: Ipv6Addr) -> bool {
    ip.segments()[0] == 0x2002
}

/// `2001::/32` (RFC 4380, Teredo). Same rationale as 6to4: a legacy NAT
/// traversal mechanism with no legitimate reason to appear as a
/// resource-fetch destination, blocked outright rather than unwrapping its
/// XOR-obfuscated embedded IPv4 address.
fn is_teredo(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    seg[0] == 0x2001 && seg[1] == 0x0000
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

/// 64:ff9b:1::/48 (RFC 8215, local-use IPv4/IPv6 translation prefix).
/// Unlike the well-known prefix above, this one is reserved for translators
/// inside a single operator's network and never names a global
/// destination: a local NAT64 behind it can map to RFC 1918 space (for
/// example `64:ff9b:1::a00:1` reaching 10.0.0.1). The whole /48 is
/// therefore blocked outright rather than unwrapped and re-checked —
/// RFC 8215 does not even fix where inside the prefix the IPv4 address is
/// embedded, so there is no single embedded address to re-check.
fn is_nat64_local_use(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    seg[0] == 0x0064 && seg[1] == 0xff9b && seg[2] == 0x0001
}

/// 2001:db8::/32 (RFC 3849) and 3fff::/20 (RFC 9637), documentation ranges.
/// Not covered by a stable `Ipv6Addr` method — `is_documentation` is still
/// feature-gated `ip` (rust-lang/rust#27709).
fn is_documentation_v6(ip: Ipv6Addr) -> bool {
    let seg = ip.segments();
    (seg[0] == 0x2001 && seg[1] == 0x0db8) || (seg[0] == 0x3fff && (seg[1] & 0xF000) == 0)
}

#[cfg(test)]
mod tests;
