# Semantics

This document describes the execution model and behavioral rules of the Nexus language.

## Evaluation Strategy

Nexus is a **call-by-value** language. All expressions are fully evaluated to values before being passed to functions or used in constructors.

### Evaluation Order
Nexus follows a strict **left-to-right** evaluation order:
- **Arguments:** In a function call `f(a: e1, b: e2)`, `e1` is evaluated before `e2`.
- **Binary Operators:** In `e1 + e2`, `e1` is evaluated before `e2`.
- **Records/Constructors:** Fields and arguments are evaluated in the order they appear in the source code.

## Scoping

Nexus uses **lexical scoping**. Bindings are visible only within the block where they are defined and in nested blocks.

### Variable Shadowing
Shadowing is permitted. A `let` binding in an inner block can reuse a name from an outer block, masking the outer binding until the inner block ends.

## Sigil Semantics

Sigils are not just syntactic markers; they represent fundamental semantic constraints on how data is handled at runtime.

### Mutability (`~`)
- **Scope-Bound:** Mutable bindings are restricted to the stack of the function that defines them.
- **No Escape:** Mutable references cannot be returned from functions or stored in heap-allocated structures, ensuring mutation remains localized and predictable.
- **Assignment:** The `<-` operator updates the value of a mutable binding.
- **Concurrency:** Mutable references cannot be captured by concurrent tasks (`conc`) or asynchronous closures to prevent race conditions.

### Linearity (`%`)
- **Exactly Once (composites):** A non-primitive linear binding must be consumed exactly once via a function call, a return, or pattern matching.
- **Auto-drop (primitives):** Primitive linear values (`i64`, `f64`, `bool`, `string`, `unit`) are automatically released at scope end. Using `%` on primitives is valid but triggers a warning.
- **Static Enforcement:** The type system ensures linear resources (like file handles or sockets) are never leaked and never used after consumption.
- **No Discard:** The wildcard pattern `_` cannot be used to discard a non-primitive linear value. Every composite linear resource must be explicitly handled.
- **No Ref:** Mutable references to linear types (`Ref<%T>`) are strictly forbidden to prevent aliasing violations.
- **Weakening at Calls:** A plain value `T` can be passed to a function expecting a linear parameter `%T`. In this context, the value is treated as a linear resource for the duration of the call.

### Borrowing (`&`)
- **Immutable View:** The `&` sigil represents a **borrowed reference** that provides read-only access to a value.
- **Non-Consuming:** Borrowing does not consume the resource, allowing it to be used again later. This is essential for inspecting linear values (`%T`) without destroying them.
- **Generic Applicability:** While frequently seen with arrays (e.g., `&[| T |]`) to avoid copying large data, `&` is a general type constructor `&T` that can be applied to any type.
- **Temporary access:** A &is always temporary and its lifetime is tied to the scope of the binding it was created from.

## Closures and Captures

- **Lexical Captures:** Lambdas can capture immutable bindings from their lexical scope.
- **Mutability Restriction:** Closures cannot capture mutable bindings (`~`).
- **Linearity Propagation:** If a closure captures a linear value (`%`), the closure itself becomes linear and can only be invoked (consumed) once.

Nexus tracks two signature dimensions:

- **Effects (`effect`)** for builtin runtime actions (`Console`, `Exn`).
- **Coeffects (`require`)** for environment capabilities declared by `port`.

Handlers are values (`handler Port do ... endhandler`) and are introduced lexically with `inject ... do ... endinject`.
`try ... catch` discharges `Exn` from the protected region.

## Concurrency Model

### Structured Concurrency (`conc`)
- A `conc` block spawns multiple `task` units.
- The `conc` block is synchronous with respect to its caller: it blocks until **all** child tasks have completed.
- In the reference interpreter, tasks may execute sequentially, but the semantics allow for parallel execution.

## Exception Handling

- `raise` immediately terminates the current computation and unwinds the stack until it hits a `try...catch` block.
- The `Exn` value is passed to the `catch` parameter, and execution resumes in the catch block.
