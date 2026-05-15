# Examples

Minimal, runnable demonstrations of Nexus features. Each file is
self-contained; build any one with `nexus build <path>` and execute it
with `nexus run <path>`.

## Root corpus — end-to-end samples

| File | What it shows |
| ---- | --- |
| [`hello.nx`](./hello.nx) | Minimal `main` with `Console.println` |
| [`fib.nx`](./fib.nx) | Recursion + `Console.println(from_i64(...))` |
| [`math.nx`](./math.nx) | Bare module export (no `main`) |
| [`module_test.nx`](./module_test.nx) | `import` from a sibling module |
| [`di_cap.nx`](./di_cap.nx) | Define a `cap`, write a `handler`, `inject` it |
| [`network_access.nx`](./network_access.nx) | Network I/O + `try/catch` |
| [`bench_parallel.nx`](./bench_parallel.nx) | Lazy thunks + `lazy.force_all` parallelism |

## Feature corpus — one file per language surface

Files under [`feature/`](./feature/) are minimal demonstrations of a
single language feature. Each begins with a `// <name>: <one-line
summary>` header and a `// difficulty: <beginner|intermediate|advanced>`
tag.

### Linear types and borrowing

| File | What it shows |
| ---- | --- |
| [`feature/linear_consume.nx`](./feature/linear_consume.nx) | `%`-prefixed linear bindings, consumed exactly once |
| [`feature/linear_thread.nx`](./feature/linear_thread.nx) | Thread a linear value through a recursive accumulator |
| [`feature/linear_borrow.nx`](./feature/linear_borrow.nx) | `&` borrow — read a linear handle without consuming |
| [`feature/mutable_ref.nx`](./feature/mutable_ref.nx) | `let ~x = init; ~x <- v` mutable cells |

### Generics

| File | What it shows |
| ---- | --- |
| [`feature/generics_poly_fn.nx`](./feature/generics_poly_fn.nx) | `fn <T>` and `type Pair<A, B>` parametric definitions |

### Pattern matching

| File | What it shows |
| ---- | --- |
| [`feature/match_basics.nx`](./feature/match_basics.nx) | The four core `match` shapes (literal / record / list / ctor) |
| [`feature/match_guards_or.nx`](./feature/match_guards_or.nx) | Or-patterns (`A | B`) with shared bindings |
| [`feature/match_nested.nx`](./feature/match_nested.nx) | Sum-of-product nested destructure |
| [`feature/record_destructure.nx`](./feature/record_destructure.nx) | Record-field destructure |
| [`feature/let_destructure.nx`](./feature/let_destructure.nx) | Irrefutable `let Ctor(...) = expr` |

### Exceptions

| File | What it shows |
| ---- | --- |
| [`feature/try_catch.nx`](./feature/try_catch.nx) | `raise` + `try / catch | Pattern -> ...` + `throws { ... }` row |
| [`feature/exception_group.nx`](./feature/exception_group.nx) | `exception group G = A | B` closed sums |

### Effects (caps + handlers)

| File | What it shows |
| ---- | --- |
| [`feature/handler_basic.nx`](./feature/handler_basic.nx) | Define a `cap`, install a `handler`, `inject` it |
| [`feature/effects_console_fs.nx`](./feature/effects_console_fs.nx) | Two caps in one function (`require { Console, Fs }`) |

### Lazy evaluation

| File | What it shows |
| ---- | --- |
| [`feature/lazy_force.nx`](./feature/lazy_force.nx) | `@t` thunk + force |
| [`feature/lazy_parallel.nx`](./feature/lazy_parallel.nx) | `lazy.force_all` parallel dispatch |

### FFI

| File | What it shows |
| ---- | --- |
| [`feature/ffi.nx`](./feature/ffi.nx) | `import external "MODULE"` + `external NAME = "wasm_name" : T` |

### Standard library

| File | What it shows |
| ---- | --- |
| [`feature/list_basics.nx`](./feature/list_basics.nx) | `std:list` core (`map`, `length`, `nth`) |

## Adding a new example

1. Pick the smallest piece of code that demonstrates the feature.
2. Lead with a `//` header: `// <name>: <one-line summary>` followed by
   a short prose explanation. Close with `// require: { ... }` and
   `// difficulty: <level>` tags so the file is grep-friendly.
3. Build and run it: `nexus build examples/feature/your_file.nx` and
   `nexus run examples/feature/your_file.nx`.
4. Add a row to the table above.
