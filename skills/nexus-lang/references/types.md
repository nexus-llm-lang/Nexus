# Nexus Type System Reference

## Primitive Types

| Type | Size | Default | Notes |
|------|------|---------|-------|
| `i32` | 32-bit int | — | Requires explicit annotation |
| `i64` | 64-bit int | Yes | Integer literals default to this |
| `f32` | 32-bit float | — | Requires explicit annotation |
| `f64` / `float` | 64-bit float | Yes | Float literals default to this |
| `bool` | — | — | `true` / `false` |
| `char` | — | — | Single Unicode character (`'a'`, `'\n'`, `'\x41'`, `'\u{1F600}'`) |
| `string` | — | — | UTF-8, immutable |
| `unit` | — | — | `()` literal |

## Record Types (Structural)

```nexus
// Named record type
export type User = { id: i64, name: string, email: string }

// Construction
let u = { id: 1, name: "Alice", email: "a@b.com" }

// Field access
let n = u.name

// Inline anonymous records are supported
let pair = { x: 10, y: 20 }
```

## Algebraic Data Types (Sum Types)

```nexus
export type Option<T> = Some(val: T) | None
export type Result<T, E> = Ok(val: T) | Err(err: E)
export type List<T> = Nil | Cons(v: T, rest: List<T>)

// Constructors always use labeled fields (except nullary constructors)
let x = Some(val: 42)
let empty = None
let xs = Cons(v: 1, rest: Nil)
```

## Opaque Types

```nexus
// Constructors hidden from importers — only module can create/destructure
export opaque type Handle = Handle(id: i64)
export opaque type Set = Set(id: i64)
```

## List Type `[ T ]`

```nexus
// Immutable singly-linked list
let xs: [ i64 ] = [1, 2, 3]    // sugar for Cons(v:1, rest:Cons(v:2, rest:Cons(v:3, rest:Nil)))

// Pattern matching
match xs do
  case Nil -> return 0
  case Cons(v: head, rest: tail) -> return head + sum(xs: tail)
end
```

- Cannot contain mutable references
- `[ T ]` is the type, `Nil` / `Cons(v:, rest:)` are constructors

## Array Type `[| T |]` (Linear)

```nexus
// Mutable, heap-allocated, must be consumed
let %arr = [| 1, 2, 3 |]

// Read via borrow
let lock = &%arr
let v = lock[0]

// Mutate via borrow
lock[1] <- 42

// Pass to function (consuming)
consume(arr: %arr)
```

- Arrays are **linear** — the `%` sigil is mandatory
- Created with `[| ... |]` syntax
- Mutation requires borrowing first

## Linear Types (`%`)

Linear bindings must be consumed exactly once. Consumption means:
- Passed to a function as an argument
- Destructured via pattern match
- Returned from the function

```nexus
let %handle = open_file(path: "data.txt")
// ... use handle ...
close(handle: %handle)    // consumed here — REQUIRED

// ILLEGAL: unused linear binding
let %h = open_file(path: "x.txt")
return ()  // ERROR: %h not consumed

// ILLEGAL: wildcard on composite linear value
let %h = open_file(path: "x.txt")
let _ = %h  // ERROR: cannot discard linear value with wildcard
```

**Exception**: Primitive linear values (`i64`, `f64`, `bool`, `string`, `unit`) auto-drop at scope end.

## Borrowing (`&`)

```nexus
let %arr = [| 1, 2, 3 |]
let view = &%arr           // immutable borrow — does NOT consume %arr
let len = length(arr: view)
consume(arr: %arr)         // %arr still available to consume

// Multiple simultaneous borrows are allowed
let v1 = &%arr
let v2 = &%arr             // OK: multiple immutable borrows
```

- `&` creates an immutable, non-consuming view
- Borrow does NOT count as consumption
- Multiple borrows allowed simultaneously

## Mutable References (`~`)

```nexus
let ~counter = 0
~counter <- ~counter + 1     // reassign
let val = ~counter           // dereference

// Gravity Rule — mutable refs cannot:
// - escape the defining function (no return, no capture in closures)
// - hold linear types
// - cross concurrency boundaries (no capture in conc tasks)
```

## Function Types

```nexus
// Basic
(a: i64, b: i64) -> i64

// With effects
(path: string) -> string require { PermFs } throws { Exn }

// Higher-order
(f: (val: i64) -> i64, x: i64) -> i64

// Unit return
() -> unit

// Generic
<T>(val: T) -> Option<T>
```

## Generics

```nexus
// Generic type definition
export type Pair<A, B> = Pair(left: A, right: B)

// Generic function
export let map = fn <T, U>(opt: Option<T>, f: (val: T) -> U) -> Option<U> do
  match opt do
    case Some(val: v) ->
      let mapped = f(val: v)
      return Some(val: mapped)
    case None -> return None
  end
end

// Type parameters use UIDENT (uppercase start)
// Instantiation is implicit (inferred at call sites)
```

## Exception Types

```nexus
// Declare custom exceptions
export exception NotFound(msg: string)
export exception Timeout(ms: i64)

// Built-in root type: Exn
// All exceptions extend Exn and can be caught as Exn

// Functions that may throw must declare it
let risky = fn () -> i64 throws { Exn } do
  raise NotFound(msg: "oops")
end
```

## Type Coercions

| From | To | When |
|------|----|------|
| `%T` | `T` | Passing linear to non-linear param (consumes) |
| `T` | `&T` | Automatic for read-only params |
| `bool` | `i32` | WASM level (0=false, 1=true) |

## Pattern Matching Types

```nexus
// Constructor patterns — must match all fields by label
case Ok(val: v) -> ...
case Err(err: e) -> ...

// Record patterns — partial match with trailing _
case { name: n, _ } -> ...     // ignores other fields

// Wildcard
case _ -> ...

// Literal patterns
case 0 -> ...
case true -> ...
case "hello" -> ...

// Variable binding with sigil
case Handle(%h) -> ...        // binds h as linear
```
