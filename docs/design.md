---
layout: default
title: Design
---

# Design

Nexus is built on one observation: **LLMs are strong at literal program constructs but weak at contextual ones.** Garbage collection, implicit conversions, ambient I/O, continuation-based control flow -- these contextual mechanisms are where LLM-generated code breaks and where human review fails. Nexus replaces each with a syntactically explicit alternative.

## Literal vs Contextual

| Contextual (eliminated) | Literal (Nexus alternative) |
|---|---|
| Implicit resource cleanup (GC, finalizers) | `%` linear types -- consumed exactly once |
| Hidden aliasing | `&` borrow -- explicit read-only view |
| Ambient I/O | `require { PermNet }` -- declared capability |
| Implicit control transfer (continuations) | `try/catch` -- traditional unwind semantics |
| Positional arguments | `add(a: 1, b: 2)` -- mandatory labeled arguments |
| Brace-matching (off-by-one) | `do ... end` -- keyword-terminated blocks |
| Implicit scope termination | `if ... then ... end`, `match ... do ... end` |

The principle is simple: if a construct requires looking elsewhere to understand what happens here, replace it with something that doesn't.

## Why Capabilities, Not Effects

Most effect system research focuses on *algebraic effects* -- functions perform effect operations, and handlers intercept them using delimited continuations. The handler decides whether and how to resume the suspended computation. This is powerful but fundamentally *contextual*: the call site `Logger.info(msg: x)` tells you nothing about control flow. The handler could resume, abort, restart, or run the continuation multiple times.

Nexus rejects continuations entirely. Instead:

- **Ports** define stateless interfaces (like traits or interfaces in other languages).
- **Handlers** are ordinary values that implement a port's methods.
- **`inject`** supplies a handler to a lexical scope -- dependency injection, not algebraic effect handling.
- **Port calls** are direct, statically resolved function calls. `Logger.info(msg: x)` calls a function. It returns. No implicit control transfer.

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

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler, console_logger do
    Logger.info(msg: "starting")
  end
  return ()
end
```

The tradeoff is explicit: less expressive handlers in exchange for every call site meaning exactly what it says.

## Linear Types as Literal Resource Tracking

Garbage collectors and finalizers are contextual -- resources disappear "sometime later" through an invisible mechanism. Nexus makes resource lifecycle visible in syntax with the `%` sigil:

```nexus
let %h = Fs.open_read(path: path)   // acquire
let %r = Fs.read(handle: %h)    // consume %h, get new handle back
match %r do
  case { content: c, handle: %h2 } ->
    Fs.close(handle: %h2)     // release
end
```

The compiler enforces exactly-once consumption. Fail to consume, consume twice, or discard with `_` -- rejected at compile time. No GC, no finalizers, no implicit drop.

## Borrowing as Explicit Aliasing

Hidden aliasing is a major source of bugs in both human and LLM-generated code. The `&` sigil makes every alias visible:

```nexus
let server = Net.listen(addr: addr)
let req = Net.accept(server: &server)  // &server: borrow, not consume
let method = request_method(req: &req) // &req: borrow, not consume
let _ = Net.respond(req: req, ...)     // consume req
Net.stop(server: server)               // consume server
```

Every read-without-consuming is syntactically marked. No hidden reference counting, no shared pointers, no implicit copies.

## Labeled Arguments and Keyword Blocks

Positional arguments require looking at the function signature to know what each argument means. Brace-delimited blocks require counting braces to find boundaries. Nexus eliminates both:

```nexus
// Every argument is self-documenting
let result = request(
  method: "POST",
  url: "https://api.example.com",
  headers: headers,
  body: payload
)

// Every block has an unambiguous terminator
if condition then
  // ...
end
```

## Capability-Based Security

Ambient authority -- where any function can read files, make network requests, or access the clock -- is deeply contextual. The `require` clause makes the security surface visible at every function boundary:

```nexus
let main = fn () -> unit require { PermNet, PermConsole } do
  inject net_handler, stdio_handler do
    let body = Net.get(url: "https://example.com")
    Console.println(val: body)
  end
  return ()
end
```

`require { PermNet, PermConsole }` is checked at compile time and enforced at the WASI runtime level. A function cannot perform network I/O unless it declares `PermNet` and a handler satisfying `Net` is injected. See [WASM and WASI](../env/wasm) for the permission-to-capability mapping.
