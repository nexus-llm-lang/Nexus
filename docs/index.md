---
layout: default
title: Nexus Language
---

# Nexus

Nexus is a programming language built on one premise: **LLMs are strong at literal program constructs but weak at contextual ones.**

Garbage collection, implicit conversions, ambient I/O, continuation-based control flow -- these contextual mechanisms are where LLM-generated code breaks and where human review fails. Nexus replaces them with syntactically explicit alternatives: linear types for resource tracking, mandatory borrow annotations for aliasing, coeffects for capability requirements, and keyword-terminated blocks for structure.

## Quick Start

```bash
nexus                             # REPL
nexus run example.nx              # interpret
nexus build example.nx            # compile to main.wasm
nexus check example.nx            # typecheck only
```

## Hello World

```nexus
import { Console }, * as stdio from stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
    inject stdio.system_handler do
        Console.println(val: "Hello, Nexus!")
    end
    return ()
end
```

Everything is explicit: the function requires `PermConsole`, a handler is injected to satisfy it, and the `main` entrypoint returns `unit`.

## Coeffects, Not Effects

Most effect systems give handlers the power to *resume* computations via delimited continuations. This makes control flow *contextual* -- what happens after an effect operation depends on which handler is installed, and whether it chooses to resume.

Nexus rejects continuations entirely. Instead, it uses **coeffects**: the `require { ... }` clause declares capabilities a function *needs from its environment*.

```nexus
fn (args...) -> RetType require { Coeffects } effect { Effects }
```

- `require { ... }` -- coeffects. Capabilities provided via ports and handlers.
- `effect { ... }` -- builtin effects. Only `Exn` (exceptions with traditional unwind semantics).

### Ports and Handlers

A **port** defines an interface. A **handler** implements it. `inject` supplies the handler.

```nexus
port Logger do
    fn info(msg: string) -> unit
end

let console_logger = handler Logger require { Console } do
    fn info(msg: string) -> unit do
        Console.println(val: "[INFO] " ++ msg)
        return ()
    end
end

let program = fn () -> unit require { Logger } do
    Logger.info(msg: "starting")
    return ()
end

let main = fn () -> unit require { PermConsole } do
    inject stdio.system_handler do
        inject console_logger do
            program()
        end
    end
    return ()
end
```

Port calls are **direct method calls** resolved statically -- not effect operations that suspend into a continuation. Handlers are values; `inject` is dependency injection, not algebraic effect handling.

### Why Not Algebraic Effects?

Algebraic effect handlers with continuations let a handler decide whether and how to resume a suspended computation. This is powerful but fundamentally *contextual*: the call site `Logger.info(...)` tells you nothing about control flow -- the handler could resume, abort, restart, or run the continuation multiple times.

Nexus makes the tradeoff explicit: less expressive handlers in exchange for **every call site meaning exactly what it says**. `Logger.info(msg: x)` calls a function. It returns. No implicit control transfer.

## Resource Management

### Linear Types (`%`)

Linear bindings must be consumed **exactly once**:

```nexus
let %h = Fs.open_read(path: path)
let %r = Fs.read(handle: %h)
match %r do
    case { content: c, handle: %h2 } ->
        Fs.close(handle: %h2)
end
```

Fail to consume, consume twice, or discard with `_` -- the compiler rejects it. No garbage collector, no finalizers.

### Borrowing (`&`)

Read without consuming:

```nexus
let server = Net.listen(addr: addr)
let req = Net.accept(server: &server)    -- borrow server
let method = request_method(req: &req)   -- borrow req
let _ = Net.respond(req: req, ...)       -- consume req
Net.stop(server: server)                 -- consume server
```

`&T` coerces to `T` for reading. The original binding remains live.

### Mutable References (`~`)

Stack-confined mutation. Cannot escape the defining function (Gravity Rule):

```nexus
let ~count = 0
~count <- ~count + 1
```

### Sigil Summary

| Sigil | Meaning | Scope |
|---|---|---|
| (none) | Immutable | Lexical |
| `~` | Mutable | Stack-confined |
| `%` | Linear (consumed once) | Lexical, tracked |
| `&` | Borrowed (read-only view) | Temporary |

## Control Flow

### If-Else

```nexus
if condition then
    // ...
else
    // ...
end
```

### Match (Statement and Expression)

Match works both as a statement and as an expression:

```nexus
// Statement — each case contains statements
match result do
    case Ok(val: v) -> return process(v: v)
    case Err(err: e) -> return handle_error(e: e)
end

// Expression — each case produces a value
let code = match color do
    case Red -> 1
    case Green -> 2
    case Blue -> 3
end
```

### While Loop

```nexus
let ~sum = 0
let ~i = 0
while ~i < 10 do
    ~sum <- ~sum + ~i
    ~i <- ~i + 1
end
```

### For Loop

```nexus
let ~product = 1
for i = 1 to 6 do
    ~product <- ~product * i
end
// ~product is 120 (5!)
```

The loop variable `i` is immutable and scoped to the loop body. The range is `[start, end)` (exclusive upper bound).

## Exception Handling

Traditional unwind semantics. No continuations.

```nexus
exception NotFound(msg: string)

let search = fn (key: string) -> string effect { Exn } do
    raise NotFound(msg: key)
end

let main = fn () -> unit do
    try
        let _ = search(key: "missing")
    catch e ->
        match e do
            case NotFound(msg: m) -> ()
            case _ -> ()
        end
    end
    return ()
end
```

`try/catch` discharges `Exn` from the protected block. This is the *only* builtin effect.

## Capability-Based Security

Runtime permissions map to WASI capabilities:

| Permission | WASI Capability |
|---|---|
| `PermFs` | Filesystem |
| `PermNet` | Network, HTTP |
| `PermConsole` | stdio |
| `PermRandom` | Random |
| `PermClock` | Clocks |
| `PermProc` | Process |
| `PermEnv` | Environment variables |

```nexus
let main = fn () -> unit require { PermNet, PermConsole } do
    inject net_mod.system_handler, stdio.system_handler do
        let body = Net.get(url: "https://example.com")
        Console.println(val: body)
    end
    return ()
end
```

Checked at compile time. Enforced at the WASI runtime level.

## Algebraic Data Types

```nexus
pub type Result<T, E> = Ok(val: T) | Err(err: E)

match result do
    case Ok(val: v) -> process(v: v)
    case Err(err: e) -> handle_error(e: e)
end
```

Labeled constructor arguments. Exhaustive pattern matching.

## Structured Concurrency

```nexus
conc do
    task worker1 do
        // runs in parallel
    end
    task worker2 do
        // runs in parallel
    end
end
// both tasks complete before continuing
```

## WebAssembly Target

Nexus compiles to the WASM component model:

```bash
nexus build program.nx
wasmtime run -Scli -Shttp -Sinherit-network main.wasm
```

`main` must return `unit`. Permissions in `require { ... }` determine which WASI capabilities are requested.

## Documentation

### Design

- [Design Thesis](design) -- Why every construct is literal, not contextual

### Language Specification

- [Syntax](spec/syntax) -- Grammar and EBNF
- [Types](spec/types) -- Type system, linear types, borrowing, memory management
- [Effects and Coeffects](spec/effects) -- Ports, handlers, inject, exceptions
- [Semantics](spec/semantics) -- Evaluation model, entrypoint, concurrency

### Environment

- [CLI](env/cli) -- Command-line interface
- [WASM and WASI](env/wasm) -- Capability mapping and enforcement
- [FFI](env/ffi) -- Wasm interop
- [Standard Library](env/stdlib) -- Builtin modules

## Source

[GitHub Repository](https://github.com/Nymphium/Nexus)
