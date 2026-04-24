---
layout: default
title: Checked Exceptions and Capabilities
---

# Checked Exceptions and Capabilities

Nexus separates two concerns in function signatures: **capabilities** (what the function needs from its environment) and **checked exceptions** (what exceptions the function may throw). This distinction is central to the [design thesis](../../design#why-capabilities-not-effects) -- every dependency and side effect is declared, not implied.

> **Terminology note.** The `require` clause is sometimes called a *coeffect* in the literature, as it tracks environmental requirements (the dual of effects). We use *capability* because Nexus's system — row-polymorphic, discharged by `inject`, annotations on function signatures — differs structurally from coeffect calculi (Petricek et al.), which use semiring-indexed annotations on individual variable bindings.

## Function Signature Shape

```nexus
fn (args...) -> Ret require { Capabilities } throws { Exceptions }
```

Both clauses are optional. Omitted means empty row (pure function with no requirements).

```nexus
let pure = fn (x: i64) -> i64 do return x + 1 end

let greet = fn (msg: string) -> unit require { Console } do
  Console.println(val: msg)
  return ()
end

let risky = fn () -> unit throws { Exn } do
  raise RuntimeError(val: "oops")
end
```

## Checked Exceptions

The only builtin throws type is `Exn`. `try/catch` discharges `Exn` from the protected block:

```nexus
exception NotFound(msg: string)

let search = fn (key: string) -> string throws { Exn } do
  raise NotFound(msg: key)
end

let main = fn () -> unit do
  try
    let _ = search(key: "missing")
  catch e ->
    match e do
      | NotFound(msg: m) -> ()
      | _ -> ()
    end
  end
  return ()
end
```

Exception declarations extend the builtin `Exn` type:

```nexus
export exception PermissionDenied(msg: string, code: i64)
```

`raise` is an expression that immediately unwinds to the nearest `catch`. All I/O capabilities use the capability system, not throws.

## Ports

A `port` defines a capability interface -- a set of function signatures that must be provided by the environment:

```nexus
export port Logger do
  fn info(msg: string) -> unit
  fn warn(msg: string) -> unit
end
```

When a function calls `Logger.info(...)`, it must have `Logger` in its `require` row.

Port methods can themselves declare throws and capabilities:

```nexus
export port Fs do
  fn open_read(path: string) -> %Handle throws { Exn }
  fn read(handle: %Handle) -> { content: string, handle: %Handle }
  fn close(handle: %Handle) -> unit
end
```

## Handlers

A handler is a value that implements all methods of a port:

```nexus
let console_logger = handler Logger require { Console } do
  fn info(msg: string) -> unit do
    Console.println(val: "[INFO] " ++ msg)
    return ()
  end
  fn warn(msg: string) -> unit do
    Console.println(val: "[WARN] " ++ msg)
    return ()
  end
end

let mock_logger = handler Logger do
  fn info(msg: string) -> unit do return () end
  fn warn(msg: string) -> unit do return () end
end
```

Handler `require { ... }` propagates: injecting `console_logger` adds `Console` to the caller's requirements.

The type checker enforces:
- Handler methods must match port signatures exactly
- All port methods must be implemented (exhaustive)
- Handler method bodies inherit the handler's `require` clause

## Inject

`inject` supplies handler values to a lexical scope, discharging matching `require` entries:

```nexus
inject stdio.system_handler do
  inject console_logger do
    program()  // program's Logger + Console requirements satisfied
  end
end
```

Rules:
- `inject` must reduce requirements -- injecting an unused handler is a type error
- Multiple handlers can be injected in a single `inject` statement: `inject h1, h2 do ... end`
- Handler requirements propagate to the enclosing scope

## Exception Groups

Exception groups let you catch multiple related exceptions with a single pattern. See [Exception Groups](../exception-groups) for the full reference.

```nexus
exception NotFound(path: string)
exception PermDenied(path: string)
exception group IOError = NotFound | PermDenied

let safe_read = fn (path: string) -> string require { Fs } do
  try
    return Fs.read_to_string(path: path)
  catch
    | IOError -> return ""
  end
end
```

## Main Constraints

The `main` function has special restrictions:

- Signature: `() -> unit`
- `throws` must be empty (all exceptions must be handled internally)
- `require` may contain any subset of runtime permissions: `{ PermFs, PermNet, PermConsole, PermRandom, PermClock, PermProc, PermEnv }`

```nexus
let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello")
  end
  return ()
end
```

## Permission Mapping

Runtime permissions (`PermFs`, `PermNet`, etc.) are special capabilities that map to WASI capabilities. They serve as the bridge between the type system and the runtime sandbox. See [WASM and WASI](../../env/wasm) for the complete mapping table.

## Row Typing

Throws and capability rows are checked by row unification (no subtyping). Open rows use tail variables for polymorphism:

```nexus
// This function is polymorphic over additional requirements
let log_and_do = fn <R>(f: () -> unit require { Logger | R }) -> unit require { Logger | R } do
  Logger.info(msg: "starting")
  f()
  return ()
end
```

Compatibility is structural -- two rows unify if they contain the same entries (order-independent).
