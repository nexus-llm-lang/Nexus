# Examples index

This corpus is the entry point for both **humans learning Nexus** and
**LLMs training/referencing** the language. It is paired with a
machine-readable counterpart at [`index.json`](./index.json) (validated
against [`index.schema.json`](./index.schema.json)).

## Layout

| Directory | Purpose | Count |
| --- | --- | --- |
| `examples/` (root) | Historical end-to-end samples (hello, fib, bench) | 7 |
| `examples/feature/` | One minimal example per language feature / stdlib module | 38 |
| `examples/negative/` | Intentionally broken snippets; runner asserts the diagnostic (compile-time *or* runtime throw) | 12 |

Total: **57 examples**. See [`negative/run.sh`](./negative/run.sh) for
the negative-corpus driver.

## How to use

* **Look up by topic** — grep `index.json`'s `topics`, `stdlib`, `require`,
  or `throws` arrays. Each entry carries `difficulty: beginner | intermediate | advanced`.
* **Find a minimal example for a feature** — every entry under
  `examples/feature/` is the *minimal* compilable demonstration of one
  surface (one cap, one stdlib module, one pattern variety, etc.).
* **Show me what NOT to do** — `examples/negative/` is the
  intentionally-broken corpus. Each file declares the diagnostic it
  expects to provoke via header comments (`// expect-fail: E2007`).

## Positive examples — by topic

### Language features

| File | Demonstrates |
| --- | --- |
| [`feature/handler_basic.nx`](./feature/handler_basic.nx) | Define a `cap`, install a `handler`, `inject` it |
| [`feature/effects_console_fs.nx`](./feature/effects_console_fs.nx) | Two caps in one function (`require { Console, Fs }`) |
| [`feature/linear_consume.nx`](./feature/linear_consume.nx) | `%`-prefixed linear bindings, consumed exactly once |
| [`feature/linear_borrow.nx`](./feature/linear_borrow.nx) | `&` borrow — read without consuming the linear handle |
| [`feature/match_basics.nx`](./feature/match_basics.nx) | The four core `match` shapes (literal / record / list / ctor) |
| [`feature/match_guards_or.nx`](./feature/match_guards_or.nx) | Arm guards (`when ...`) and or-patterns |
| [`feature/match_nested.nx`](./feature/match_nested.nx) | Sum-of-product nested destructure |
| [`feature/generics_poly_fn.nx`](./feature/generics_poly_fn.nx) | `fn <T>(...)` parametric functions |
| [`feature/record_destructure.nx`](./feature/record_destructure.nx) | Record-field destructure shorthand |
| [`feature/let_destructure.nx`](./feature/let_destructure.nx) | Irrefutable `let Ctor(...) = expr` |
| [`feature/mutable_ref.nx`](./feature/mutable_ref.nx) | Mutable reference cells |
| [`feature/lazy_force.nx`](./feature/lazy_force.nx) | `@t` single-thunk lazy bindings |
| [`feature/lazy_parallel.nx`](./feature/lazy_parallel.nx) | `lazy.force_all` for parallel WASI-thread dispatch |
| [`feature/try_catch.nx`](./feature/try_catch.nx) | `try / catch | Exn -> ...` and `throws { ... }` rows |
| [`feature/exception_group.nx`](./feature/exception_group.nx) | `exception group G = A | B` closed sums |
| [`feature/exn_todo.nx`](./feature/exn_todo.nx) | `exn.todo()` typed placeholder |
| [`feature/import_forms.nx`](./feature/import_forms.nx) | Named / wildcard / combined `import` shapes |
| [`feature/if_expression.nx`](./feature/if_expression.nx) | `if` is a value-producing expression |
| [`feature/recursive_loop.nx`](./feature/recursive_loop.nx) | Tail-recursive accumulator (no `for` / `while`) |

### stdlib modules

