# Effect and Coeffect System

Nexus separates runtime side effects and environment requirements:

- `effect { ... }`: builtin runtime effects.
- `require { ... }`: coeffects (requirements on provided ports/handlers).

Both are row-like signatures tracked by the type checker.

## Function Signature Shape

```nexus
fn (args...) -> Ret require { Coeffects } effect { Effects }
```

Examples:

```nexus
let pure = fn (x: i64) -> i64 do
  return x + 1
endfn

let log = fn (msg: string) -> unit effect { Console } do
  print(val: msg)
  return ()
endfn

let fetch = fn (url: string) -> string require { Net } do
  return Net.get(url: url)
endfn
```

## Builtin Effects

Current builtin effects are:

- `Console`: console I/O (`print`, etc.)
- `Exn`: exception raising/propagation

`try ... catch` discharges `Exn` from the protected block.

```nexus
let risky = fn () -> unit effect { Exn } do
  raise RuntimeError(msg: [=[oops]=])
endfn

let main = fn () -> unit effect { Console } do
  try
    risky()
  catch e ->
    print(val: [=[recovered]=])
  endtry
  return ()
endfn
```

## Coeffects via Ports

`port` declarations define required capabilities (coeffects), not builtin effects.

```nexus
port Logger do
  fn info(msg: string) -> unit
endport

let program = fn () -> unit require { Logger } do
  Logger.info(msg: [=[hello]=])
  return ()
endfn
```

## Handler and Inject

A handler value implements one port:

```nexus
let logger = handler Logger do
  fn info(msg: string) -> unit do
    print(val: msg)
    return ()
  endfn
endhandler
```

`inject` supplies handler values to a lexical scope and discharges matching requirements:

```nexus
let main = fn () -> unit effect { Console } do
  inject logger do
    program()
  endinject
  return ()
endfn
```

Type checking enforces:

- Handler methods must match the corresponding port signatures.
- Handler methods must be exhaustive for the port (no missing methods).
- `inject` must reduce requirements; injecting an unused handler is a type error.

## Main Constraints

`main` is intentionally constrained:

- `() -> unit`
- `effect` may contain only `Console` (or be empty)
- `require` may contain only `Net` and/or `Fs` (or be empty)

This maps to capability-style component execution (for example WASI networking/filesystem).

## Subtyping

Nexus does not implement effect/coeffect subtyping today. Compatibility is checked by row unification.
