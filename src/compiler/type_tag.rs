//! FNV-1a based tag computation for constructors and records.
//!
//! A single implementation shared by HIR, LIR, and codegen passes.

/// FNV-1a 64-bit offset basis.
const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// FNV-1a 64-bit prime.
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Compute the FNV-1a 64-bit hash of the given byte slices.
pub fn fnv1a_hash(parts: &[&[u8]]) -> u64 {
    let mut hash: u64 = FNV_OFFSET_BASIS;
    for part in parts {
        for &b in *part {
            hash ^= b as u64;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
    }
    hash
}

/// Tag for an enum constructor: FNV-1a of name, then mix in arity.
pub fn constructor_tag(name: &str, arity: usize) -> i64 {
    let mut hash = fnv1a_hash(&[name.as_bytes()]);
    hash ^= arity as u64;
    hash = hash.wrapping_mul(FNV_PRIME);
    hash as i64
}

/// Tag for a record type: FNV-1a of `"rec"` + joined sorted field names, then
/// mix in the field count.
pub fn record_tag(sorted_field_names: &[String]) -> i64 {
    let shape = sorted_field_names.join(",");
    let mut hash = fnv1a_hash(&[b"rec", shape.as_bytes()]);
    hash ^= sorted_field_names.len() as u64;
    hash = hash.wrapping_mul(FNV_PRIME);
    hash as i64
}
