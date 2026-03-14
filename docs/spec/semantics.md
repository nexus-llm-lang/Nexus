---
layout: default
title: Semantics
---

# Semantics

This document describes the execution model of Nexus.

## Evaluation Strategy

Nexus is **call-by-value**. All expressions are fully evaluated before being passed to functions or constructors.

### Evaluation Order

Strict **left-to-right**:
- Function arguments: `f(a: e1, b: e2)` evaluates `e1` before `e2`
- Binary operators: `e1 + e2` evaluates `e1` before `e2`
- Records and constructors: fields evaluated in source order

## Scoping

**Lexical scoping.** Bindings are visible in the block where they are defined and in nested blocks.

**Shadowing** is permitted. An inner `let` can reuse a name from an outer scope, masking it until the inner block ends.

## Sigil Behavioral Semantics

Sigils are not annotations -- they impose runtime behavioral constraints.

### Mutability (`~`)

- **Stack-confined**: mutable bindings exist only on the stack of the defining function
- **No escape**: cannot be returned, stored in heap structures, or captured by closures
- **Assignment**: `~x <- expr` updates the value
- **Concurrency**: cannot be captured by `conc` tasks (prevents data races)

### Linearity (`%`)

- **Exactly-once consumption** (composites): must be consumed via function call, pattern match, or return
- **Auto-drop** (primitives): `i64`, `f64`, `bool`, `string`, `unit` are released at scope end
- **Static enforcement**: the type checker tracks linear bindings and rejects programs that leak or double-use them
- **No discard**: `_` cannot discard composite linear values
- **No mutable ref**: `~` cannot hold linear types

### Borrowing (`&`)

- **Immutable view**: read-only access without consumption
- **Non-consuming**: the source binding remains live
- **Coercion**: `&T` coerces to `T` for reading operations

## Closures and Captures

- **Lexical captures**: lambdas capture immutable bindings from enclosing scope
- **No mutable capture**: closures cannot capture `~` bindings
- **Linearity propagation**: capturing a `%` binding makes the closure linear (single-use)
- **Recursive lambdas**: must use an immutable `let` binding with explicit type annotation

## Exception Propagation

`raise` immediately terminates the current computation and unwinds the call stack until it reaches a `try/catch` block. The `Exn` value is passed to the `catch` parameter:

```nexus
try
  raise NotFound(msg: "key")
catch e ->
  // e : Exn
  match e do
    case NotFound(msg: m) -> ()
    case _ -> ()
  end
end
```

Exceptions are checked -- any function that may raise must declare `throws { Exn }`. `try/catch` discharges `Exn` from the protected region.

## Loops

### While Loop

```nexus
while condition do
  body
end
```

Evaluates `condition` before each iteration. If the condition is `false`, exits. The condition must be `bool`. Returns `unit`.

### For Loop

```nexus
for var = start to end_expr do
  body
end
```

Desugared to:

```nexus
let ~var = start
let ~__end = end_expr
while ~var < ~__end do
  body
  ~var <- ~var + 1
end
```

`start` and `end_expr` must be `i64`. The loop variable is immutable within the body. The range is `[start, end_expr)` (exclusive upper bound). If `start >= end_expr`, the body never executes.

## Match as Expression

Match can appear in expression position. Each case body produces a value:

```nexus
let result = match x do
  case 1 -> 10
  case 2 -> 20
  case _ -> 30
end
```

All non-diverging case bodies must produce the same type. Cases containing a `return` statement diverge and do not contribute to the unified result type.

## Concurrency Model

### Structured Concurrency (`conc`)

```nexus
conc do
  task worker1 do
    // ...
  end
  task worker2 do
    // ...
  end
end
```

- `conc` spawns multiple `task` units and blocks until **all** complete
- Tasks cannot capture mutable (`~`) bindings from the enclosing scope
- The compiled WASM output uses OS-thread parallelism via `std::thread::scope`

## Implicit Unit Return

Functions with return type `unit` may omit the trailing `return ()`. If the function body does not contain any `return` statement, the compiler implicitly appends `return ()`:

```nexus
let greet = fn (name: string) -> unit require { Console } do
  Console.println(val: "Hello, " ++ name)
  // implicit return ()
end
```

Functions with non-`unit` return types still require explicit `return`.

## Entrypoint

### `main` Function

Every Nexus program must define a `main` function with these constraints:

- **Signature**: `() -> unit`
- **Effects**: must be empty (all exceptions handled internally)
- **Requirements**: may include any subset of `{ PermFs, PermNet, PermConsole, PermRandom, PermClock, PermProc, PermEnv }`
- **Visibility**: must not be `export`

```nexus
let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello")
  end
  return ()
end
```

The runtime calls `main`, which performs all side effects via injected handlers. Exit code is `0` on success, non-zero on unhandled error.
