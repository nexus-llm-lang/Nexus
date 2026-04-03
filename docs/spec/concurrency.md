---
layout: default
title: Concurrency
---

# Concurrency (`conc`)

The `conc` block executes tasks in parallel using OS-level threads. All tasks run concurrently and the block waits for all to complete.

## Syntax

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

Task names are identifiers scoped to the `conc` block.

## Task Semantics

- Each task runs in its own thread with **isolated linear memory**
- Free variables from the enclosing scope are captured and serialized to the task's heap
- Mutable (`~`) bindings cannot be captured (gravity rule)
- Linear (`%`) bindings transfer ownership into the task
- Tasks share the same WASI capabilities as the parent

## Variable Capture

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

## Exception Propagation

Exceptions raised inside a `conc` task propagate to the join point. If multiple tasks raise, the first exception is propagated. Exception groups work across task boundaries:

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

## Lazy Thunks in Tasks

Lazy thunks (`@`) can be used inside task bodies for deferred initialization:

```nexus
conc do
  task process do
    let @config: string = load_config()
    run_with(config: @config)
  end
end
```

However, lazy thunks **cannot be shared across task boundaries**. They are linear closures that capture references to the parent's memory -- serializing them would break linearity guarantees. Create lazy thunks inside the task, not outside.

---

See also: [Lazy Evaluation](../lazy), [Exception Groups](../exception-groups), [Syntax](../syntax)
