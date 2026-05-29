# Nexus Idiomatic Code Patterns

## Program Structure

Every executable Nexus program needs a `main` function:

```nexus
// Minimal program — no caps required
let main = fn () -> unit do
  return ()
end
```

For I/O programs, see [effects_console_fs.nx](../../../examples/feature/effects_console_fs.nx) (two caps, inject, try/catch) or [import_forms.nx](../../../examples/feature/import_forms.nx) (named / wildcard / combined imports).

## Import Patterns

Working example: [import_forms.nx](../../../examples/feature/import_forms.nx) (named / wildcard / combined import forms).

The three shapes:
- Named: `import { Foo, bar } from "mod"` — bring specific bindings into scope.
- Wildcard: `import * as m from "mod"` — namespace alias.
- Combined: `import { Foo }, * as m from "mod"` — both at once.

## Custom Cap + Handler (Dependency Injection)

Working example: [handler_basic.nx](../../../examples/feature/handler_basic.nx) (define a `cap`, two `handler` implementations, swap via `inject`).

Pattern: (1) declare `cap Foo do fn op(...) end`; (2) implement `let h = handler Foo require { ... } do fn op ... end`; (3) annotate callers with `require { Foo }`; (4) install at the boundary with `inject h do ... end`.

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
  `src/frontend/parser/core.nx`); order is not significant between independent
  handlers.
- The comma form keeps `BODY`'s indentation flat. Nested `inject` walls add an
  indentation level per handler with no extra meaning.
- Production code already favours this form: `main` in `src/main.nx` injects
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

## Handler arm `with @ k` (named continuation binder)

Inside a handler arm body, the resumption continuation is implicitly in
scope. The `with @ name` clause **binds the continuation to an explicit
name** for the body, shadowing the implicit form. Useful when an arm needs
to capture-and-defer (e.g. enqueue the continuation, run it on another
thread, or invoke it more than once).

```nexus
let yielding_sched = handler Coroutine do
  fn yield(val: i64) -> i64 with @ k do
    // `k` is the continuation; calling it with the resume value steps
    // the suspended computation forward by one tick.
    enqueue(task: @k)
    return 0  // arm returns to whoever called yield, not to the user code
  end
end
```

- The bare arm form `fn yield(val: i64) -> i64 do ... end` still works
  without `with @ k` — the continuation is just unnameable.
- Without `with @ k`, calling-the-continuation syntax (`resume(val: ...)`)
  exposes it under a fixed implicit name; the explicit form lets you choose
  the name (`k`, `kont`, `cc`, etc.) and is required if the arm body
  contains a nested closure that wants to capture the continuation.
- Parsed by `src/frontend/parse_args.nx::parse_optional_with_kont`; carried as
  `cont_binder: Option<string>` on `HandlerArm` in `src/syntax/ast.nx` (line 155).

## Error Handling Patterns

### Result-based (prefer for recoverable errors)

Working example: [result_basics.nx](../../../examples/feature/result_basics.nx) (`Ok`/`Err` return, `map`, `unwrap_or`, `is_err`).

### Exception-based (for unrecoverable or I/O errors)

Working example: [try_catch.nx](../../../examples/feature/try_catch.nx) (declare exceptions, `throw`, selective catch arms, bare catch).

See also [exception_group.nx](../../../examples/feature/exception_group.nx) for grouping multiple exceptions under one `throws` label.

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
  | Err(err: m) -> throw ConfigError(msg: m)
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

For traversing a list, use `match`/recursion or `list.fold_left` — there is no
`for x in xs` syntax. For an array, index it inside an integer-range `for` loop
(`for i = 0 to n do ... a[i] ... end`) through a `&[| T |]` borrow parameter.

## Linear Resource Management

Nexus linear types (`%`) enforce exactly-once consumption at compile time.

- [linear_consume.nx](../../../examples/feature/linear_consume.nx) — `%Box` consumed exactly once; ownership transferred on call.
- [linear_borrow.nx](../../../examples/feature/linear_borrow.nx) — `&Box` borrow: inspect without consuming; caller retains ownership.
- [linear_thread.nx](../../../examples/feature/linear_thread.nx) — threading `%Counter` through recursive accumulation.
- [hashmap_basics.nx](../../../examples/feature/hashmap_basics.nx) — linear `HashMap`: writes consume + return new handle, reads borrow.
- [stringmap_basics.nx](../../../examples/feature/stringmap_basics.nx) — linear `StringMap` with the same ownership story.

## Array Patterns

Arrays `[| T |]` are a low-level **linear** primitive. There is **no `std:array`
module** — element access happens only through `&[| T |]` **borrow parameters**
(borrows are valid solely in argument position; you cannot `let view = &%arr`).
Inside such a parameter, index with `a[i]` (read) and `a[i] <- v` (write), and
walk it with an integer-range `for` loop.

