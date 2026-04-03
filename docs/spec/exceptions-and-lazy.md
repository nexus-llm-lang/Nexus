---
layout: default
title: Exception Groups and Lazy Evaluation
---

# Exception Groups and Lazy Evaluation

Two features that complement the base exception/coeffect system: **exception groups** for hierarchical error handling, and **lazy thunks** (`@`) for deferred computation.

## Exception Groups

Exception groups define a named set of exception types. A `catch` arm matching a group name expands to match all members.

### Declaration

```nexus
exception group Name = A | B | C
```

The `export` modifier makes the group visible to importers:

```nexus
export exception group ParseError = UnexpectedToken | MissingMain
```

### Structured Exceptions

Individual exceptions carry typed fields, not pre-formatted strings. Combined with groups, this gives both fine-grained and coarse-grained error handling:

```nexus
exception UnexpectedToken(expected: string, got: string, span: Span)
exception MissingMain
exception group ParseError = UnexpectedToken | MissingMain
```

### Catching Groups

Catching a group name expands to match all members:

```nexus
try
  parse(tokens: tokens)
catch
  case ParseError ->
    // catches both UnexpectedToken and MissingMain
    Console.eprintln(val: format_error(err: e))
end
```

### Catching Specific Exceptions

Fine-grained matching destructures individual exception fields:

```nexus
try
  compile(src: src)
catch
  case UnexpectedToken(expected: e, got: g, span: sp) ->
    Console.eprintln(val: "expected " ++ e ++ ", got " ++ g)
  case TypeMismatch(func_name: f, detail: d, span: sp) ->
    Console.eprintln(val: "in " ++ f ++ ": " ++ d)
  case CompileError ->
    Console.eprintln(val: "other compile error")
end
```

### Nested Groups

Groups can reference other groups' members, but groups are expanded at catch sites, not hierarchically nested. That is, a group is always a flat set of exception constructors resolved at the point of use.

### Multi-Arm Catch

The `catch` block supports multiple `case` arms with pattern matching, replacing the legacy single-parameter `catch ident ->` form. Each arm matches a specific exception constructor or group name:

```nexus
try
  operation()
catch
  case NotFound(msg: m) -> handle_missing(m: m)
  case PermDenied(msg: m) -> handle_perm(m: m)
  case IOError -> handle_generic()
end
```

## Lazy Evaluation (`@`)

### The `@` Sigil

The `@` sigil marks a binding as lazy. The bound expression is wrapped in a zero-argument thunk and evaluated only when forced.

### Syntax

```nexus
let @x: string = expensive_computation()  // creates thunk, NOT evaluated
let result = @x                            // forces thunk, evaluates now
```

### How It Works

- `let @x = expr` desugars to `let x = fn () -> T do return expr end`
- `@x` (force) calls the thunk: equivalent to `x()`
- Type annotation recommended: `let @x: T = ...` (default return type is i64 without annotation)

### Lazy Type (`@T`)

The type `@T` represents a suspended computation that produces `T` when forced.

```nexus
let delayed: @string = @("hello" ++ " world")
```

### Linearity Constraint

Lazy thunks are linear values (closures). They must be consumed (forced) exactly once on every execution path. Conditional forcing violates linearity:

```nexus
let @msg: string = "computed: " ++ from_i64(val: n)
// ERROR: if verbose is false, thunk is never consumed
if verbose then Console.eprintln(val: @msg) end
```

This means lazy thunks are best suited for unconditionally-evaluated deferred computation, not for conditionally-skipped work.

### Lazy Parameters

Function parameters can use the `@` sigil for call-by-need semantics:

```nexus
let log = fn (level: i64, @msg: string) -> unit do
  if level > 0 then Console.println(val: @msg) end
end
```

**Note**: Lazy parameter syntax (`@param: T`) requires the caller to provide a lazy-compatible expression. The nxc self-hosting compiler does not yet support `@` in call-site argument labels.

## Concurrency and Parallelism

### Conc Blocks

The `conc` block executes tasks in parallel using OS-level threads:

```nexus
conc do
  task fetch_data do
    let data = Net.get(url: api_url)
  end
  task write_log do
    Logger.info(msg: "fetching started")
  end
end
```

All tasks run concurrently and the block waits for all to complete.

### Task Semantics

- Each task runs in its own thread with isolated linear memory
- Free variables from the enclosing scope are captured and serialized
- Mutable (`~`) bindings cannot be captured (gravity rule)
- Linear (`%`) bindings transfer ownership into the task
- Tasks share the same WASI capabilities as the parent

### Interaction with Exceptions

Exceptions raised inside a `conc` task propagate to the join point. If multiple tasks raise, the first exception is propagated.

### Interaction with Lazy

Lazy thunks can be used inside tasks for deferred initialization:

```nexus
conc do
  task process do
    let @config: string = load_config()
    // config loaded only when first accessed
    run_with(config: @config)
  end
end
```

However, lazy thunks cannot be shared across task boundaries (they are linear closures and cannot be serialized).

---

See also: [Checked Exceptions and Coeffects](../effects), [Types](../types), [Syntax](../syntax)
