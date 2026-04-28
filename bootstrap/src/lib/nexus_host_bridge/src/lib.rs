pub mod headers_codec;
pub mod url_guard;

// The WIT bindings, server state, and Guest impl are only meaningful on the
// wasm target; host builds (cargo test) compile only url_guard and
// headers_codec so the pure logic (SSRF blocking, canonical/wire headers
// conversion) can be unit-tested without WASI symbols.
#[cfg(target_family = "wasm")]
mod host_impl;
