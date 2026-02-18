# Effect System

Nexus uses a row-based effect system to track and manage side effects.

## `perform` Discipline

- Effectful calls must use `perform`.
- Pure calls must not use `perform`.

## Syntax

Effects are declared using the `effect` keyword in function signatures.

```nexus
fn print_val(x: i64) -> unit effect { Console } do
  let x_str = i64_to_string(val: x)
  let msg = [=[Value: ]=] ++ x_str
  perform print(val: msg)
endfn
```

### Row Types

Effects are represented as rows, which can be closed or open (polymorphic).

- **Closed Row**: `{ E1, E2 }`. Exactly these effects.
- **Open Row**: `{ E1 | r }`. Includes `E1` and any other effects represented by `r`.

## Effect Polymorphism

Functions can be polymorphic over effects using effect variables.

```nexus
fn apply<E>(f: () -> unit effect E) -> unit effect E do
  perform f()
endfn
```

When `apply` is called, `E` is unified with the actual effects of the passed function.

## Exception Handling

Nexus provides a native exception effect `Exn`.
You can extend `Exn` with top-level exception constructors.

```nexus
exception NotFound(string)
exception PermissionDenied(string, i64)
```

### Raising Exceptions

```nexus
let err = NotFound([=[something went wrong]=])
raise err
```

### Catching Exceptions

```nexus
try
  perform risky_action()
catch e ->
  match e do
    case NotFound(msg) -> perform handle_error(msg: msg)
    case PermissionDenied(path, code) -> perform handle_perm(path: path, code: code)
  endmatch
endtry
```

The `try-catch` block handles the `Exn` effect, removing it from the resulting effect row of the block.

Runtime failures are also reified as `Exn`:

```nexus
case RuntimeError(msg) -> ...
case InvalidIndex(i) -> ...
```

## Ports and Handlers

Nexus provides a mechanism for defining abstract interfaces (`port`) and their implementations (`handler`). This allows for modular effect handling and dependency injection.

### Port Definition

A `port` defines a set of effectful operations (an interface).

```nexus
port KeyValueStore do
  fn get(key: string) -> string
  fn set(key: string, val: string) -> unit
endport
```

Functions in a port implicitly have an effect corresponding to the port itself (e.g., `KeyValueStore`).

### Handler Definition

A `handler` implements a specific port.

```nexus
handler InMemoryKVS for KeyValueStore do
  fn get(key: string) -> string do
    // ... implementation ...
    return [=[value]=]
  endfn

  fn set(key: string, val: string) -> unit do
    // ... implementation ...
    return ()
  endfn
endhandler
```

Currently, Nexus supports **static global dispatch** for handlers. If a handler is defined for a port, calls to that port's functions (e.g., `perform KeyValueStore.get(key: "k")`) are automatically dispatched to the registered handler.

## Subtyping

Subtyping is **not** supported in the current version of the effect system. All effect matching is done via unification of rows.
