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
fn blocks_this_network_range_which_reaches_localhost_on_linux() {
    // 0.0.0.0/8: a connect to any of these reaches 127.0.0.1-bound
    // services on Linux, so the whole /8 must be blocked, not just
    // the single unspecified address.
    assert!(!is_globally_routable(v4("0.0.0.0")));
    assert!(!is_globally_routable(v4("0.1.2.3")));
    assert!(!is_globally_routable(v4("0.255.255.255")));
    // Same range reached through the IPv4-mapped and NAT64 unwrap paths.
    assert!(!is_globally_routable(v6("::ffff:0.0.0.0")));
    assert!(!is_globally_routable(v6("64:ff9b::")));
    // Just outside the range.
    assert!(is_globally_routable(v4("1.0.0.0")));
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
