# Nexus Standard Library Reference

Full API: https://nexus-llm-lang.github.io/Nexus/latest/env/stdlib

The standard library is the `std` package, rooted at `nxlib/stdlib/`. Every module is imported with the form `"std:<name>"`.

## Module Index

| Import path | Sources | Common contents |
|-------------|---------|-----------------|
| `"std:stdio"` | FFI | `Console` cap (`println`, `eprintln`, `read_line`), `system_handler` |
| `"std:filesystem"` | FFI | `Fs` cap (`read_to_string`, `write_string`, `exists`, fd-based I/O) |
| `"std:network"` | FFI | `Net` cap, `Request`/`Response`/`Server` types, HTTP client + listener |
| `"std:process"` | FFI | `Proc` cap (`exit`, `argv`, `exec`) |
| `"std:environment"` | FFI | `Env` cap (`get_env`, `has_env`, `set_env`) |
| `"std:clock"` | FFI | `Clock` cap (`now`, `sleep`) |
| `"std:random"` | FFI | `Random` cap (`random_i64`, `random_range`) |
| `"std:math"` | FFI | `abs_i64`, `sqrt`, `floor`, `pow`, `i64_to_float`, ... |
| `"std:string_ops"` | FFI | `length`, `substring`, `index_of`, `starts_with`, `from_i64`, ... |
| `"std:char"` | FFI | Char/byte helpers (shares the `string-ops` WIT interface) |
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
| `"std:lazy"` | pure | `Lazy<T>`, host-side lazy force |
| `"std:arena"` | pure | `heap_mark`, `heap_reset` (compiler-internal) |
| `"std:core"` | FFI | Compiler intrinsics (`array-length`) |

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

## WIT Interface Naming

For each `std:<module>`, the WIT module name (used by codegen and component composition) is `nexus:std/<module>`, with underscores converted to hyphens:

| Import path | WIT interface |
|-------------|---------------|
| `std:stdio` | `nexus:std/stdio` |
| `std:string_ops` | `nexus:std/string-ops` |
| `std:bytebuffer` | `nexus:std/bytebuffer` |

Use this form in `import external "std:<iface>"` declarations inside FFI-binding files (the WIT interface name uses hyphens regardless of the underscore-friendly import path).

## See Also

- `./patterns.md` — Idiomatic Nexus patterns
- https://nexus-llm-lang.github.io/Nexus/latest/env/stdlib — Full API reference
