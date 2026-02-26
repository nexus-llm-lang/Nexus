# Memory Management

Nexus manages resources through a substructural type system combining **linear types**, **borrowing**, **automatic resource management**, and **stack-confined mutable references**. There is no garbage collector. Ownership and lifetime constraints are enforced entirely at compile time by the typechecker.

## Design Principles

1. **Automatic primitive management, explicit composite consumption.** Primitive linear values are auto-dropped at scope end. Composite types (Records, ADTs) must be consumed via pattern matching. The compiler rejects programs that leak or double-use resources.
2. **No aliasing of mutable state.** Mutable references (`~`) are confined to the stack frame that creates them and cannot escape.
3. **Borrowing is zero-cost at the type level.** A `&` produces an immutable view (`&T`) without transferring ownership, allowing repeated reads of a linear value.

## Linear Types (`%`)

A binding prefixed with `%` is **linear**: it must be consumed exactly once along every control-flow path.

### Declaration

```nexus
let %handle = acquire_resource()
let %arr: [| i64 |] = [| 1, 2, 3 |]
```

When a value is bound with `%`, the typechecker wraps its type in `%T` (internally `Linear(T)`) and begins tracking it. Arrays (`[| T |]`) are inherently linear regardless of sigil because they require unique ownership for mutation.

### Consumption

A linear binding is consumed when it appears as:

- A function argument: `release(handle: %handle)`
- A return value: `return %handle`
- A match target: `match %handle do case _ -> () endmatch`
- **Auto-drop at scope end** (primitives only)

After consumption, any further use is a compile-time error:

```nexus
let %x = acquire()
release(r: %x)
release(r: %x)   // Error: linear variable '%x' already consumed
```

> **Note:** Primitive types (`i64`, `f64`, `bool`, `string`, `unit`) bound with `%` are automatically released when their scope ends. The compiler emits a warning that the `%` sigil is unnecessary for these types.

### Enforcement Rules

| Rule | Description |
|---|---|
| Exactly-once | Every non-primitive linear binding must be consumed exactly once before the end of its scope. Primitives are auto-dropped. |
| No wildcard discard | Matching a non-primitive linear value with `_` is forbidden. Use a named binding to destructure it. |
| Branch consistency | Both branches of `if`/`else` and all arms of `match` must leave the same set of linear variables consumed. |
| No array extraction | Indexing a linear array (`%arr[i]`) to move out an element is forbidden. Use `&` for read access. |

### Linear Parameters

Function parameters can be declared linear:

```nexus
let consume = fn (%x: Handle) -> unit do
  match x do case _ -> () endmatch
  return ()
endfn
```

**Linearity weakening** allows passing a plain (non-linear) value to a linear parameter:

```nexus
consume(x: 10)   // OK: 10 is promoted to %i64 for the call
```

The reverse is not allowed: a linear value cannot be passed where a non-linear parameter is expected (it would violate the consumption guarantee).

### Linear Closures

If a lambda captures a linear binding from its enclosing scope, the closure itself becomes linear:

```nexus
let %resource = acquire()
let f = fn () -> unit do
  release(r: %resource)   // captures %resource
  return ()
endfn
// f is now %(() -> unit) -- must be called exactly once
f()
```

The captured linear binding is consumed in the outer scope at the point the lambda is created.

## Borrowing (`&`)

The `&` expression creates a temporary, immutable view of a binding **without consuming** it.

### Syntax

```nexus
&%x        // &a linear binding
&~y        // &a mutable binding
&z         // &an immutable binding
```

### Type

`&` produces `&T` (internally `Borrow(T)`). If the target is `%T` or `&T`, the inner type is unwrapped first, so the result is always `&T` where `T` is the base type.

### Semantics

- **Does not consume** the target. The linear tracking set is unchanged.
- **Coerces to the base type.** `&T` unifies with `T`, so a borrowed value can be passed to functions expecting the underlying type.
- **Multiple borrows are allowed.** A linear value can be borrowed any number of times before it is consumed.

### Typical Pattern: Read-then-consume

```nexus
let %arr = [| 10, 20, 30 |]
let first = (&%arr)[0]     // read without consuming
let second = (&%arr)[1]    // &again
match %arr do case _ -> () endmatch  // consume the array
```

### Consume-and-return Pattern

For port-based APIs where handlers must be stateless (no borrow), a linear resource is consumed and returned alongside the result in a record:

```nexus
fn read(handle: %Handle) -> { content: string, handle: %Handle }
```

The caller destructures the record to get both the result and the new handle:

