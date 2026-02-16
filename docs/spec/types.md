# Types

Nexus uses a strict, nominal type system with Hindley-Milner type inference.

## Primitive Types

- `i64`: 64-bit signed integer.
- `bool`: Boolean (`true` / `false`).
- `str`: UTF-8 string.
- `unit`: The unit type `()` (similar to `void`).

## Compound Types

### Records

Records are anonymous structures with named fields.

```nexus
type User = {
  id: i64,
  name: str
}
```

Field access uses `.`.

```nexus
let u = {id: 1, name: "Nexus"}
let name = u.name
```

### Variants (Algebraic Data Types)

Currently, Nexus supports `Result<T, E>` and user-defined variants implicitly via pattern matching.
Constructors like `Ok(T)` and `Err(E)` are built-in for `Result`.

## Generics

Functions can be generic over types.

```nexus
fn id<T>(x: T) -> T do
  return x
endfn
```

Nexus enforces parametricity (cannot inspect generic values unless constrained).

## Type Inference

Nexus infers types for local variables (`let`).
However, top-level function signatures must be explicitly typed.
