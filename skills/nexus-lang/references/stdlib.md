# Nexus Standard Library Reference

Full API: https://nexus-llm-lang.github.io/latest/env/stdlib

The standard library is the `std` package, rooted at `nxlib/stdlib/`. Every module is imported with the form `"std:<name>"`.

## Module Index

| Import path | Sources | Common contents |
|-------------|---------|-----------------|
| `"std:stdio"` | FFI | `Console` cap (`println`, `eprintln`, `read_line`), `system_handler` |
| `"std:fs"` | FFI | `Fs` cap (`read_to_string`, `write_string`, `exists`, fd-based I/O) |
| `"std:network"` | FFI | `Net` cap, `Request`/`Response`/`Server` types, HTTP client + listener |
| `"std:proc"` | FFI | `Proc` cap (`exit`, `argv`, `exec`) |
| `"std:env"` | FFI | `Env` cap (`get_env`, `has_env`, `set_env`) |
| `"std:clock"` | FFI | `Clock` cap (`now`, `sleep`) |
| `"std:rand"` | FFI | `Random` cap (`random_i64`, `random_range`) |
| `"std:math"` | FFI | `abs_i64`, `sqrt`, `floor`, `pow`, `i64_to_float`, ... |
| `"std:str"` | FFI | `length`, `substring`, `index_of`, `starts_with`, `from_i64`, ... |
| `"std:char"` | FFI | Char/byte helpers (shares the `string` host module) |
| `"std:bytebuffer"` | FFI | `ByteBuffer` opaque type, push/get/write helpers |
| `"std:hashmap"` | FFI | `HashMap` (i64→i64) opaque type |
| `"std:set"` | FFI | `HashSet` opaque type |
| `"std:stringmap"` | FFI | `StringMap` (string→i64) opaque type |
| `"std:array"` | FFI | Linear array helpers |
| `"std:list"` | pure | `length`, `reverse`, `map`, `fold_left`, `filter`, ... |
| `"std:option"` | pure | `Option<T>` = `Some(val) \| None` |
| `"std:result"` | pure | `Result<T, E>` = `Ok(val) \| Err(err)` |
| `"std:tuple"` | pure | `Pair<A, B>(left, right)`, `fst`, `snd` |
| `"std:exn"` | pure | Exception helpers, `backtrace()` |
| `"std:lazy"` | pure | `Lazy<T>` combinators — `force_all` (real parallel via host_spawn/host_join), `race`/`cancel`/`detach` (sequential pending first-of/drop primitives) |
| `"std:lazy_host"` | FFI / WASI threads | `host_spawn(@T) -> %Task<T>`, `host_join(%Task<T>) -> T`, `host_force` — lower-level parallel-lazy primitives backing `std:lazy.force_all` |
| `"std:core"` | pure | `id` (polymorphic identity) |
| `"std:json"` | pure | `parse`, `serialize`, `get_field`, `JsonValue` algebra (`JsonInt`, `JsonString`, `JsonArray`, `JsonObject`, ...) |
| `"std:simd"` | FFI | 128-bit SIMD intrinsics — lane-wise `i32x4`/`i64x2` `_add`/`_mul`, autovectorized `*_add_array`/`*_mul_array`/`*_sub_array`/`*_div_array` over `f32x4`/`f64x2`/`i32x4`/`i64x2`, plus `scalar_*_array` non-SIMD fallbacks |

Modules occasionally referenced by name elsewhere — `std:chan`, `std:sched`, `std:string`, `std:_nx` — are intentionally omitted: they're either duplicates of an existing module under a different label or speculative names with no `nxlib/stdlib/<name>.nx` source (see nexus-ds7e / nexus-xqzl).

### Runtime intrinsics (compiler-internal)

These modules live under `nxlib/stdlib/runtime/` and bind low-level wasm
operations or codegen-recognised externs.  User code should not import
these directly unless writing compiler internals or runtime tests.

| Import path | Purpose |
|-------------|---------|
| `"std:runtime/math"` | `__nx_f64_*` math intrinsics (`sqrt`, `floor`, ...) |
| `"std:runtime/mem"` | Bounds-checked load/store, `MemoryOutOfBounds` |
| `"std:runtime/arena"` | `heap_mark`, `heap_reset` (compiler arena reset) |

### Source-only directories

- `nxlib/stdlib/test/` — test framework modules (`assert`).  Imported as
  `std:test/<name>`.

## Capability Permissions

Stdlib caps require permissions on `main`:

```nexus
let main = fn () -> unit require { PermConsole, PermFs, PermNet } do
  inject stdio.system_handler do
    inject fs_mod.system_handler do
      // ...
    end
  end
end
```

Permissions: `PermConsole`, `PermFs`, `PermNet`, `PermProc`, `PermClock`, `PermRandom`. Pure modules (list, option, result, tuple, exn, lazy) need none.

## See Also

- `./patterns.md` — Idiomatic Nexus patterns
- https://nexus-llm-lang.github.io/latest/env/stdlib — Full API reference
