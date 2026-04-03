---
layout: default
title: Lazy Evaluation
---

# Lazy Evaluation (`@`)

The `@` sigil introduces call-by-need semantics. A lazy binding wraps its expression in a zero-argument thunk — a closure that is evaluated only when forced. Execution remains sequential; `@` controls *when* a computation runs, not *how* it's scheduled.

## Syntax

```nexus
let @x: string = expensive_computation()  // creates thunk, NOT evaluated
let result = @x                            // forces thunk, evaluates now
```

## Desugaring

`let @x: T = expr` desugars to:

```nexus
let x = fn () -> T do return expr end
```

`@x` (force) desugars to calling the thunk: `x()`.

Type annotation is recommended. Without it, the thunk return type defaults to `i64`.

## Lazy Type (`@T`)

The type `@T` represents a suspended computation producing `T` when forced:

```nexus
let delayed: @string = @("hello" ++ " world")
```

## Lazy Parameters

Function parameters can use the `@` sigil for call-by-need argument passing:

```nexus
let log = fn (level: i64, @msg: string) -> unit do
  if level > 0 then Console.println(val: @msg) end
end
```

The caller's argument expression is wrapped in a thunk automatically. The function body forces it with `@msg` only when needed.

**Note**: The nxc self-hosting compiler does not yet support `@` in call-site argument labels.

## Linearity Constraint

Lazy thunks are closures. Closures that capture variables are subject to linearity rules — a thunk must be consumed (forced) on **every** execution path:

```nexus
// OK: thunk always forced
let @msg: string = "result: " ++ from_i64(val: n)
Console.println(val: @msg)

// ERROR: thunk not consumed when verbose is false
let @msg: string = "result: " ++ from_i64(val: n)
if verbose then Console.println(val: @msg) end
```

Lazy thunks work for unconditionally-evaluated deferred computation. For conditionally-skipped work, use plain `if/then` guards instead.

---

See also: [Types](../types), [Syntax](../syntax)
