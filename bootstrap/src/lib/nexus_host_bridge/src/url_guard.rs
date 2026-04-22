//! Pure URL/host validation and HTTP status-reason helpers.
//!
//! Kept free of WASI bindings so the host target (i.e. `cargo test`) can
//! compile and exercise them. The WASM client code in `lib.rs` calls these
//! before opening any outgoing connection.

use http::StatusCode;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// Decide whether an outgoing HTTP destination should be denied.
///
/// The check is syntactic: it inspects the host portion of the URL as
/// declared by the caller. IP literals in blocked ranges and well-known
/// metadata/loopback hostnames are rejected before any DNS resolution or
/// TCP connect happens.
///
/// Limitation: this does not defend against DNS rebinding, where an
/// attacker-controlled domain initially resolves to a public IP (passing
/// this gate) and later resolves to a private IP when wasi-sockets connects.
/// Atomic resolve-then-validate requires a socket API the WASI 0.2
/// outgoing-handler does not expose.
///
/// Returns `Some(reason)` if blocked, `None` if allowed.
pub fn is_blocked_host(host: &str) -> Option<&'static str> {
    // http::Uri::host() normally strips IPv6 brackets; trim defensively.
    let host = host.trim_start_matches('[').trim_end_matches(']');

    if let Ok(ip) = host.parse::<IpAddr>() {
        return match ip {
            IpAddr::V4(v4) => blocked_ipv4_reason(v4),
            IpAddr::V6(v6) => blocked_ipv6_reason(v6),
        };
    }

    let lower = host.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "localhost"
            | "ip6-localhost"
            | "ip6-loopback"
            | "metadata.google.internal"
            | "metadata.goog"
    ) {
        return Some("blocked hostname");
    }
    if lower.ends_with(".localhost") || lower.ends_with(".local") {
        return Some("loopback/mDNS hostname");
    }
    None
}

fn blocked_ipv4_reason(ip: Ipv4Addr) -> Option<&'static str> {
    if ip.is_unspecified() {
        Some("unspecified IPv4 (0.0.0.0/8)")
    } else if ip.is_loopback() {
        Some("loopback IPv4 (127.0.0.0/8)")
    } else if ip.is_link_local() {
        // 169.254.0.0/16 — covers AWS/GCP IMDS 169.254.169.254
        Some("link-local IPv4 (169.254.0.0/16, includes cloud metadata)")
    } else if ip.is_private() {
        Some("private IPv4 (10/8, 172.16/12, 192.168/16)")
    } else if ip.is_multicast() {
        Some("multicast IPv4 (224.0.0.0/4)")
    } else if ip.is_broadcast() {
        Some("broadcast IPv4 (255.255.255.255)")
    } else if ip.is_documentation() {
        Some("documentation IPv4 (TEST-NET)")
    } else {
        None
    }
}

fn blocked_ipv6_reason(ip: Ipv6Addr) -> Option<&'static str> {
    if ip.is_unspecified() {
        return Some("unspecified IPv6 (::)");
    }
    if ip.is_loopback() {
        return Some("loopback IPv6 (::1)");
    }
    if ip.is_multicast() {
        return Some("multicast IPv6 (ff00::/8)");
    }

    // IPv4-mapped (::ffff:a.b.c.d) and v4-compat (::a.b.c.d, deprecated):
    // reject if the embedded IPv4 is blocked. is_loopback above catches ::1
    // before this runs, so v4-compat's lenient match doesn't false-positive.
    if let Some(v4) = ip.to_ipv4() {
        if let Some(reason) = blocked_ipv4_reason(v4) {
            return Some(reason);
        }
    }

    let segments = ip.segments();
    if segments[0] & 0xfe00 == 0xfc00 {
        return Some("unique-local IPv6 (fc00::/7)");
    }
    if segments[0] & 0xffc0 == 0xfe80 {
        return Some("link-local IPv6 (fe80::/10)");
    }
    None
}