```nexus
// Read/write through borrow parameters — &%arr is only ever passed as an argument.
let fill = fn (a: &[| i64 |], n: i64) -> unit do
  for i = 0 to n do          // exclusive upper bound: i ∈ [0, n)
    a[i] <- i * 10
  end
  return ()
end

let sum = fn (a: &[| i64 |], n: i64) -> i64 do
  let ~total = 0
  for i = 0 to n do
    ~total <- ~total + a[i]
  end
  return ~total
end

// caller:
//   let %arr = [| 0, 0, 0 |]
//   fill(a: &%arr, n: 3)
//   let s = sum(a: &%arr, n: 3)
```

Consumption caveat: a `%[| T |]` is linear and must be consumed exactly once, but
arrays have no element-wise destructure and the language ships no array-free /
`consume` intrinsic — so for most code prefer the immutable `[T]` list
(recursion / `std:list`) and reach for `[| T |]` only when in-place mutation is
genuinely required.

## Pattern shorthands

### `Ctor(_)` — match any constructor instance (all fields ignored)

When matching on a constructor and you don't need any of its fields,
`Ctor(_)` is the shorthand for "match any payload":

```nexus
match opt do
  | Some(_) -> "got some"          // any payload, don't bind it
  | None -> "got none"
end
```

This is distinct from `Ctor(field: _)` (named-field wildcard) and from
plain `_` (matches anything). Use `Ctor(_)` when you care about the
constructor tag but not the data.

Equivalent expansions for a ctor with N fields:

```nexus
| Some(_)             // shorthand
| Some(val: _)        // explicit single-field wildcard
| Some(_, _)          // not valid — positional patterns aren't accepted
```

The parser recognises this in `src/frontend/parse_pattern.nx::is_ctor_wildcard_all`.

## Handler `require { Row }` — always row-typed

Handler declarations must spell out their cap requirements as a row in
braces, never as a bare type identifier:

```nexus
// CORRECT — required row
let logger_handler = handler Logger require { PermConsole } do
  fn info(msg: string) -> unit do
    Console.println(val: msg)
  end
end

// CORRECT — no requirements (empty row)
let pure_handler = handler PureCap do
  fn pure_op() -> i64 do return 0 end
end

// WRONG — bare type, src rejects this even though some lenient grammars
// would parse it
let logger_handler = handler Logger require PermConsole do
  ...
end
```

The same braced-row form applies wherever `require { ... }` appears
(function signatures, cap declarations). Empty row is `require { }` or
omit `require` entirely.

## Bitwise OR (`|`)

The `|` operator is bitwise-or on integers. The compiler resolves `|` to
either pattern alternation (`| Pat -> ...`) or bitwise-or based on lookahead
to the next `->` at depth 0.

```nexus
let masked = 12 | 10   // bitwise-or: 0b1100 | 0b1010 = 0b1110 = 14
```

### Disambiguation rule

At the **tail expression** of a match/catch arm body, `|` defers to the
outer arm parser when followed by a pattern + `->` at depth 0. To force
bitwise-or in tail position, wrap with parens:

```nexus
// Tail-position bitwise-or — needs parens:
match x do
  | 0 -> 256
  | _ -> (x | 1)        // OK
end

// Without parens, this would re-enter arm parsing:
match x do
  | 0 -> 256
  | _ -> x | 1          // parse error: `1` expected to be followed by `->`
end
```

Other operator positions don't need disambiguation:

```nexus
let v = a | b           // OK — let-rhs is unambiguous
let w = (a | b) + c     // OK — paren'd
if (flags | MASK) != 0 then ... end   // OK — if-cond is unambiguous
```

The disambiguation logic lives in `src/frontend/parser/core.nx::parse_prec_loop`
via `pcore.pipe_starts_arm` (depth-tracking lookahead scan, 64-token fuel
cap).

## Lazy Evaluation & Parallel Execution

The `@` sigil marks lazy bindings. A lazy binding defers evaluation until forced.
A single `@expr` force is **synchronous** — it runs the thunk on the calling
thread. Real parallelism comes from forcing many thunks at once via the
`std:lazy` / `std:lazy_host` combinators (each thunk then runs on its own OS
thread, via WASI threads).

**Thunk-creation vs force.** The only surface form that *creates* a thunk is
the let-binding sigil `let @x = e` — it wraps `e : T` into a thunk of type
`@T`. Every `@e` in expression position is **force** (`@T → T`), including
both `@x` (bare ident) and `@(expr)` (parenthesized compound). The parser
treats `@(...)` as `ExprForce`, not as thunk-introduction. This matches the
formal type-system rule `wrapSigil(@, τ) = @τ` on let-bindings together with
T-Force on expressions.

Working examples:
- [lazy_force.nx](../../../examples/feature/lazy_force.nx) — single thunk creation and synchronous force.
- [lazy_parallel.nx](../../../examples/feature/lazy_parallel.nx) — `lazy.force_all` on multiple thunks (WASI threads).

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

