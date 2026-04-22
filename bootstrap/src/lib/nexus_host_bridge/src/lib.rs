pub mod url_guard;

// The WIT bindings, server state, and Guest impl are only meaningful on the
// wasm target; host builds (cargo test) compile only url_guard so the pure
// SSRF-blocking logic can be unit-tested without WASI symbols.
#[cfg(target_family = "wasm")]
mod host_impl;
