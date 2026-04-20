// Force-link all sub-crate symbols so their #[no_mangle] exports appear in the
// final cdylib wasm module.  Each `extern crate` + `use` ensures the linker
// pulls in the rlib and preserves its exported functions.

extern crate nexus_clock_wasm;
extern crate nexus_collection_wasm;
extern crate nexus_core_wasm;
extern crate nexus_fs_wasm;
extern crate nexus_math_wasm;
extern crate nexus_proc_wasm;
extern crate nexus_random_wasm;
extern crate nexus_stdio_wasm;
extern crate nexus_string_wasm;
// nexus_net_wasm MUST be last — its nexus:cli imports must come after all WASI
// imports so the wasm_merge identity remap works without dep code rewriting.
extern crate nexus_net_wasm;

// Reference at least one item from each crate to prevent the linker from
// discarding the crate entirely.  We use a single never-called function whose
// address is taken but never invoked.
#[used]
static _FORCE_LINK: [fn(); 0] = [];
