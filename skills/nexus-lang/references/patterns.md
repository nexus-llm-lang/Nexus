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

## Multi-Handler Injection (single `inject`, comma-separated)

When a program needs more than one handler in scope, use the comma form
`inject h1, h2, h3 do BODY end` — not nested `inject h1 do inject h2 do ... end end`.

### Bad

```nexus
inject stdio.system_handler do
  inject console_logger do
    inject mock_repo do
      register_user(name: "Bob", email: "bob@example.com")
    end
  end
end
```

### Good

```nexus
inject stdio.system_handler, console_logger, mock_repo do
  register_user(name: "Bob", email: "bob@example.com")
end
```

### Why

- The parser accepts a comma-separated list of handler expressions in one
  `inject` (see `parse_inject_stmt` / `parse_inject_handlers` in
  `src/frontend/parser.nx`); order is not significant between independent
  handlers.
- The comma form keeps `BODY`'s indentation flat. Nested `inject` walls add an
  indentation level per handler with no extra meaning.
- Production code already favours this form: `main` in `src/driver.nx` injects
  four system handlers (`stdio`, `fs_mod`, `clock_mod`, `proc`) in a single
  statement.

### One handler depending on another's effects

A handler like `console_logger` that calls `Console.println` inside its arm
body does not need to be nested under `stdio.system_handler` — within a single
`inject` both handlers are in scope when `BODY` (and any handler arm body)
runs, so `Console.println` from inside `console_logger.info` resolves to
`stdio.system_handler` correctly.

### Exception: scope-shrinking

The only legitimate reason to nest is to keep one handler's scope narrower
than another's — e.g. injecting a mock repo only around a specific test step
while keeping logging available throughout. If both handlers want the same
scope, write one `inject`.

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

// Selective catch in main — pattern-match on exception constructors
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

// Bare catch — single identifier binds the whole exception
let run_or_log = fn () -> unit require { Console } do
  try
    do_work()
  catch err ->
    Console.eprintln(val: "<task>: " ++ format_error(err))
  end
end
```

### Catch form: bare vs selective

**Bare `catch <ident> -> body end`** binds the caught exception to one identifier and runs a single body — no `|` arms, no destructuring. Use when every escaping exception is handled the same way (log, rethrow, fall back to a default). `catch _ -> ...` is the same form with the value discarded. This is the dominant form in the in-tree codebase.

**Selective `catch | Pat -> ... | Pat -> ... end`** uses pipe-separated arms to pattern-match on exception constructors and payload fields. Use when arms need to branch on the exception variant or extract fields.

The two forms are mutually exclusive in a single `try` — pick one based on whether the handler discriminates on the exception or not.

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

## List Pattern Flattening (prefer nested cons over staircase `match`)

When a `match` arm immediately re-`match`es the tail it just bound, collapse the
two `match`es into one outer arm with a nested cons pattern. `::` chains in
patterns just like it does in expressions.

### Bad

```nexus
match xs do
  | [] -> handle_empty()
  | prog :: rest ->
    match rest do
      | [] -> handle_one(p: prog)
      | second :: rest2 -> handle_two(p: prog, s: second, t: rest2)
    end
end
```

### Good

```nexus
match xs do
  | [] -> handle_empty()
  | [prog] -> handle_one(p: prog)
  | prog :: second :: rest2 -> handle_two(p: prog, s: second, t: rest2)
end
```

The flattened form makes the arity each arm expects (zero / one / two-or-more
elements) visible at a glance instead of buried in a second `match`. The
single-element case uses the literal `[x]` pattern; longer prefixes use
`a :: b :: rest` (and so on) — `::` is right-associative in patterns just like
in expressions.

### Why

- The staircase form forces the reader to walk two indentation levels and
  reconstruct the prefix in their head. The flat form names the shape directly.
- The pattern compiler emits the same decision tree either way; this is a pure
  legibility win.
- It composes with the broader "aggressively flatten nested matches" rule: a
  `case Outer(...) :: rest -> match rest do case Inner(...) :: _ -> ...` should
  collapse to `case Outer(...) :: Inner(...) :: _ -> ...`.

### Confirmed in-tree

- `finalize` in `src/ir/rdrname.nx` uses
  `_ :: _ :: _ -> return Ambiguous(...)` to assert "two or more matches" —
  exactly the chained-cons form.

### Exception: independent lists in lockstep

If the inner `match` is on a *different* list (zip-style traversal), there is
no nested cons pattern that captures both lists' shapes in a single arm. Leave
the staircase or restructure into a helper. Tuple-pattern matching is not used
in the current codebase; do not introduce it speculatively.

## Integer-range `for` loop

Nexus has one built-in `for` form: a bounded integer-range loop. There is no
collection-iteration `for x in list` — see the anti-patterns table in
`SKILL.md`.

```nexus
// Sum 0 + 1 + 2 + 3 + 4 = 10. Upper bound is exclusive.
let ~sum = 0
for i = 0 to 5 do
  ~sum <- ~sum + i
