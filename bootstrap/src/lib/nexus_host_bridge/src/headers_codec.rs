//! Bidirectional codec between the bridge canonical headers format
//! (`name:value\n` per line, single LF terminator, no `\r`, no space after the
//! colon, no trailing blank line) and the HTTP/1.1 wire format
//! (`name: value\r\n`, ASCII space, CRLF). The canonical form is the contract
//! between stdlib (`nxlib/stdlib/network.nx::encode_headers`) and the WIT
//! bridge boundary; conversion to/from the HTTP/1.1 wire form happens here
//! and only at the socket I/O boundary in `host_impl.rs`.
//!
//! Empty / blank input lines are skipped on both sides. Lines without a colon
//! are dropped silently (defensive — the host's HTTP parser also tolerates
//! malformed lines rather than panicking on adversarial peer input).

/// Convert canonical `name:value\n` form into HTTP/1.1 wire form
/// (`name: value\r\n`).
pub fn canonical_to_wire(canonical: &str) -> String {
    let mut out = String::with_capacity(canonical.len() + canonical.len() / 8);
    for line in canonical.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            out.push_str(name.trim());
            out.push_str(": ");
            out.push_str(value.trim());
            out.push_str("\r\n");
        }
    }
    out
}

/// Convert HTTP/1.1 wire form (`name: value\r\n`, possibly `name:value\n`)
/// into canonical `name:value\n` form. The first blank line terminates input;
/// callers that have already split body off may pass the headers slice
/// directly without trailing CRLF.
pub fn wire_to_canonical(wire: &str) -> String {
    let mut out = String::with_capacity(wire.len());
    for line in wire.lines() {
        if line.is_empty() {
            // CRLF blank line marks end of headers — caller should have split,
            // but be defensive.
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            out.push_str(name);
            out.push(':');
            out.push_str(value.trim());
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_to_wire_basic() {
        let canonical = "Content-Type:application/json\nX-Foo:bar\n";
        let wire = canonical_to_wire(canonical);
        assert_eq!(wire, "Content-Type: application/json\r\nX-Foo: bar\r\n");
    }

    #[test]
    fn wire_to_canonical_basic() {
        let wire = "Content-Type: application/json\r\nX-Foo: bar\r\n";
        let canonical = wire_to_canonical(wire);
        assert_eq!(canonical, "Content-Type:application/json\nX-Foo:bar\n");
    }

    /// Acceptance-defining test for nexus-upzz.5: encode → wire → decode
    /// round-trip is the identity on canonical input. Locks the symmetry of
    /// the bridge boundary conversion so future edits to either direction
    /// don't drift in isolation.
    #[test]
    fn canonical_wire_canonical_roundtrip_is_identity() {
        let canonical = "Content-Type:application/json\nX-Foo:bar\nAccept:*/*\n";
        let wire = canonical_to_wire(canonical);
        let back = wire_to_canonical(&wire);
        assert_eq!(canonical, back);
    }

    #[test]
    fn wire_canonical_wire_roundtrip_normalizes_then_stable() {
        // Wire form may have varying whitespace; normalise via canonical, then
        // back to wire — second wire pass must equal the canonical-derived form.
        let wire1 = "X-Foo:bar\r\nX-Baz:  qux  \r\n";
        let canonical = wire_to_canonical(wire1);
        let wire2 = canonical_to_wire(&canonical);
        // Whitespace is normalised by the canonical pass.
        assert_eq!(wire2, "X-Foo: bar\r\nX-Baz: qux\r\n");
        // And re-decoding gives the same canonical form.
        assert_eq!(wire_to_canonical(&wire2), canonical);
    }

    #[test]
    fn empty_and_malformed_lines_dropped() {
        let canonical = "\n\nFoo:bar\nbroken-line\n";
        let wire = canonical_to_wire(canonical);
        assert_eq!(wire, "Foo: bar\r\n");
    }

    #[test]
    fn empty_input_is_empty_output_both_directions() {
        assert_eq!(canonical_to_wire(""), "");
        assert_eq!(wire_to_canonical(""), "");
    }
}