/// RFC 9110 reason phrase for a status code.
///
/// Unknown or non-standard codes return an empty string so the response
/// line reads `HTTP/1.1 <code> ` instead of silently mislabelling them
/// "OK" (e.g. `HTTP/1.1 418 OK` for a teapot response).
pub fn status_reason(code: i64) -> &'static str {
    u16::try_from(code)
        .ok()
        .and_then(|c| StatusCode::from_u16(c).ok())
        .and_then(|sc| sc.canonical_reason())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_aws_gcp_metadata_ip() {
        assert!(is_blocked_host("169.254.169.254").is_some());
    }

    #[test]
    fn blocks_loopback_ipv4() {
        assert!(is_blocked_host("127.0.0.1").is_some());
        assert!(is_blocked_host("127.255.255.254").is_some());
    }

    #[test]
    fn blocks_private_ipv4_ranges() {
        assert!(is_blocked_host("10.0.0.1").is_some());
        assert!(is_blocked_host("172.16.0.1").is_some());
        assert!(is_blocked_host("172.31.255.255").is_some());
        assert!(is_blocked_host("192.168.1.1").is_some());
    }

    #[test]
    fn allows_public_ipv4() {
        assert_eq!(is_blocked_host("8.8.8.8"), None);
        assert_eq!(is_blocked_host("1.1.1.1"), None);
    }

    #[test]
    fn blocks_unspecified_and_broadcast() {
        assert!(is_blocked_host("0.0.0.0").is_some());
        assert!(is_blocked_host("255.255.255.255").is_some());
    }

    #[test]
    fn blocks_link_local_ipv4_boundary() {
        assert!(is_blocked_host("169.254.0.0").is_some());
        assert!(is_blocked_host("169.254.255.255").is_some());
    }

    #[test]
    fn blocks_ipv6_loopback_and_unspecified() {
        assert!(is_blocked_host("::1").is_some());
        assert!(is_blocked_host("::").is_some());
    }

    #[test]
    fn blocks_ipv6_unique_local() {
        assert!(is_blocked_host("fc00::1").is_some());
        assert!(is_blocked_host("fd12:3456::1").is_some());
    }

    #[test]
    fn blocks_ipv6_link_local() {
        assert!(is_blocked_host("fe80::1").is_some());
    }

    #[test]
    fn blocks_ipv4_mapped_private() {
        assert!(is_blocked_host("::ffff:169.254.169.254").is_some());
        assert!(is_blocked_host("::ffff:127.0.0.1").is_some());
        assert!(is_blocked_host("::ffff:10.0.0.1").is_some());
    }

    #[test]
    fn allows_ipv4_mapped_public() {
        assert_eq!(is_blocked_host("::ffff:8.8.8.8"), None);
    }

    #[test]
    fn blocks_localhost_hostnames() {
        assert!(is_blocked_host("localhost").is_some());
        assert!(is_blocked_host("LOCALHOST").is_some());
        assert!(is_blocked_host("my.localhost").is_some());
    }

    #[test]
    fn blocks_cloud_metadata_hostnames() {
        assert!(is_blocked_host("metadata.google.internal").is_some());
        assert!(is_blocked_host("Metadata.Google.Internal").is_some());
        assert!(is_blocked_host("metadata.goog").is_some());
    }

    #[test]
    fn blocks_mdns_local_suffix() {
        assert!(is_blocked_host("printer.local").is_some());
    }

    #[test]
    fn allows_normal_public_hostnames() {
        assert_eq!(is_blocked_host("example.com"), None);
        assert_eq!(is_blocked_host("api.github.com"), None);
    }

    #[test]
    fn tolerates_ipv6_brackets() {
        assert!(is_blocked_host("[::1]").is_some());
    }

    #[test]
    fn status_reason_returns_canonical_phrase_for_known_codes() {
        assert_eq!(status_reason(200), "OK");
        assert_eq!(status_reason(201), "Created");
        assert_eq!(status_reason(204), "No Content");
        assert_eq!(status_reason(404), "Not Found");
        assert_eq!(status_reason(418), "I'm a teapot");
        assert_eq!(status_reason(500), "Internal Server Error");
    }

    #[test]
    fn status_reason_empty_for_unknown_codes() {
        // Prior behaviour: fell back to "OK". New behaviour: empty string.
        assert_eq!(status_reason(299), "");
        assert_eq!(status_reason(999), "");
    }

    #[test]
    fn status_reason_empty_for_out_of_range() {
        assert_eq!(status_reason(-1), "");
        assert_eq!(status_reason(70_000), "");
    }
}
