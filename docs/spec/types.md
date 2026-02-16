# Types

Nexus uses a strict type system with Hindley-Milner type inference, structural records, and linear types.

## Primitive Types

- `i64`: 64-bit signed integer.
- `float`: 64-bit floating point number.
- `bool`: Boolean (`true` / `false`).
- `str`: UTF-8 string.
- `unit`: The unit type `()`.

## Compound Types

### Records

Records are structural and can be defined using `type`.

```nexus
type User = {
  id: i64,
  name: str
}
```

### Enums (ADTs)

Custom algebraic data types are defined via `enum`.

```nexus
enum Option<T> {
  Some(T),
  None
}
```

### Lists

Immutable lists (vectors) are supported.

```nexus
let l: [i64] = [1, 2, 3]
let nested = [[1], [2]]
```

Lists cannot contain mutable references (`Ref<T>`).

### Arrays

Mutable, linear arrays are supported. Arrays are inherently linear to ensure unique ownership for mutation.

```nexus
let %arr: [| i64 |] = [| 1, 2, 3 |]
%arr[0] <- 42
let val = (borrow %arr)[0]
```

Arrays cannot contain mutable references (`Ref<T>`).

## Memory Management

### Linear Types

Linear types (prefixed with `%`) must be consumed exactly once. They ensure resources (like database transactions) are never leaked or reused inappropriately.

```nexus
let %tx = db.begin_tx()
perform db.commit(tx: %tx) // tx is consumed
```

### Borrowing

The `borrow` keyword allows temporary, immutable access to a linear value without consuming it.

```nexus
fn peek(x: &i64) -> unit do ... endfn

let %x = 10
perform peek(x: borrow %x) // %x is NOT consumed
perform drop_i64(x: %x)    // %x is consumed here
```

### Mutable References

Mutable stack references use `~` and `ref(T)` type. They are restricted by the "Gravity Rule" (cannot be stored in immutable variables or returned from functions).

```nexus
let ~count = 1
~count <- ~count + 1
```

## Effect Types

Function effects are represented as row-polymorphic types `{ E1, E2 | r }`.

```nexus
fn g() -> unit effect { IO, Net } do ... endfn
```

The `perform` keyword can be used to explicitly denote an effectful function call, though it is currently optional.

```nexus
perform g()
```
