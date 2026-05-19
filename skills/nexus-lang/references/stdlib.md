# Nexus Standard Library Reference

Full API: https://nexus-llm-lang.github.io/latest/env/stdlib

The standard library is the `std` package, rooted at `nxlib/stdlib/`. Every module is imported with the form `"std:<name>"`.

## Module Index

| Import path | Source | Common contents | Example |
|-------------|--------|-----------------|---------|
| `"std:stdio"` | [io/stdio.nx](../../../nxlib/stdlib/io/stdio.nx) | `Console` cap (`println`, `eprintln`, `read_line`), `system_handler` | [effects_console_fs.nx](../../../examples/feature/effects_console_fs.nx) |
| `"std:fs"` | [io/fs.nx](../../../nxlib/stdlib/io/fs.nx) | `Fs` cap (`read_to_string`, `write_string`, `exists`, fd-based I/O), `FsThrow` group | [effects_console_fs.nx](../../../examples/feature/effects_console_fs.nx) |
| `"std:network"` | [io/network.nx](../../../nxlib/stdlib/io/network.nx) | `Net` cap, `Request`/`Response`/`Server` types, HTTP client + listener | [network_request.nx](../../../examples/feature/network_request.nx) |
| `"std:proc"` | [io/proc.nx](../../../nxlib/stdlib/io/proc.nx) | `Proc` cap (`exit`, `argv`, `exec`) | [proc_argv.nx](../../../examples/feature/proc_argv.nx) |
| `"std:env"` | [io/env.nx](../../../nxlib/stdlib/io/env.nx) | `Env` cap (`get_env`, `has_env`, `set_env`) | [env_basics.nx](../../../examples/feature/env_basics.nx) |
| `"std:clock"` | [io/clock.nx](../../../nxlib/stdlib/io/clock.nx) | `Clock` cap (`now`, `sleep`) | [clock_basics.nx](../../../examples/feature/clock_basics.nx) |
| `"std:log"` | [io/log.nx](../../../nxlib/stdlib/io/log.nx) | `Logger` cap (`trace`/`debug`/`info`/`warn`/`error`), `console_logger`, `json_logger` | [log_basics.nx](../../../examples/feature/log_basics.nx) |
| `"std:rand"` | [numeric/rand.nx](../../../nxlib/stdlib/numeric/rand.nx) | `Random` cap (`next_i64`, `range`, `next_bool`) | [rand_basics.nx](../../../examples/feature/rand_basics.nx) |
| `"std:math"` | [numeric/math.nx](../../../nxlib/stdlib/numeric/math.nx) | `abs_i64`, `sqrt`, `floor`, `pow`, `i64_to_float`, ... | [math_basics.nx](../../../examples/feature/math_basics.nx) |
| `"std:simd"` | [numeric/simd.nx](../../../nxlib/stdlib/numeric/simd.nx) | 128-bit SIMD intrinsics — `i32x4`/`i64x2` `_add`/`_mul`, autovectorized array ops | — |
| `"std:str"` | [text/str.nx](../../../nxlib/stdlib/text/str.nx) | `length`, `substring`, `index_of`, `starts_with`, `from_i64`, ... | [str_basics.nx](../../../examples/feature/str_basics.nx) |
| `"std:char"` | [text/char.nx](../../../nxlib/stdlib/text/char.nx) | Char/byte helpers (`is_alpha`, `is_digit`, `is_whitespace`) | [char_basics.nx](../../../examples/feature/char_basics.nx) |
| `"std:regexp"` | [text/regexp.nx](../../../nxlib/stdlib/text/regexp.nx) | NFA matcher: `from_string`, `test`, `find`, `gsub` | [regexp_basics.nx](../../../examples/feature/regexp_basics.nx) |
| `"std:bytebuffer"` | [collection/bytebuffer.nx](../../../nxlib/stdlib/collection/bytebuffer.nx) | Linear `%ByteBuffer` — `push_*`, `get_byte`, `to_string` | [bytebuffer_basics.nx](../../../examples/feature/bytebuffer_basics.nx) |
| `"std:hashmap"` | [collection/hashmap.nx](../../../nxlib/stdlib/collection/hashmap.nx) | Linear `%HashMap` (i64→i64) — `put`, `get`, `remove`, `size` | [hashmap_basics.nx](../../../examples/feature/hashmap_basics.nx) |
| `"std:stringmap"` | [collection/stringmap.nx](../../../nxlib/stdlib/collection/stringmap.nx) | Linear `%StringMap` (string→i64) — same ownership as hashmap | [stringmap_basics.nx](../../../examples/feature/stringmap_basics.nx) |
| `"std:list"` | [collection/list.nx](../../../nxlib/stdlib/collection/list.nx) | `length`, `reverse`, `map`, `fold_left`, `filter`, ... | [list_basics.nx](../../../examples/feature/list_basics.nx) |
| `"std:tuple"` | [collection/tuple.nx](../../../nxlib/stdlib/collection/tuple.nx) | `Pair<A, B>(left, right)`, `fst`, `snd` | [tuple_pair.nx](../../../examples/feature/tuple_pair.nx) |
| `"std:option"` | [core/option.nx](../../../nxlib/stdlib/core/option.nx) | `Option<T>` = `Some(val) \| None`; `map`, `and_then`, `unwrap_or` | [option_basics.nx](../../../examples/feature/option_basics.nx) |
| `"std:result"` | [core/result.nx](../../../nxlib/stdlib/core/result.nx) | `Result<T, E>` = `Ok(val) \| Err(err)`; `map`, `and_then`, `is_err` | [result_basics.nx](../../../examples/feature/result_basics.nx) |
| `"std:exn"` | [core/exn.nx](../../../nxlib/stdlib/core/exn.nx) | Exception helpers, `todo()`, `backtrace()` | [exn_todo.nx](../../../examples/feature/exn_todo.nx) |
| `"std:core"` | [core/core.nx](../../../nxlib/stdlib/core/core.nx) | `id` (polymorphic identity) | — |
| `"std:lazy"` | [concurrency/lazy.nx](../../../nxlib/stdlib/concurrency/lazy.nx) | `force_all` (parallel WASI threads), `race`/`cancel`/`detach` | [lazy_parallel.nx](../../../examples/feature/lazy_parallel.nx) |
| `"std:lazy_host"` | [concurrency/lazy_host.nx](../../../nxlib/stdlib/concurrency/lazy_host.nx) | `host_spawn(@T) -> %Task<T>`, `host_join(%Task<T>) -> T` | [lazy_parallel.nx](../../../examples/feature/lazy_parallel.nx) |
| `"std:json"` | [encoding/json.nx](../../../nxlib/stdlib/encoding/json.nx) | `parse`, `serialize`, `get_field`, `JsonValue` algebra | [json_basics.nx](../../../examples/feature/json_basics.nx) |
| `"std:jsonrpc"` | [encoding/jsonrpc.nx](../../../nxlib/stdlib/encoding/jsonrpc.nx) | JSON-RPC 2.0 framing — `frame_message`/`unframe_one`, `read_message`/`write_message` | — |
| `"std:pbt"` | [meta/pbt.nx](../../../nxlib/stdlib/meta/pbt.nx) | `Gen<T>` generators, `forall` runner, `small_int`/`bool_gen`/`string_gen` | [pbt_basics.nx](../../../examples/feature/pbt_basics.nx) |
| `"std:argparse"` | [meta/argparse.nx](../../../nxlib/stdlib/meta/argparse.nx) | CLI argument parser — spec builder, `parse`, `render_help` | — |

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
