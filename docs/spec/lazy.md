---
layout: default
title: Lazy Evaluation and Concurrency
---

# Lazy Evaluation and Concurrency

Lazy thunks (`@`) and `conc` blocks share the same foundation: **deferred computation**. A lazy binding suspends an expression as a zero-argument closure; a `conc` task suspends a block of statements as a thread-spawnable closure. Both use closure conversion to capture free variables, and both are subject to linearity constraints.

## Lazy Thunks (`@`)

### Syntax

```nexus
let @x: string = expensive_computation()  // creates thunk, NOT evaluated
let result = @x                            // forces thunk, evaluates now
```

### Desugaring

`let @x: T = expr` desugars to:

```nexus
let x = fn () -> T do return expr end
```

`@x` (force) desugars to calling the thunk: `x()`.

Type annotation is recommended. Without it, the thunk return type defaults to `i64`.

### Lazy Type (`@T`)

The type `@T` represents a suspended computation producing `T` when forced:

```nexus
let delayed: @string = @("hello" ++ " world")
```

### Lazy Parameters

Function parameters can use the `@` sigil for call-by-need argument passing:

```nexus
let log = fn (level: i64, @msg: string) -> unit do
  if level > 0 then Console.println(val: @msg) end
end
```

The caller's argument expression is wrapped in a thunk automatically. The function body forces it with `@msg` only when needed.

**Note**: The nxc self-hosting compiler does not yet support `@` in call-site argument labels.

## Concurrency (`conc`)

### Syntax

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

All tasks run concurrently on OS-level threads. The `conc` block waits for all tasks to complete.

### Task Semantics

- Each task runs in its own thread with **isolated linear memory**
- Free variables from the enclosing scope are captured and serialized to the task's heap
- Mutable (`~`) bindings cannot be captured (gravity rule)
- Linear (`%`) bindings transfer ownership into the task
- Tasks share the same WASI capabilities as the parent

### Variable Capture

Tasks capture free variables by value. The compiler performs closure conversion to serialize captured values across thread boundaries:

```nexus
let name = "world"
conc do
  task greet do
    // `name` is captured by value — copied to the task's heap
    Console.println(val: "hello " ++ name)
  end
end
```

Mutable references cannot cross task boundaries because they violate thread isolation:

```nexus
let ~counter = 0
conc do
  task inc do
    // ERROR: cannot capture ~counter in conc task
    ~counter <- ~counter + 1
  end
end
```

### Exception Propagation

Exceptions raised inside a `conc` task propagate to the join point. If multiple tasks raise, the first exception is propagated. [Exception groups](../exception-groups) work across task boundaries:

```nexus
try
  conc do
    task risky do
      raise NotFound(path: "/tmp/missing")
    end
  end
catch
  case IOError ->
    // catches NotFound from the task
    Console.eprintln(val: "IO error in task")
end
```

## Shared Foundation: Closures and Linearity

Lazy thunks and conc tasks are both closures under the hood. The same linearity rules apply:

| Constraint | Lazy thunk (`@`) | Conc task |
|---|---|---|
| Capture immutable | yes (by value) | yes (serialized) |
| Capture mutable (`~`) | yes (but makes thunk linear) | no (gravity rule) |
| Capture linear (`%`) | yes (transfers ownership) | yes (transfers ownership) |
| Must consume | exactly once, on every path | always (join is implicit) |
| Cross-thread sharing | no (linear closure) | n/a (each task is independent) |

A lazy thunk must be forced on every execution path — conditional forcing violates linearity:

```nexus
// OK: thunk always forced
let @msg: string = "result: " ++ from_i64(val: n)
Console.println(val: @msg)

// ERROR: thunk not consumed when verbose is false
let @msg: string = "result: " ++ from_i64(val: n)
if verbose then Console.println(val: @msg) end
```

Lazy thunks can be created inside task bodies for deferred initialization, but cannot be shared across task boundaries — serializing a closure would break linearity guarantees:

```nexus
conc do
  task process do
    let @config: string = load_config()  // OK: created inside task
    run_with(config: @config)
  end
end
```

---

See also: [Exception Groups](../exception-groups), [Types](../types), [Syntax](../syntax)
