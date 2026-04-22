fn main() {
    // The preview1→preview2 adapter (wasi_snapshot_preview1.reactor.wasm) allocates its
    // State struct via cabi_realloc during component initialisation.  With the default
    // 17-page (≈1 MB) initial memory the allocation falls beyond the linear-memory
    // boundary, causing an out-of-bounds trap at runtime.  Requesting 2 MB (32 pages)
    // up-front gives enough headroom for the adapter shim and for string
    // lifting/lowering buffers used by the canonical ABI.
    //
    // The flag is wasm-ld-specific; on native (host) test builds clang rejects it,
    // so gate on the target family.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("wasm") {
        println!("cargo:rustc-link-arg=--initial-memory=2097152");
    }
}
