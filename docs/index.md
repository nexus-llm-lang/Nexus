---
layout: default
title: Nexus Language
---

# Nexus

Nexus is an **LLM-friendly** programming language designed for maximum readability, safety, and seamless integration with modern AI-assisted development workflows.

## Why Nexus?

- **Explicit Context:** Keyword-terminated blocks (`endfn`, `endmatch`) provide clear boundaries for LLM context windows.
- **Labeled Clarity:** Mandatory labeled arguments at call sites reduce ambiguity for both humans and AI.
- **Predictable Structure:** Strict A-Normal Form (ANF) ensures the language is easy to reason about and transform.
- **Human-Reviewable:** The same properties that help LLMs make LLM-generated code straightforward for humans to verify.

## Quick Start

```bash
# REPL
nexus

# Run a script
nexus run example.nx

# Build packed executable
nexus build example.nx

# Build component wasm
nexus build example.nx --wasm
```

## Hello World

```nexus
let main = fn () -> unit effect { Console } do
  print(val: [=[Hello, Nexus!]=])
  return ()
endfn
```

## Features at a Glance

### Resource Management via Sigils

| Sigil | Meaning | Example |
|---|---|---|
| (none) | Immutable binding | `let x = 10` |
| `~` | Mutable (stack-confined) | `let ~count = 0` |
| `%` | Linear (must be consumed) | `let %handle = open()` |
| `&` | Borrowed (read-only view) | `let &view = &%arr` |

Primitive linear values are **auto-dropped** at scope end. Composite types (Records, ADTs) are consumed via pattern matching.

### Effect System

Functions declare their side effects and environment requirements:

```nexus
let fetch = fn (url: string) -> string require { Net } effect { Console } do
  return net.get(url: url)
endfn
```

- `effect { ... }` -- builtin runtime effects (`Console`, `Exn`)
- `require { ... }` -- coeffects via ports and handlers

### Algebraic Data Types

```nexus
pub type Result<T, E> = Ok(val: T) | Err(err: E)

match result do
  case Ok(val: v) -> process(v: v)
  case Err(err: e) -> handle_error(e: e)
endmatch
```

### Wasm-First

Nexus targets the WebAssembly component model for portable, secure deployment:

```bash
nexus build program.nx --wasm
wasmtime run -Scli -Shttp -Sinherit-network main.wasm
```

## Example: Fibonacci

```nexus
let fib = fn (n: i64) -> i64 do
  if n <= 1 then
    return n
  endif
  return fib(n: n - 1) + fib(n: n - 2)
endfn

let main = fn () -> unit effect { Console } do
  let s = i64_to_string(val: fib(n: 10))
  print(val: [=[fib(10) = ]=] ++ s)
  return ()
endfn
```

## Documentation

### Language Specification

- [Syntax](spec/syntax.md) -- Grammar and EBNF
- [Types](spec/types.md) -- Type system and inference
- [Semantics](spec/semantics.md) -- Evaluation model
- [Memory Management](spec/memory.md) -- Linear types, borrowing, auto-drop
- [Effect System](spec/effect_system.md) -- Effects and coeffects

### Environment

- [CLI](cli.md) -- Command-line interface
- [FFI](env/ffi.md) -- Wasm interop
- [Runtime](env/runtime.md) -- Entrypoint and execution
- [Standard Library](env/stdlib.md) -- Builtin modules

## Source

[GitHub Repository](https://github.com/Nymphium/nexus)
