# Nexus Idiomatic Code Patterns

## Program Structure

Every executable Nexus program needs a `main` function:

```nexus
// Minimal program
let main = fn () -> unit do
  return ()
end

// With I/O
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello!")
  end
  return ()
end

// Implicit return unit
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello!")
  end
  // no need `return ()`
end
```

## Import Patterns

```nexus
// Import cap + module alias (most common for I/O)
import { Console }, * as stdio from "std:stdio"

// Import specific items
import { Option, Some, None } from "std:option"
import { Result, Ok, Err } from "std:result"

// Import as module alias (for utility functions)
import * as list from "std:list"
import * as str from "std:str"
import * as math from "std:math"

// Combine both
import { Net, Request, Response }, * as net_mod from "std:network"
```

## Custom Cap + Handler (Dependency Injection)

```nexus
// 1. Define domain types
type User = { id: i64, name: string, email: string }

// 2. Define cap (interface)
cap UserRepository do
  fn find_by_id(id: i64) -> Option<User>
  fn save(user: User) -> Result<unit, string>
end

// 3. Business logic depends on cap
let register = fn (name: string, email: string) -> Result<unit, string> require { UserRepository, Logger } do
  let user = { id: 0, name: name, email: email }
  Logger.info(msg: "Registering: " ++ email)
  return UserRepository.save(user: user)
end

// 4. Handler for production
let db_repo = handler UserRepository require { PermFs } do
  fn find_by_id(id: i64) -> Option<User> do
    // ... real implementation
    return None
  end
  fn save(user: User) -> Result<unit, string> do
    // ... real implementation
    return Ok(val: ())
  end
end

// 5. Handler for testing
let mock_repo = handler UserRepository do
  fn find_by_id(id: i64) -> Option<User> do
    return Some(val: { id: id, name: "Test", email: "test@test.com" })
  end
  fn save(user: User) -> Result<unit, string> do
    return Ok(val: ())
  end
end
```

## Error Handling Patterns

### Result-based (prefer for recoverable errors)

```nexus
import { Result, Ok, Err } from "std:result"

let parse_config = fn (raw: string) -> Result<Config, string> do
  if str.length(s: raw) == 0 then
    return Err(err: "empty config")
  else
    // ... parse
    return Ok(val: config)
  end
end
```

### Exception-based (for unrecoverable or I/O errors)

```nexus
exception ConfigError(msg: string)

let load_or_die = fn (path: string) -> Config require { Fs } throws { Exn } do
  let raw = Fs.read_to_string(path: path)
  let res = parse_config(raw: raw)
  match res do
    | Ok(val: c) -> return c
    | Err(err: msg) -> raise ConfigError(msg: msg)
  end
end

// Selective catch in main
let main = fn () -> unit require { PermFs, PermConsole } do
  inject fs_mod.system_handler, stdio.system_handler do
    try
      let cfg = load_or_die(path: "config.txt")
      Console.println(val: "Config loaded")
    catch
      | ConfigError(msg: m) -> Console.println(val: "Config error: " ++ m)
      | _ -> Console.println(val: "Unknown error")
    end
  end
end
```

## Bool Dispatch vs ADT Destructuring

**`if` is for bool dispatch. `match` is for ADT destructuring.**
Reading `match` should signal "destructuring an algebraic data type" — when every
reader can rely on that, large pattern-match trees scan faster.

### Good

```nexus
// bool: use if/else
if is_empty then
  return Err(err: "empty")
else
  return Ok(val: parse(input: raw))
end

// ADT: use match (the value carries variants/payloads)
match res do
  | Ok(val: c)  -> return c
  | Err(err: m) -> raise ConfigError(msg: m)
end
```

### Bad

```nexus
// Bool dispatch dressed up as ADT destructuring.
// The reader has to scan two arms to discover this is just `if flag`.
match flag do
  | true  -> do_a()
  | false -> do_b()
end
```

### Exception: guard-style single arm

When you genuinely want "do this only when one shape matches, otherwise fall
through / raise", a single-arm `match` (or `match` + wildcard) reads cleaner
than nested `if`. Keep these.

```nexus
// OK: one meaningful arm + wildcard
match parse(input: raw) do
  | Ok(val: c) -> use(config: c)
  | _          -> ()
end
```

### Why

- `match X do | true -> A | false -> B end` is 4 lines for what `if X then A else B end` says in 1.
- `match` is the language's most expensive read — every site costs the reader a "destructuring or just dispatch?" check. Reserving it for ADTs makes that check trivial.
- The pattern compiler emits extra match-tree code where a plain branch would suffice.

## Linear Resource Management

Nexus linear types (`%`) enforce exactly-once consumption at compile time.

```nexus
// File handle: open → use → close (compiler enforces the chain)
let process_file = fn (path: string) -> string require { Fs } throws { Exn } do
  let %handle = Fs.open_read(path: path)
  let { content: content, handle: %h } = Fs.read(handle: %handle)
  Fs.close(handle: %h)
  return content
end

// HashMap: create → use → free
let count_words = fn (words: [ string ]) -> unit do
  let %map = smap.empty()
  // ... populate map ...
  let keys = smap.keys(m: &%map)
  smap.free(m: %map)    // must explicitly free
end
```

## Array Patterns

Arrays are linear (`%`). Borrow (`&`) for reads/writes, consume when done.

```nexus
// Create (linear) and borrow (view)
let %arr = [| 0, 0, 0 |]
let view = &%arr
view[0] <- 10
view[1] <- 20
view[2] <- 30

// Read
let first = view[0]
let len = array.length(arr: &%arr)

// Iterate via borrow
array.for_each(arr: &%arr, f: fn (val: i64) -> unit do
  Console.println(val: str.from_i64(val: val))
end)

// Consume (f receives each element as linear)
array.consume(arr: %arr, f: fn (val: %i64) -> unit do end)
```

## Lazy Evaluation & Parallel Execution

The `@` sigil marks lazy bindings. A lazy binding defers evaluation until forced.
Forcing (`@expr`) triggers parallel execution: the thunk is spawned as an independent
task and joined when the result is needed.

```nexus
// Lazy binding: RHS is NOT evaluated here
let @expensive = heavy_computation(input: data)

// Force: spawns thunk, blocks until result ready
let result = @expensive

// Lazy with captured variables
let base = 100
let @derived = base * 2 + some_call(n: base)
let val = @derived    // captures `base`, evaluates in parallel

// Inline force on expression (no binding needed)
let quick = @(x + y)
```

### Type: `@T`

`@T` is a lazy thunk that, when forced, produces a value of type `T`.

```nexus
// Type annotation
let compute = fn (input: @i64) -> i64 do
  return @input + 1
end
```

### Runtime model

- `let @x = expr` compiles to a `LazySpawn` — the expression is packaged as a
  thunk with captured free variables and submitted to `nexus:runtime/lazy` for
  parallel execution
- `@x` compiles to a `LazyJoin` — blocks the current thread until the thunk
  completes and returns the result
- Thunks that have no side effects can safely run in parallel with the main thread

