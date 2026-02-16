# Effect System

Nexus uses a row-based effect system to track and manage side effects.

## Syntax

Effects are declared using the `effect` keyword in function signatures.

```nexus
fn print_val(x: i64) -> unit effect { Console } do
  perform print(val: "Value: " ++ to_string(val: x))
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

### Raising Exceptions

```nexus
raise "something went wrong"
```

### Catching Exceptions

```nexus
try
  perform risky_action()
catch e ->
  perform handle_error(msg: e)
endtry
```

The `try-catch` block handles the `Exn` effect, removing it from the resulting effect row of the block.

## Ports and Handlers

Nexus provides a mechanism for defining abstract interfaces (`port`) and their implementations (`handler`). This allows for modular effect handling and dependency injection.

### Port Definition

A `port` defines a set of effectful operations (an interface).

```nexus
port KeyValueStore do
  fn get(key: str) -> str
  fn set(key: str, val: str) -> unit
endport
```

Functions in a port implicitly have an effect corresponding to the port itself (e.g., `KeyValueStore`).

### Handler Definition

A `handler` implements a specific port.

```nexus
handler InMemoryKVS for KeyValueStore do
  fn get(key: str) -> str do
    // ... implementation ...
    return "value"
  endfn

  fn set(key: str, val: str) -> unit do
    // ... implementation ...
    return ()
  endfn
endhandler
```

Currently, Nexus supports **static global dispatch** for handlers. If a handler is defined for a port, calls to that port's functions (e.g., `perform KeyValueStore.get(key: "k")`) are automatically dispatched to the registered handler.

## Subtyping

Subtyping is **not** supported in the current version of the effect system. All effect matching is done via unification of rows.
