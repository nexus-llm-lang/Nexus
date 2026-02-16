# Effect System

Nexus uses a row-based effect system to track and manage side effects.

## Syntax

Effects are declared using the `effect` keyword in function signatures.

```nexus
fn print_val(x: i64) -> unit effect { Console } do
  perform printf(fmt: "%d", val: x)
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

## Subtyping

Subtyping is **not** supported in the current version of the effect system. All effect matching is done via unification of rows.
