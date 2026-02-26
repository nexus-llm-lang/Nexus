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
pub type User = {
  id: i64,
  name: string
}
```

### ADTs (Sum Types)

Custom algebraic data types are defined via `type` with variant alternatives.

```nexus
pub type Option<T> = Some(val: T) | None
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
let val = (&%arr)[0]
```

Arrays cannot contain mutable references (`Ref<T>`).

## Memory Management

### Linear Types

Linear types (prefixed with `%`) ensure resources (like database transactions) are never leaked or reused inappropriately.

- **Primitive** linear values (`i64`, `f64`, `bool`, `string`, `unit`) are automatically released at scope end. The `%` sigil on primitives is unnecessary and triggers a compiler warning.
- **Composite** linear values (Records, ADTs, arrays) must be explicitly consumed via pattern matching or function calls.
- If a user-defined constructor takes a linear argument (for example `[| T |]`), constructed values of that ADT are also treated as linear.

```nexus
let %tx = db.begin_tx()
db.commit(tx: %tx) // tx is consumed
```

### Borrowing

The `&` keyword allows temporary, immutable access to a linear value without consuming it.

```nexus
fn peek(x: &i64) -> unit do ... endfn

let %x = 10
let x_ref = &%x
peek(x: x_ref) // %x is NOT consumed
// %x is auto-dropped at scope end (primitive)
```

### Mutable References

Mutable stack references use `~` and `ref(T)` type. They are restricted by the "Gravity Rule" (cannot be stored in immutable variables or returned from functions).

```nexus
let ~count = 1
~count <- ~count + 1
```

## Effect Types

Function signatures are split into:

- `require { ... }` rows for coeffects (ports)
- `effect { ... }` rows for builtin effects

Both use row-polymorphic forms like `{ E1, E2 | r }`.

```nexus
fn g() -> unit require { Net } effect { Console } do ... endfn
```

Calls use normal function-call syntax; compatibility with `require`/`effect` is checked statically.

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