| Module | Example |
| --- | --- |
| `std:stdio` | [`feature/handler_basic.nx`](./feature/handler_basic.nx) (Console + inject) |
| `std:fs` | [`feature/effects_console_fs.nx`](./feature/effects_console_fs.nx) (write/read/remove) |
| `std:list` | [`feature/list_basics.nx`](./feature/list_basics.nx) |
| `std:str` | [`feature/str_basics.nx`](./feature/str_basics.nx) |
| `std:option` | [`feature/option_basics.nx`](./feature/option_basics.nx) |
| `std:result` | [`feature/result_basics.nx`](./feature/result_basics.nx) |
| `std:math` | [`feature/math_basics.nx`](./feature/math_basics.nx) |
| `std:hashmap` | [`feature/hashmap_basics.nx`](./feature/hashmap_basics.nx) (linear) |
| `std:stringmap` | [`feature/stringmap_basics.nx`](./feature/stringmap_basics.nx) (linear) |
| `std:char` | [`feature/char_basics.nx`](./feature/char_basics.nx) |
| `std:tuple` | [`feature/tuple_pair.nx`](./feature/tuple_pair.nx) |
| `std:rand` | [`feature/rand_basics.nx`](./feature/rand_basics.nx) (cap) |
| `std:clock` | [`feature/clock_basics.nx`](./feature/clock_basics.nx) (cap) |
| `std:env` | [`feature/env_basics.nx`](./feature/env_basics.nx) (cap) |
| `std:proc` | [`feature/proc_argv.nx`](./feature/proc_argv.nx) (cap) |
| `std:network` | [`feature/network_request.nx`](./feature/network_request.nx) (cap; see also [`network_access.nx`](./network_access.nx) for a live call) |
| `std:log` | [`feature/log_basics.nx`](./feature/log_basics.nx) (cap) |
| `std:json` | [`feature/json_basics.nx`](./feature/json_basics.nx) |
| `std:bytebuffer` | [`feature/bytebuffer_basics.nx`](./feature/bytebuffer_basics.nx) (linear) |
| `std:regexp` | [`feature/regexp_basics.nx`](./feature/regexp_basics.nx) |
| `std:pbt` | [`feature/pbt_basics.nx`](./feature/pbt_basics.nx) |
| `std:lazy` | [`feature/lazy_parallel.nx`](./feature/lazy_parallel.nx) |
| `std:exn` | [`feature/exn_todo.nx`](./feature/exn_todo.nx) |

## Negative examples

Each fixture under `examples/negative/` declares its expected failure
mode through one of two header-comment flavors.

### Flavor A — compile-time diagnostic

```
// expect-fail: E2007            (required; the diagnostic code)
// expect-msg: linear binding    (optional; repeatable substring assertion)
```

The runner asserts the fixture (a) fails to compile, (b) reports the
declared code in the diagnostic, and (c) contains every declared
substring.

### Flavor B — runtime throw (added in fl9t.1)

```
// expect-runtime-throw: before unwrap   (required; ≥1; substring assertion
//                                         against the combined run output)
// expect-msg: optional extra            (optional; treated as additional
//                                         substring assertions)
```

The runner asserts the fixture (a) compiles successfully, (b) exits
non-zero under `nexus run`, and (c) every declared substring appears in
the combined stdout+stderr. wasmtime does not surface the exception
constructor on stderr, so fixtures typically anchor their assertion on
a `println` line emitted just before the throwing site.

### Run

```sh
./examples/negative/run.sh                 # walks examples/negative/
./examples/negative/run.sh <other-dir>     # walks another directory
```

### Corpus

| File | Mode | Demonstrates |
| --- | --- | --- |
| [`negative/linear_unused.nx`](./negative/linear_unused.nx) | E2007 | `%`-binding falls out of scope unconsumed |
| [`negative/lazy_unforced.nx`](./negative/lazy_unforced.nx) | E2005 | `@`-binding is never forced |
| [`negative/lazy_double_force.nx`](./negative/lazy_double_force.nx) | E2006 | `@`-binding forced more than once |
| [`negative/main_wrong_return.nx`](./negative/main_wrong_return.nx) | E2004 | `main` returns non-`unit` |
| [`negative/non_exhaustive_match.nx`](./negative/non_exhaustive_match.nx) | E2001 | `match` misses a sum-type variant |
| [`negative/type_mismatch.nx`](./negative/type_mismatch.nx) | E2001 | `let` annotation contradicts RHS |
| [`negative/unbound_variable.nx`](./negative/unbound_variable.nx) | E2001 | Reference to an undefined identifier |
| [`negative/missing_argument.nx`](./negative/missing_argument.nx) | E2001 | Call site omits a named parameter |
| [`negative/parse_unterminated_string.nx`](./negative/parse_unterminated_string.nx) | E1001 | Unterminated `"..."` literal |
| [`negative/import_module_not_found.nx`](./negative/import_module_not_found.nx) | E4004 | `import "std:NAME"` with no matching file |
| [`negative/option_unwrap_none.nx`](./negative/option_unwrap_none.nx) | runtime | `std:option.unwrap(None)` raises `RuntimeError` |
| [`negative/result_unhandled_err.nx`](./negative/result_unhandled_err.nx) | runtime | Caller raises on `Err(...)` (idiomatic Result→Exn lift) |

## Schema

The structured index lives at [`index.json`](./index.json). It is validated
against [`index.schema.json`](./index.schema.json) (JSON Schema draft
2020-12). Adding a new example requires appending an entry; the
runner-side check (in CI) ensures every `.nx` under `examples/` is
indexed and that no index entry references a missing file.
