# Types

Nexus uses a strict type system with Hindley-Milner type inference, structural records, and linear types.

## Primitive Types

- `i32`: 32-bit signed integer.
- `i64`: 64-bit signed integer.
- `f32`: 32-bit floating point number.
- `f64`: 64-bit floating point number.
- `float`: Alias of `f64`.
- `bool`: Boolean (`true` / `false`).
- `string`: UTF-8 string.
- `unit`: The unit type `()`.

Numeric literals default to `i64` (integers) and `f64` (floats) unless constrained by annotations or surrounding type context.

## Compound Types

### Records

Records are structural and can be defined using `type`.

```nexus
type User = {
  id: i64,
  name: string
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

`drop` is a language statement for explicit consumption.

```nexus
drop <expr>
```

When `val` is linear (for example `%Tx`, arrays, or user-defined values bound with `%`), `drop` consumes it exactly once.
For non-linear values (`i32`, `f64`, `string`, etc.), `drop` is allowed and simply discards the value.
If a user-defined constructor takes a linear argument (for example `[| T |]`), constructed values of that ADT are also treated as linear.

```nexus
let %tx = db.begin_tx()
perform db.commit(tx: %tx) // tx is consumed
```

### Borrowing

The `borrow` keyword allows temporary, immutable access to a linear value without consuming it.

```nexus
fn peek(x: &i64) -> unit do ... endfn

let %x = 10
let x_ref = borrow %x
perform peek(x: x_ref) // %x is NOT consumed
drop %x          // %x is consumed here
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

The `perform` keyword is mandatory for effectful calls and forbidden for pure calls.

```nexus
perform g()
```

## Function Values and Closures

Functions are first-class values, including inline lambda literals.

```nexus
let f = fn (x: i64) -> i64 do
  return x + 1
endfn
```

Closure rules:

- Closures cannot capture mutable references (`Ref<T>`).
- If a closure captures a linear value, the closure itself becomes linear (`%((...) -> ...)` conceptually).
- A recursive local lambda requires an immutable `let` binding and an explicit type annotation.
- Linearity weakening at call sites is supported: a plain `T` can be passed to a parameter of type `%T`.
