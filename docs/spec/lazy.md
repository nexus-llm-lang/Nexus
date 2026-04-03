---
layout: default
title: Lazy Evaluation, Concurrency, and Parallelism
---

# Lazy Evaluation, Concurrency, and Parallelism (`@`)

The `@` sigil is Nexus's unified primitive for lazy evaluation, concurrency, and parallelism. It replaces the former `conc` block syntax with a design rooted in **one-shot delimited continuations** and **linear types**.

A lazy binding `let @x = expr` suspends `expr` as an unevaluated thunk. Forcing with `@x` evaluates it. Independent thunks within a force expression are evaluated in parallel via DAG scheduling — data dependencies determine execution order, not left-to-right evaluation.

## Design Rationale

- **No async/await keywords**: `@` integrates into the existing sigil system (`%` linear, `&` borrow, `@` lazy) rather than adding new syntax
- **Lazy, not eager**: Unlike JS Promises (eager evaluation, implicit error swallowing), `@` thunks are unevaluated until forced
- **One-shot continuation**: Internally based on one-shot delimited continuations (cf. OCaml 5). Linear types guarantee single-use — no copying, no multi-shot
- **Exception propagation**: No separate rejection channel. `raise` inside a thunk propagates via standard `try/catch` at the force site

## Sigil Table

| Sigil | Meaning | Evaluation | Linearity |
|---|---|---|---|
| (none) | Value | Eager | Non-linear |
| `%` | Linear value | Eager | Linear |
| `@` | Suspended computation | Lazy | Linear |
| `&` | Borrow | — | Borrow |

## Syntax

```nexus
let @x = expensive_computation()  // creates thunk, NOT evaluated
let result = @x                    // forces thunk, evaluates now
```

`@T` is the type of a suspended computation producing `T`:

```nexus
let delayed: @string = @("hello" ++ " world")
```

## DAG Parallel Evaluation

`@expr` builds a dependency DAG from the expression's AST and evaluates independent nodes in parallel:

```nexus
@(f(a: x, b: y))
//     f(a:x, b:y)    ← level 2: apply (after args resolve)
//       / | \
//      f   x   y      ← level 1: parallel evaluation
```

Nested calls create deeper DAGs:

```nexus
@(f(a: g(b: x)))
//     f(a: y)         ← level 3: apply
//       |
//     g(b: x) → y    ← level 2: apply
//    / |  \
//   f   g   x         ← level 1: parallel evaluation
```

Record force `@{ a: x, b: y }` is a special case — a height-2 tree where fields are leaves evaluated in parallel. This replaces `conc` blocks:

```nexus
// Before (conc block — removed):
conc do
  task t1 do arr[0] <- compute1() end
  task t2 do arr[1] <- compute2() end
end

// After (@ sigil):
let @p1 = do arr[0] <- compute1() end
let @p2 = do arr[1] <- compute2() end
let _ = @{ r1: p1, r2: p2 }
```

## Linearity

`@T` is inherently linear — a one-shot continuation must be consumed exactly once. Three consumption operations:

| Operation | Executes? | Waits? | Use case |
|---|---|---|---|
| `@x` (force) | Yes | Yes | Normal evaluation |
| `detach(a: x)` | Yes | No | Fire-and-forget |
| `cancel(a: x)` | No | — | Discard unneeded computation |

Unconsumed `@T` is a compile error. Copying is forbidden (not multi-shot).

`@`'s linearity is orthogonal to the result's linearity:

```nexus
let @a = compute_string()   // @string — result is copyable
let @b = acquire_server()   // @%Server — result is linear
let s = @a                  // s: string (non-linear binding)
let %srv = @b               // %srv: %Server (linear binding)
```

Capturing `@x` in a closure makes the closure itself linear:

```nexus
let @x = heavy_compute()
let f = fn () -> i64 do @x end   // f captures @x → f is linear
f()   // OK: consumes f
f()   // ERROR: f already consumed
```

## Deadlock Freedom

Linear types structurally prevent deadlock:

1. **No forward references**: `let` bindings are sequential — a thunk cannot reference a later-defined `@` value, so simple cycles are syntactically impossible
2. **No sharing**: `@T` is non-copyable — two thunks cannot depend on the same `@` value, so circular dependencies cannot be constructed
3. **Acyclic DAG**: The parallel evaluation DAG is derived from the AST, which is structurally a tree (acyclic)

## Data Race Freedom

The existing borrow checker prevents data races during parallel force:

```nexus
let %arr = [| 0, 0 |]
let @a = do let lock = &%arr; lock[0] <- 1 end
let @b = do let lock = &%arr; lock[1] <- 2 end   // ERROR: %arr already borrowed
```

Shared mutable state across parallel thunks requires explicit concurrency primitives (channels, atomics).

## Exception Semantics

Exceptions raised inside a thunk propagate at the force site via standard `try/catch`:

```nexus
let @result = do
  raise NotFound(path: "/missing")
end

try
  let v = @result   // force → exception propagates here
catch
  case NotFound(path: p) -> handle(p: p)
end
```

During parallel force (`@{ a: x, b: y }`), if one thunk raises:
- The exception propagates at the join point
- The other thunk's continuation is dropped (= cancelled)
- If already running, it completes but the result is discarded
- No resource leak — linear types guarantee cleanup

## Standard Library (`stdlib/lazy.nx`)

Only the `@` sigil is built into the language. Combinators are provided as stdlib functions backed by runtime host functions:

| Function | Signature | Description |
|---|---|---|
| `race` | `(a: @T, b: @T) -> T` | Returns first to complete; loser is cancelled |
| `cancel` | `(a: @T) -> unit` | Discard without evaluating |
| `detach` | `(a: @T) -> unit` | Start evaluation, don't wait for result |
| `force_all` | `(tasks: [@T]) -> [T]` | Parallel force of a list |

## Current Implementation Status

The `@` sigil is fully implemented:
- **Thunk creation**: `let @x = expr` — desugars to zero-argument closures
- **Force**: `@x` — evaluates and consumes the thunk (one-shot)
- **Type system**: `@T` tracked as linear; unconsumed `@T` is a compile error (including primitives like `@i64`)
- **Bare-name access**: `x` (without `@`) references the thunk without forcing — enables `cancel(a: x)` / `detach(a: x)`
- **Closure linearization**: Capturing `@x` in a closure makes the closure linear
- **DAG parallel evaluation**: Compiler detects 2+ consecutive forces and emits `LazySpawn`/`LazyJoin` for parallel execution via OS threads
- **Runtime**: `nexus:runtime/lazy` host module with `__nx_lazy_spawn(thunk, num_captures) -> task_id` and `__nx_lazy_join(task_id) -> result`
- **stdlib/lazy.nx**: `race`, `cancel`, `detach`, `force_all` combinators

---

See also: [Exception Groups](../exception-groups), [Types](../types), [Syntax](../syntax)
