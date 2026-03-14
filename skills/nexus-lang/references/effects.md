# Nexus Effects & Coeffects Reference

## Overview

Nexus separates function dependencies into two categories:
- **Coeffects** (`require`): What capabilities the function needs from its environment
- **Checked exceptions** (`throws`): What exceptions the function may throw

Function signature structure:
```
fn (args...) -> ReturnType require { Coeffects } throws { Exceptions }
```

Both `require` and `throws` are optional.

## Coeffects: Ports & Handlers

### Ports (Interfaces)

A port declares an abstract interface that the environment must provide:

```nexus
export port Logger do
  fn info(msg: string) -> unit
  fn error(msg: string) -> unit
end

export port UserRepository do
  fn find(id: i64) -> Option<User>
  fn save(user: User) -> Result<unit, string>
end

// Port methods can themselves require/throw
export port DataService do
  fn fetch(url: string) -> string throws { Exn }
end
```

### Handlers (Implementations)

Handlers implement port methods as ordinary values:

```nexus
// Simple handler
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

// Handler for testing (no deps)
let mock_logger = handler Logger do
  fn info(msg: string) -> unit do
    return ()
  end
  fn error(msg: string) -> unit do
    return ()
  end
end
```

Rules:
- Handler must implement **all** port methods (exhaustive)
- Method signatures must **exactly match** the port
- Handler's `require` clause propagates to callers
- Handler is a value — can be stored, passed, returned

### Using Ports (`require` + `inject`)

```nexus
// 1. Declare requirement
let process = fn (data: string) -> unit require { Logger, Console } do
  Logger.info(msg: "Processing: " ++ data)
  Console.println(val: data)
  return ()
end

// 2. Inject handler to satisfy requirement
inject console_logger do
  process(data: "hello")    // Logger requirement satisfied
end

// Multiple handlers injected simultaneously
inject handler_a, handler_b do
  some_function()
end
```

### Injection Rules

1. `inject` must **reduce** the set of unsatisfied requirements
2. Multiple handlers can be injected in one `inject` statement
3. Handler requirements propagate upward to the caller
4. Injection scope is lexical (`do ... end` block)

### Calling Port Methods

```nexus
// Port methods called as: PortName.method_name(args...)
Console.println(val: "hello")
Logger.info(msg: "message")
Net.get(url: "https://example.com")

// Port calls are statically resolved to direct function calls
// (NOT dynamic dispatch — no vtable, no funcref)
```

## Runtime Permissions

The `main` function's `require` clause lists the program's runtime permissions:

| Permission | What it grants | CLI Flag |
|-----------|---------------|----------|
| `PermConsole` | stdin/stdout/stderr | `--allow-console` |
| `PermFs` | Filesystem access | `--allow-fs` |
| `PermNet` | HTTP client/server, sockets | `--allow-net` |
| `PermRandom` | Random number generation | `--allow-random` |
| `PermClock` | Wall clock, monotonic clock, sleep | `--allow-clock` |
| `PermProc` | Process exit, environment | `--allow-proc` |
| `PermEnv` | Environment variables | `--allow-env` |

```nexus
let main = fn () -> unit require { PermConsole, PermFs, PermNet } do
  inject stdio.system_handler, fs_mod.system_handler, net_mod.system_handler do
    // all three permissions available here
  end
  return ()
end
```

### Permission ↔ Stdlib Handler Mapping

Each stdlib I/O module provides a `system_handler`:

```nexus
import { Console }, * as stdio from stdlib/stdio.nx      // stdio.system_handler → PermConsole
import { Fs }, * as fs_mod from stdlib/fs.nx              // fs_mod.system_handler → PermFs
import { Net }, * as net_mod from stdlib/net.nx           // net_mod.system_handler → PermNet
import { Random }, * as rand_mod from stdlib/random.nx    // rand_mod.system_handler → PermRandom
import { Clock }, * as clock_mod from stdlib/clock.nx     // clock_mod.system_handler → PermClock
import { Proc }, * as proc_mod from stdlib/proc.nx        // proc_mod.system_handler → PermProc
import { Env }, * as env_mod from stdlib/env.nx           // env_mod.system_handler → PermEnv
```

## Checked Exceptions

```nexus
// Declare exception
exception NotFound(msg: string)
exception Timeout(ms: i64, url: string)

// Throw
raise NotFound(msg: "user 42 not found")

// Catch
try
  let result = risky_operation()
  process(val: result)
catch e ->
  match e do
    case NotFound(msg: m) ->
      Console.println(val: "Not found: " ++ m)
    case Timeout(ms: t, url: u) ->
      Console.println(val: "Timeout after " ++ from_i64(val: t) ++ "ms")
    case _ ->
      Console.println(val: "Unknown error")
  end
end
```

### Exception Rules

- `raise` is an expression that never returns (diverges)
- `try/catch` discharges `throws { Exn }` from the protected block
- Functions that may raise must declare `throws { Exn }`
- `main` must have empty `throws` (all exceptions handled)
- Built-in root type: `Exn`

## Main Function Constraints

```nexus
// MUST:
// - Return unit
// - Have empty throws (handle all exceptions)
// MAY:
// - Require runtime permissions
// MUST NOT:
// - Be export
// - Throw unhandled exceptions

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    try
      do_work()
    catch e ->
      Console.println(val: "Error occurred")
    end
  end
  return ()
end
```

## Row Typing (Advanced)

Coeffect and exception types use row typing for polymorphism:

```nexus
// A function polymorphic over additional requirements
let wrap = fn <R>(f: () -> unit require { Logger | R }) -> unit require { Logger | R } do
  Logger.info(msg: "before")
  f()
  Logger.info(msg: "after")
  return ()
end
```

Row compatibility is structural and order-independent.