```nexus
let %h = Fs.open_read(path: [=[data.txt]=])
let %r = Fs.read(handle: %h)
match %r do case { content: content, handle: %h2 } ->
  Fs.close(handle: %h2)
  // use content
endmatch
```

This pattern enables mock handlers that can construct handle values freely (the handle type is non-opaque) while preserving linear resource safety.

### Borrow in Function Signatures

Functions that only need read access should accept `&T`:

```nexus
let sum = fn (xs: &[| i64 |]) -> i64 do
  // xs is borrowed -- caller retains ownership
  return xs[0] + xs[1]
endfn

let %data = [| 3, 7 |]
let total = sum(xs: &%data)
match %data do case _ -> () endmatch
```

## Automatic Resource Management

Nexus does not have an explicit `drop` statement. Resource cleanup is handled automatically based on type:

### Primitive Auto-drop

Primitive types (`i64`, `f64`, `bool`, `string`, `unit`) are automatically released when their scope ends. Using the `%` sigil on primitives is permitted but triggers a compiler warning since it is unnecessary.

```nexus
let msg = [=[hello]=]
print(val: msg)
// msg is automatically cleaned up at scope end
```

### Composite Consumption via Pattern Matching

Records and ADTs must be consumed through pattern matching. Destructured fields follow the same rules recursively.

```nexus
let %handle = acquire()
match %handle do
  case Resource { id: id } ->
    // id is a primitive: auto-dropped at scope end
    process(id: id)
endmatch
// %handle is consumed by the match
```

### Consumption and Control Flow

Linear bindings must be consumed on **all** paths. The typechecker ensures consumption is consistent across branches:

```nexus
let %x = acquire()
if condition then
  release(r: %x)    // consumed in then-branch
else
  release(r: %x)    // must also be consumed in else-branch
endif
```

Omitting consumption in either branch is a compile-time error ("linear mismatch").

## Mutable References (`~` / `ref(T)`)

Mutable bindings use the `~` sigil. At the type level, `let ~x = v` wraps the value in `ref(T)`.

### Declaration and Assignment

```nexus
let ~count: i64 = 0
~count <- ~count + 1
~count <- ~count * 2
```

Reading `~count` auto-dereferences the reference, returning the inner value. Assignment with `<-` updates the inner value in place.

### The Gravity Rule

Mutable references are **second-class**: they are confined to the stack frame that creates them. The typechecker enforces:

| Restriction | Rationale |
|---|---|
| Cannot return `ref(T)` from a function | Prevents dangling references |
| Cannot store in an immutable binding | `let x = ~y` is forbidden (use `let ~x = ...` instead) |
| Cannot capture in a closure | Prevents mutable aliasing across scopes |
| Cannot store in arrays or lists | Prevents heap-escaping references |
| Cannot hold linear types | `let ~x: %T = ...` is forbidden (prevents aliasing of linear resources) |

These restrictions ensure that mutation is always **local**, **predictable**, and **invisible** to callers.

### Array Mutation via Index Assignment

Arrays support element-level mutation through index assignment:

```nexus
let %arr = [| 1, 2, 3 |]
%arr[0] <- 42
let val = (&%arr)[0]   // val == 42
match %arr do case _ -> () endmatch
```

Note that the array itself is linear (`%`), not mutable (`~`). Index assignment is a special form that mutates the array in-place without requiring a mutable reference wrapper.

## Interaction Summary

### Sigil Compatibility Matrix

| Operation | Immutable (`let x`) | Mutable (`let ~x`) | Linear (`let %x`) |
|---|---|---|---|
| Read | `x` | `~x` (auto-deref) | `%x` (consumes) |
| Borrow | `&x` | `&~x` | `&%x` (does not consume) |
| Assign | forbidden | `~x <- val` | `%arr[i] <- val` (index only) |
| Cleanup | auto (scope end) | auto (scope end) | auto (primitives) / match (composites) |
| Return | `return x` | forbidden | `return %x` (consumes) |
| Capture in closure | allowed | forbidden | allowed (closure becomes linear) |
| Pass to `conc` task | allowed | forbidden | forbidden (use borrow) |

### Type Constructors

| Syntax | Internal | Meaning |
|---|---|---|
| `%T` | `Linear(T)` | Must be consumed exactly once |
| `&T` | `Borrow(T)` | Immutable borrowed view |
| `ref(T)` | `Ref(T)` | Stack-confined mutable cell |
| `[| T |]` | `Array(T)` | Mutable, inherently linear array |
