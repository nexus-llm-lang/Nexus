fn main() {
    // The preview1→preview2 adapter (wasi_snapshot_preview1.reactor.wasm) allocates its
    // State struct via cabi_realloc during component initialisation.  With the default
    // 17-page (≈1 MB) initial memory the allocation falls beyond the linear-memory
    // boundary, causing an out-of-bounds trap at runtime.  Requesting 2 MB (32 pages)
    // up-front gives enough headroom for the adapter shim and for string
    // lifting/lowering buffers used by the canonical ABI.
    println!("cargo:rustc-link-arg=--initial-memory=2097152");
}
