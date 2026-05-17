# Examples index

This corpus is the entry point for both **humans learning Nexus** and
**LLMs training/referencing** the language. Everything indexed here is
*positive* (compilable, runnable) material — intentionally-broken
fixtures live under [`tests/negative/`](../tests/negative/) as compiler
regression tests, not as user copy-from material. It is paired with a
machine-readable counterpart at [`index.json`](./index.json) (validated
against [`index.schema.json`](./index.schema.json)).

## Layout

| Directory | Purpose | Count |
| --- | --- | --- |
| `examples/` (root) | Historical end-to-end samples (hello, fib, bench) | 7 |
| `examples/feature/` | One minimal example per language feature / stdlib module | 38 |

Total: **45 examples**.

## How to use

* **Look up by topic** — grep `index.json`'s `topics`, `stdlib`, `require`,
  or `throws` arrays. Each entry carries `difficulty: beginner | intermediate | advanced`.
* **Find a minimal example for a feature** — every entry under
  `examples/feature/` is the *minimal* compilable demonstration of one
  surface (one cap, one stdlib module, one pattern variety, etc.).

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

## Negative fixtures

Intentionally-broken snippets are not example material — they are
compiler regression tests and live under
[`tests/negative/`](../tests/negative/) with their own runner
([`tests/negative/run.sh`](../tests/negative/run.sh)). See the
file-local header comments there for the `expect-fail:` /
`expect-runtime-throw:` directive shapes.

## Schema

The structured index lives at [`index.json`](./index.json). It is validated
against [`index.schema.json`](./index.schema.json) (JSON Schema draft
2020-12). Adding a new example requires appending an entry; the
runner-side check (in CI) ensures every `.nx` under `examples/` is
indexed and that no index entry references a missing file.
