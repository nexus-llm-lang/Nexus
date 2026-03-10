---
layout: default
title: Types
---

# Types

Nexus uses a strict type system with Hindley-Milner inference, structural records, algebraic data types, and linear types for resource tracking.

## Primitive Types

| Type | Description |
|---|---|
| `i32` | 32-bit signed integer |
| `i64` | 64-bit signed integer |
| `f32` | 32-bit floating-point |
| `f64` | 64-bit floating-point |
| `float` | Alias of `f64` |
| `bool` | Boolean (`true` / `false`) |
| `string` | Immutable UTF-8 string |
| `unit` | The unit type, written `()` as a value |

Numeric literals default to `i64` (integers) and `f64` (floats) unless constrained by type annotations.

## Compound Types

### Records

Structural record types. Defined with `type` or used inline:

```nexus
pub type User = { id: i64, name: string }

let u = { id: 1, name: "Alice" }
```

### ADTs (Sum Types)

Algebraic data types with labeled constructor arguments:

```nexus
pub type Result<T, E> = Ok(val: T) | Err(err: E)
pub type Option<T> = Some(val: T) | None()
```

Constructors with no fields still require `()` in patterns and expressions (e.g., `None()`).

### Lists

Immutable singly-linked lists:

```nexus
let xs: [i64] = [1, 2, 3]
```

Lists cannot contain mutable references.

### Arrays

Linear mutable arrays. Arrays are inherently linear to ensure unique ownership:

```nexus
let %arr: [| i64 |] = [| 1, 2, 3 |]
%arr[0] <- 42
let val = (&%arr)[0]
```

Arrays cannot contain mutable references.

### Generics

User-defined types can be parameterized:

```nexus
pub type Pair<A, B> = Pair(left: A, right: B)
```

## Linear Types (`%`)

Linear types make resource lifecycle visible in syntax (see [Design](../../design#linear-types-as-literal-resource-tracking)). The `%` sigil marks a binding that must be consumed **exactly once**.

### Rules

| Rule | Enforcement |
|---|---|
| Must consume exactly once | Compile error if unused or used twice |
| Cannot discard with `_` | Wildcard on composite linear value is rejected |
| Branch consistency | All branches must consume the same linear bindings |
| Closure capture | Capturing `%x` makes the closure itself linear |
| No mutable ref | `~` binding cannot hold a linear type |

### Primitive Auto-Drop

Primitive linear values (`i64`, `f64`, `bool`, `string`, `unit`) are automatically released at scope end. Using `%` on primitives is valid but unnecessary.

### Composite Consumption

Composite linear values (records, ADTs, arrays) must be explicitly consumed via:
- Function call (passing the linear binding as an argument)
- Pattern matching (destructuring extracts the value)
- Return (propagating ownership to the caller)

### Linearity Weakening

A plain `T` value can be passed to a function expecting `%T`. The value is treated as linear for the duration of the call.

### Linear Closures

If a closure captures a linear binding, the closure itself becomes linear and can only be invoked once:

```nexus
let %resource = acquire()
let f = fn () -> unit do
  consume(r: %resource)
  return ()
end
f()  // ok -- consumes the closure
f()  // error -- closure already consumed
```

## Borrowing (`&`)

The `&` sigil creates an immutable, non-consuming view of a value (see [Design](../../design#borrowing-as-explicit-aliasing)).

### Syntax

`&` works both as a prefix operator and as a let-binding sigil:

```nexus
let borrowed = &arr  // prefix operator on immutable binding
let &b = ~x          // let-binding sigil
let b2 = &%resource  // prefix operator on linear binding
```

### Coercion

`&T` coerces to `T` for reading. The original binding remains live and unconsumed.

### Patterns

Borrow patterns bind without consuming:

```nexus
fn peek(x: &i64) -> unit do
  // x is &i64, original not consumed
  return ()
end
```

### Properties

- Multiple borrows of the same binding are allowed simultaneously
- Borrow lifetime is tied to the scope of the source binding
- Borrowing a linear value does not consume it

## Mutable References (`~`)

The `~` sigil creates a stack-confined mutable binding:

```nexus
let ~count = 0
~count <- ~count + 1
```

### Gravity Rule

Mutable references cannot escape the defining function:

- Cannot be returned from functions
- Cannot be stored in heap-allocated structures (records, ADTs, lists)
- Cannot be captured by closures or concurrent tasks

This ensures mutation remains localized and predictable.

### Restrictions

- Cannot hold linear types (`~` on `%T` is forbidden)
- Cannot be captured by lambdas
- Cannot cross `conc` task boundaries

## Sigil Compatibility Matrix

| Operation | Immutable | `~` Mutable | `%` Linear | `&` Borrow |
|---|---|---|---|---|
| Read | yes | yes | yes (consumes) | yes |
| Assign (`<-`) | no | yes | no | no |
| Pass to function | yes (copy) | yes (copy) | yes (move) | yes (view) |
| Return from function | yes | no | yes | no |
| Store in record/ADT | yes | no | yes | yes |
| Capture in closure | yes | no | yes (makes closure linear) | yes |
| Borrow with `&` | yes | yes | yes | yes |
| Discard with `_` | yes | yes | no (composites) | yes |

## Function Types

Functions are first-class values with labeled parameters and optional throws/coeffect annotations:

```nexus
(label: T) -> R                                 // pure function
(a: i64, b: i64) -> i64                         // multiple params
() -> unit throws { Exn }                       // with throws
() -> string require { Net }                    // with coeffect
(x: T) -> R require { C | r } throws { E | e }  // open rows
```

### Closures

Lambdas capture immutable bindings from their lexical scope:

```nexus
let f = fn (x: i64) -> i64 do
  return x + 1
end
```

Closure constraints:
- Cannot capture mutable (`~`) bindings
- Capturing a linear (`%`) binding makes the closure linear
- A recursive local lambda requires an immutable `let` binding with an explicit type annotation

## Row Types

Effect and coeffect annotations use row types:

```nexus
{ Exn }          // single entry
{ Net, Fs }      // multiple entries
{ Console | e }  // open row with tail variable
```

Empty row (omitted or `{}`) means no effects/requirements.
