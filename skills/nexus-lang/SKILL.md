---
name: nexus-lang
description: "Write Nexus (.nx) programs — an LLM-friendly language with linear types, coeffects (ports/handlers), and WASM compilation. Use when writing, reviewing, or explaining Nexus code."
---

# Nexus Language

Nexus is a programming language designed for LLM-friendly code generation. Its premise: **"LLMs are strong at literal program constructs but weak at contextual ones."** Every construct is syntactically explicit — no implicit resource cleanup, no hidden aliasing, no ambient I/O.

**Documentation**: https://nymphium.github.io/Nexus/latest/

## When to Use This Skill

- Writing new `.nx` source files
- Reviewing or debugging Nexus code
- Explaining Nexus language features
- Porting algorithms to Nexus
- Working with Nexus's type system, ports/handlers, or linear types

## When NOT to Use This Skill

- Non-Nexus programming tasks
- Modifying the Nexus compiler itself (that's Rust code)

## Quick Reference

### File Extension
`.nx`

### CLI Commands
```
nexus                    # REPL
nexus run example.nx     # compile + run via WASM
nexus build example.nx   # compile to main.wasm
nexus check example.nx   # typecheck only
```

### Permission Flags (for run/build)
```
--allow-console   --allow-fs    --allow-net
--allow-random    --allow-clock --allow-proc   --allow-env
--preopen DIR     # (requires --allow-fs)
```

## Core Syntax Rules

### 1. All arguments are labeled
```nexus
// CORRECT
add(a: 1, b: 2)
Console.println(val: "hello")
Cons(v: x, rest: xs)

// WRONG — positional arguments do not exist
add(1, 2)
Console.println("hello")
```

### 2. Blocks use `do ... end` (no braces)
```nexus
if x > 0 then
  return x
else
  return 0
end

match opt do
  case Some(val: v) -> return v
  case None -> return 0
end

while running do
  process()
end
```

### 3. Functions are `let` bindings to lambdas
```nexus
export let add = fn (a: i64, b: i64) -> i64 do
  return a + b
end

// With generics
export let map = fn <T, U>(opt: Option<T>, f: (val: T) -> U) -> Option<U> do
  match opt do
    case Some(val: v) ->
      let mapped = f(val: v)
      return Some(val: mapped)
    case None -> return None
  end
end
```

### 4. Sigils: `%` (linear), `&` (borrow), `~` (mutable ref)
```nexus
let %arr = [| 1, 2, 3 |]       // linear — must be consumed exactly once
let lock = &%arr                 // borrow — immutable view, does not consume
lock[0] <- 42                   // mutation via borrow
let ~x = 10                     // mutable ref, stack-confined
~x <- 20                        // reassignment
let v = ~x                      // dereference
```

### 5. Explicit `return` required
```nexus
// Every function must have explicit return
let double = fn (n: i64) -> i64 do
  return n * 2
end
```

## Effect System (Ports & Handlers)

Nexus uses coeffects for dependency injection, NOT algebraic effects. See `./references/effects.md` for details.

```nexus
// 1. Define a port (interface)
port Logger do
  fn info(msg: string) -> unit
  fn error(msg: string) -> unit
end

// 2. Implement via handler
let console_logger = handler Logger require { Console } do
  fn info(msg: string) -> unit do
    Console.println(val: "[INFO] " ++ msg)
    return ()
  end
  fn error(msg: string) -> unit do
    Console.println(val: "[ERROR] " ++ msg)
    return ()
  end
end

// 3. Require port in functions
let greet = fn (name: string) -> unit require { Logger } do
  Logger.info(msg: "Hello, " ++ name)
  return ()
end

// 4. Inject handler at call site
let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    inject console_logger do
      greet(name: "World")
    end
  end
  return ()
end
```

## Type System Summary

| Type | Syntax | Notes |
|------|--------|-------|
| Primitives | `i32`, `i64`, `f32`, `f64`/`float`, `bool`, `string`, `unit` | `i64` and `f64` are defaults |
| Record | `{ x: i64, y: i64 }` | Structural typing |
| ADT/Enum | `Ok(val: T) \| Err(err: E)` | Labeled fields |
| List | `[ T ]` | Immutable singly-linked |
| Array | `[| T |]` | Linear, mutable |
| Function | `(a: i64) -> i64` | Always labeled params |
| Generic | `Option<T>`, `Result<T, E>` | Explicit type params |
| Linear | `%T` | Must consume exactly once |
| Borrow | `&T` | Immutable view |
| Opaque | `opaque type X = ...` | Hidden constructors |

## Common Patterns

### Hello World
```nexus
import { Console }, * as stdio from stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello, World!")
  end
  return ()
end
```

### Error Handling
```nexus
import { Result, Ok, Err } from stdlib/result.nx

exception NotFound(msg: string)

let find_user = fn (id: i64) -> string throws { Exn } do
  if id == 1 then
    return "Alice"
  else
    raise NotFound(msg: "User not found")
  end
end

// Catching exceptions
try
  let name = find_user(id: 42)
  Console.println(val: name)
catch e ->
  match e do
    case NotFound(msg: m) -> Console.println(val: m)
    case _ -> Console.println(val: "Unknown error")
  end
end
```

### List Recursion
```nexus
import * as list from stdlib/list.nx

let sum = fn (xs: [ i64 ]) -> i64 do
  match xs do
    case Nil -> return 0
    case Cons(v: v, rest: rest) -> return v + sum(xs: rest)
  end
end

// With fold
let sum2 = fn (xs: [ i64 ]) -> i64 do
  return list.fold_left(xs: xs, init: 0, f: fn (acc: i64, val: i64) -> i64 do
    return acc + val
  end)
end
```

### Concurrency
```nexus
conc do
  task fetch_data do
    let data = fetch(url: endpoint)
    Logger.info(msg: "Fetched data")
  end
  task log_start do
    Logger.info(msg: "Request started")
  end
end
// Both tasks complete before execution continues
```

## Anti-Patterns to Avoid

| Avoid | Use Instead |
|-------|-------------|
| Positional arguments: `f(1, 2)` | Labeled: `f(a: 1, b: 2)` |
| Brace blocks: `{ ... }` | `do ... end` / `then ... end` |
| `return` omitted | Always write explicit `return` |
| Capturing `~x` in closure | Only immutable bindings captured |
| `let _ = linear_val` | Consume linear values via function call or pattern match |
| Implicit I/O | Declare via `require { PermConsole }` + inject handler |
| `var x = 5` | `let ~x = 5` for mutable |
| `for x in list` | Use `match`/recursion or `list.fold_left` |

## Reference Files

- `./references/syntax.md` — Complete EBNF grammar
- `./references/types.md` — Type system, linear types, borrowing
- `./references/effects.md` — Ports, handlers, inject, permissions
- `./references/stdlib.md` — Standard library API reference
- `./references/patterns.md` — Idiomatic code patterns with examples
- `./templates/` — Starter templates for common program structures