end

// Empty when lower == upper.
for i = 5 to 5 do
  // never executed
end

// Walks 1, 2, 3, 4, 5.
let ~product = 1
for i = 1 to 6 do
  ~product <- ~product * i
end
```

- `i` is bound fresh per iteration as an `i64`.
- The upper bound is **exclusive** — `for i = lo to hi do ... end` runs the
  body for `i ∈ [lo, hi)`. When `hi <= lo` the loop body is skipped entirely.
- The body sees the enclosing scope, so mutate state via an outer `let ~x = ...`
  (the loop binder itself is immutable).

For traversing a list or array, use `match`/recursion or `list.fold_left` /
`array.fold_left` instead — there is no `for x in xs` syntax.

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

// Iterate via borrow — fold_left is the borrow-side iterator
let _ = array.fold_left(arr: &%arr, init: (), f: fn (acc: unit, val: i64) -> unit do
  Console.println(val: str.from_i64(val: val))
end)

// Consume (f receives each element as linear)
array.consume(arr: %arr, f: fn (val: %i64) -> unit do end)
```

## Lazy Evaluation & Parallel Execution

The `@` sigil marks lazy bindings. A lazy binding defers evaluation until forced.
A single `@expr` force is **synchronous** — it runs the thunk on the calling
thread. Real parallelism comes from forcing many thunks at once via the
`std:lazy` / `std:lazy_host` combinators (each thunk then runs on its own OS
thread, via WASI threads).

```nexus
// Lazy binding: RHS is NOT evaluated here
let @expensive = heavy_computation(input: data)

// Force: runs the thunk now (synchronous), returns the result
let result = @expensive

// Lazy with captured variables
let base = 100
let @derived = base * 2 + some_call(n: base)
let val = @derived    // captures `base`; forced synchronously here

// Inline force on expression (no binding needed)
let quick = @(x + y)
```

### Parallel: `std:lazy.force_all` and `std:lazy_host`

```nexus
import * as lazy from "std:lazy"

let @a = work(n: 1000)
let @b = work(n: 1001)
let @c = work(n: 1002)
let results = lazy.force_all(tasks: [a, b, c])   // a, b, c run on 3 OS threads
                                                 // results in input order: [ra, rb, rc]
```

```nexus
import { host_spawn, host_join } from "std:lazy_host"

let @t = expensive()
let h = host_spawn(a: t)        // %Task<T> — linear, must be joined
// ... overlapping work on this thread ...
let v = host_join(handle: h)    // waits for the worker, returns the value
```

Run threaded programs via the bundled `nexus` launcher (it passes
`-W threads=y,shared-memory=y -S threads` to wasmtime).

### Type: `@T`

`@T` is a lazy thunk that, when forced, produces a value of type `T`.

```nexus
// Type annotation
let compute = fn (input: @i64) -> i64 do
  return @input + 1
end
```

### Runtime model

- `let @x = expr` packages the expression as a 0-arg closure (a heap object
  whose word 0 is its funcref-table index); the captured free variables ride in
  the closure object
- `@x` is a synchronous `call_indirect` on that closure
- `std:lazy.force_all` / `std:lazy_host.host_spawn` allocate a small task struct
  in shared linear memory and call the `wasi.thread-spawn` import; the spawned
  thread re-enters at the `wasi_thread_start` export, forces the closure against
  the same shared memory, parks the result, and `notify`s the joiner — which is
  parked in `memory.atomic.wait32`
- Thunks with no side effects parallelise cleanly; combinators do not insulate
  observable effects (`race` / `cancel` / `detach` in `std:lazy` are still
  sequential — see their docstrings)

