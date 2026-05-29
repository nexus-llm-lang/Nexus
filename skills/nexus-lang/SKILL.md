---
name: nexus-lang
description: "Write Nexus (.nx) programs — an LLM-friendly language with linear types, coeffects (caps/handlers), and WASM compilation. Use when writing, reviewing, or explaining Nexus code."
---

# Nexus Language

Nexus is a programming language designed for LLM-friendly code generation. Its premise: **"LLMs are strong at literal program constructs but weak at contextual ones."** Every construct is syntactically explicit — no implicit resource cleanup, no hidden aliasing, no ambient I/O.

**Documentation**: https://nexus-llm-lang.github.io/latest/

## When to Use This Skill

- Writing new `.nx` source files
- Reviewing or debugging Nexus code
- Explaining Nexus language features
- Porting algorithms to Nexus
- Working with Nexus's type system, caps/handlers, or linear types

## When NOT to Use This Skill

- Non-Nexus programming tasks

## Ground Your Work in the Compiler — `nexus` Introspection CLI

Don't guess what the compiler sees — ask it. These subcommands are **read-only**
(they write no files and are safe to run at any point), and take either one `.nx`
file or resolve project-wide. Reach for them by situation:

| Situation | Command |
|-----------|---------|
| Just edited a file — does it still compile? | `nexus typecheck <file>` |
| What is this symbol — signature, kind, defining file, docstring? | `nexus explain <symbol> [--file <file>]` |
| What type did the compiler infer (unannotated `let`s, effect rows)? | `nexus types <file> [--transitive]` |
| Does this function touch Fs / Net / Console / …? Which caps to `require`? | `nexus caps <symbol> <file>` |
| Where is X defined / every call site of X? | `nexus grep '<kind> <name>' [path]` |
| Orienting in an unfamiliar file: defs, exports, import edges | `nexus context <file>` |
| Debug how syntax actually parsed (punning, sigils, precedence) | `nexus ast <file>` |
| Debug `let`/`fn`/handler desugaring or effect-row inference | `nexus hir <file>` |
| Chase a codegen bug / size a function's wasm output | `nexus lir <file>` |

**The feedback loop.** After every edit, run `nexus typecheck <file>` — it runs only
the front-end (parse → resolve → HIR → full typecheck incl. effect rows + linearity),
so it is fast and is the pre-commit target. Exit `0` = clean, `1` = diagnostic on
stderr, `2` = bad usage. Don't claim an edit works until it type-checks.

**Notes.**
- Append `--format json` to `ast`/`hir`/`lir`/`types`/`caps`/`explain`/`context` for
  machine-readable output (effect rows appear in `types --format json`).
- `nexus grep` is **AST-aware**, not textual: `<kind>` ∈ `let|fn|call|var` (default
  `call`), `<name>` is a literal or a `$`-wildcard (`$_` = any). Prefer it over text
  search for "every call to `Fs.open_read`" or "the `let` named `helper`".
- `nexus explain` / `nexus caps` accept a bare symbol suffix or the qualified
  `<sanitized_path>#<name>` form; `explain` without `--file` searches the whole project.
- Before finishing, `nexus fmt --check <file>` and `nexus lint <file>` are the
  formatting / style safety pass (run `nexus help` for the full subcommand list).
  To auto-fix lint findings and compiler diagnostics (E2005/E2007), use
  `nexus lint --fix <file>` (writes in place) or `nexus lint --fix-diff <file>`
  (preview only). The standalone `nexus fix` subcommand no longer exists.

## Quick Reference

### File Extension
`.nx`

## Core Syntax Rules

### 1. All arguments are labeled (order-independent)
```nexus
// CORRECT — label order does not matter
add(a: 1, b: 2)
add(b: 2, a: 1)          // equivalent
Console.println(val: "hello")
x :: xs

// WRONG — positional arguments do not exist
add(1, 2)
Console.println("hello")
```

### 2. Blocks use `do ... end` (no braces)
```nexus
if x > 0 then
  return x
else
  return 0
end

match opt do
  | Some(val: v) -> return v
  | None -> return 0
end

while running do
  process()
end
```

### 3. Functions are `let` bindings to lambdas
```nexus
export let add = fn (a: i64, b: i64) -> i64 do
  return a + b
end

// With generics
export let map = fn <T, U>(opt: Option<T>, f: (val: T) -> U) -> Option<U> do
  match opt do
    | Some(val: v) ->
      let mapped = f(val: v)
      return Some(val: mapped)
    | None -> return None
  end
end
```

### 4. Sigils: `%` (linear), `&` (borrow), `~` (mutable ref)
```nexus
let %arr = [| 1, 2, 3 |]        // linear — must be consumed exactly once
let ~x = 10                     // mutable ref, stack-confined
~x <- 20                        // reassignment
let v = ~x                      // dereference
```

**Borrows are valid only in argument position** — you cannot bind one to a `let`
(`let view = &%arr` is rejected: `T-Borrow` requires the target be non-linear).
Pass `&%arr` directly as a call argument; the callee reads/mutates through the
borrow without consuming, and `T-App` discharges the borrow on return:
```nexus
let set_elem = fn (a: &[| i64 |], i: i64, v: i64) -> unit do
  a[i] <- v                     // mutate element through a borrow parameter
  return ()
end
let get_elem = fn (a: &[| i64 |], i: i64) -> i64 do
  return a[i]                   // read element through a borrow parameter
end
// caller: set_elem(a: &%arr, i: 0, v: 42)  — &%arr only as an argument
```
`[| T |]` is a low-level linear primitive: there is **no `std:array` module**, and
element access is only via `&[| T |]` borrow parameters as shown. For ordinary code
prefer the immutable `[T]` list (recursion / `std:list`). Reach for `[| T |]` only
when you genuinely need in-place mutation.

### 5. Lazy evaluation with `@`
```nexus
let @result = expensive_call(data: input)  // deferred thunk
// ... other work ...
let val = @result                          // force: runs the thunk now (synchronous)
```
A single `@x` force is synchronous. For real parallelism use `std:lazy`'s
`force_all([@a, @b, ...])` (each thunk runs on its own OS thread, WASI threads;
results returned in input order) or `std:lazy_host`'s `host_spawn` / `host_join`
for explicit per-thunk control. Run threaded programs via the bundled `nexus`
launcher (it passes `-W threads=y,shared-memory=y -S threads` to wasmtime).

Thunk-creation vs force: `let @x = e` (let-binding sigil) is the **only**
thunk-creation form — it wraps `e` into an `@T` thunk. Every `@e` in
expression position is **force** (`@T → T`), including `@x` (bare ident) and
`@(expr)` (parenthesized compound). The let-sigil form follows the formal
spec's `wrapSigil(@, τ) = @τ`; `@(expr)` thunk-creation has no corresponding
spec rule and is not supported — write `let @x = expr` instead.

### 6. Explicit `return` required (except unit)
```nexus
// Non-unit functions must have explicit return
let double = fn (n: i64) -> i64 do
  return n * 2
end

// Unit-returning functions may omit return ()
let greet = fn (name: string) -> unit require { Console } do
  Console.println(val: "Hello, " ++ name)
end
```

### 7. Imports: `pkg:path` for packages, bare paths for local files

```nexus
// Standard library — std is the package name; module name follows the colon
import { Console }, * as stdio from "std:stdio"
import * as list from "std:list"
import { Result, Ok, Err } from "std:result"

// Local files — bare relative paths (no colon)
import { HandlerArm } from "src/syntax/ast.nx"

// FFI binding — declares the host module for subsequent `external` decls
import external "std:str"
external __nx_string_length = "__nx_string_length" : (s: string) -> i64
```

Path forms:
| Form | Resolves to | Use |
|------|-------------|-----|
| `"std:stdio"` | `nxlib/stdlib/stdio.nx` | Standard library module |
| `"pkg:path/module"` | `<pkg-root>/path/module.nx` | Third-party package module |
| `"src/foo.nx"` | Relative file path | Project-local module |
| `import external "std:<mod>"` | Host module providing `<mod>` | Pin FFI imports to a host module |

The `std` package always maps to `nxlib/stdlib/`.

### 8. Comments: line + block (nesting OK)

```nexus
// line comment

/* block comment */

/* outer /* nested */ still inside the outer comment */
```

Both `//` and `/* ... */` comment forms are supported. Block comments
**nest correctly** — the lexer counts `/*` / `*/` depth, so nested blocks
close in the right order. Useful for commenting out a region that already
contains block-comment'd code.

## Statement-typing deltas

Points where the in-tree implementation matches user expectation but the
canonical `type-system-formal.md` spec is incomplete. Code behaves as
documented here; the spec is being tightened separately.

> Note: the formal spec writes the throw-term as `raise e`; the **surface
> keyword you actually type is `throw`** (see Error Handling above). Spec rule
> names below quote the spec's `raise` spelling, but any code you write uses
> `throw`.

### Expression statements (T-ExprStmt) — nexus-ka1m

The term grammar `s ::= ... | e` admits a bare expression as a statement,
but the spec has no rule lifting `Γ; ρ_q ⊢_e e : τ ! ρ_e` into the
statement judgement `Γ; ρ_q; τ_r ⊢_s e : Γ ! ρ_e`. The implementation
*does* lift expressions: `infer_stmt` dispatches the `Expr(e)` HIR
statement straight to `infer_expr` (`src/typecheck/infer.nx`,
`infer_stmt`'s `Expr` arm). A future spec patch will add a `T-ExprStmt`
rule with output `Γ` unchanged and `tail(...) = τ`. Until then, treat
`s ::= e` derivations as "typed by `infer_expr` with the surrounding
`τ_r`".

### `tail` and divergent destructuring let — nexus-1t8n

The `tail(s̄)` predicate (§Expressions) classifies the last statement
of a block as ⊥ when it is `return`, an expression-statement `raise e`,
or a single-binder `let μx = raise e'`. The destructuring form
`let p = raise e` (handled by `T-LetPat-Diverge`) is **not** in the ⊥
list and currently falls into the `unit` "otherwise" arm. In practice
a match arm whose body is exactly `let Some(y) = raise NotFound(...) end`
types as `unit` rather than ⊥, and `T-Match`'s divergent-arm carve-out
does not fire — HIR desugars the form to
`match (raise e) do | p -> end` (`src/ir/hir/hir.nx`, `StmtLetPattern`
case), and the trailing `infer_stmts([])` yields `TyUnit`. Workaround
when you want the arm classified as divergent: write `throw e` as an
expression-statement (or precede with `return`) instead of binding it.
The pending spec fix extends the ⊥ clause to `let p = raise e'` so both
binding shapes behave uniformly.

## Effect System (Caps & Handlers)

Nexus uses coeffects for dependency injection, NOT algebraic effects. See https://nexus-llm-lang.github.io/latest/spec/effects for details.

Minimal self-contained `cap` / `handler` / `inject` (compiles as-is):
```nexus
import { Console }, * as stdio from "std:stdio"

cap Logger do                              // declare the capability (interface)
  fn log(msg: string) -> unit
end

let console_logger = handler Logger require { PermConsole } do   // implement it
  fn log(msg: string) -> unit do
    Console.println(val: "[LOG] " ++ msg)
    return ()
  end
end

let greet = fn (name: string) -> unit require { Logger } do      // depend on the cap
  Logger.log(msg: "Hello, " ++ name)
  return ()
end

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler, console_logger do                // supply handler(s)
    greet(name: "World")
  end
  return ()
end
```

Working example: [handler_basic.nx](../../examples/feature/handler_basic.nx) (define a `cap`, implement a `handler`, inject with `inject h do ... end`).

Convention: `require { ... }` and `throws { ... }` rows are sets — order is
irrelevant to the typechecker. List entries alphabetically (e.g.
`require { Console, Fs }`, `require { PermClock, PermConsole, PermFs, PermProc }`).
Omit `require { ... }` entirely when the body needs no caps.

## Type System Summary

| Type | Syntax | Notes |
|------|--------|-------|
| Primitives | `i32`, `i64`, `f32`, `f64`/`float`, `bool`, `char`, `string`, `unit` | `i64` and `f64` are defaults |
| Record | `{ x: i64, y: i64 }` | Structural typing |
| ADT/Enum | `Ok(val: T) \| Err(err: E)` | Labeled fields |
| List | `[ T ]` | Immutable singly-linked |
| Array | `[| T |]` | Linear, mutable |
| Function | `(a: i64) -> i64` | Always labeled params |
| Generic | `Option<T>`, `Result<T, E>` | Explicit type params |
| Linear | `%T` | Must consume exactly once |
| Borrow | `&T` | Immutable view |
| Lazy | `@T` | Deferred thunk; `@x` forces synchronously, `std:lazy.force_all` / `std:lazy_host` run in parallel |
| Opaque | `opaque type X = ...` | Hidden constructors |

## Common Patterns

### Hello World
```nexus
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello, World!")
  end
  return ()
end
```

`main` is the runtime entry-point hook — declare it as a plain `let main` and
**never `export` it** (`export let main` is rejected with E2018).

### Error Handling

**The keyword is `throw`** (not `raise`): `throw SomeException(field: ...)` raises an
exception; the function's `throws { ... }` row must list it. Catch with `try ... catch`.

Working example: [try_catch.nx](../../examples/feature/try_catch.nx) (throw, selective catch arms, bare catch).

**When to use which form:**
- **Bare `catch <ident> -> body end`** — one identifier binds the whole exception value. Use when the handler treats every escaping exception uniformly (log, rethrow, return a default). `catch _ -> ...` is the same shape with the value discarded.
- **Selective `catch | Pat -> ... | Pat -> ... end`** — pipe-separated arms pattern-match on exception constructors. Use when arms need to inspect payload fields or branch on the exception variant.

For multi-exception phases, declare an `exception group` and use the group name in the row:

```nexus
exception group ParseError = UnexpectedToken | UnexpectedEof

let parse_top = fn (toks: [Token]) -> Decl throws { ParseError } do
  // raises either UnexpectedToken or UnexpectedEof
end
```

Working example: [exception_group.nx](../../examples/feature/exception_group.nx) (declare a group, throw members, catch by variant).

### List Recursion

Working example: [list_basics.nx](../../examples/feature/list_basics.nx) (recursive sum, `std:list` combinators, cons patterns).

## Comment and Documentation Policy

**Do not embed `bd` issue IDs (`nexus-XXXX`) in source comments or filenames.**
The `bd` database is local to this repository and unresolvable for external readers.
Write self-contained comments that explain *why* without referencing the issue tracker.
Capture bd context in commit messages and PR descriptions instead.

See `AGENTS.md` for the full policy.

## Preferred Writing Style

These are style preferences beyond correctness — both forms compile, but the left-hand form is idiomatic in this codebase.

### 1. `let` destructuring over single-arm `match`
When a value has exactly one shape (a record, a single-constructor ADT, or a known-tuple), destructure with `let` instead of a trivial `match`.

```nexus
// PREFER
let { x: x, y: y } = point
let Some(val: v) = must_exist

// AVOID — single-arm match adds noise
match point do
  | { x: x, y: y } -> ...
end
```

### 2. `if let` over two-arm `match` (one constructor + wildcard)
When a `match` has exactly two arms — one constructor pattern with bindings and a wildcard fallback — rewrite as `if let PAT = EXPR then ... else ... end`. Both statement-form and expression-form are supported. This applies recursively: a nested `match X do | F(b) -> A | _ -> B end` inside another arm should also become `if let`.

Skip the rewrite when the constructor arm has **no bindings** (e.g. `TkColon -> true | _ -> false`) — `if let` adds syntax without payoff there. The point is to lift a meaningful destructure out of `match` noise.

```nexus
// PREFER — if let surfaces the destructure as the primary control choice
if let TkString(val: s) = peek_token(toks: toks) then
  return ParsedIdent(name: s, rest: skip(toks: toks))
else
  throw UnexpectedToken(expected: "string literal", ...)
end

// Expression form
let arr_elem = if let TyArray(elem: e) = arr_t then e else TyI64 end

// AVOID — two-arm match on a single binding constructor
match peek_token(toks: toks) do
  | TkString(val: s) -> return ParsedIdent(name: s, rest: skip(toks: toks))
  | _ -> throw UnexpectedToken(expected: "string literal", ...)
end
```

Combine with rule #1: if the `else` branch is unreachable (single-constructor type), prefer `let PAT = EXPR` instead of `if let ... else`.

### 3. Collapse staircase `match` — nest patterns + aggressive `_`
Fuse nested `match` arms into a single pattern. Use a trailing bare `_` to ignore all remaining record/constructor fields rather than binding and discarding them.

```nexus
// PREFER — one arm, nested pattern, _ swallows the rest
match res do
  | Ok(val: Some(val: v)) -> use(x: v)
  | _ -> fallback()
end

// AVOID — staircase of matches
match res do
  | Ok(val: inner) ->
    match inner do
      | Some(val: v) -> use(x: v)
      | None -> fallback()
    end
  | Err(err: _) -> fallback()
end
```

**List variant — peel multiple heads on the surface, not by re-matching the tail.** When an outer arm binds `head :: tail` only to immediately re-`match` `tail`, fold the inner shapes into the outer pattern. Use `[x]` for single-element, `a :: b :: rest` for two-or-more, etc. `::` chains right-associatively in patterns just like in expressions.

```nexus
// PREFER — arity each arm expects is visible in the pattern
match xs do
  | [] -> handle_empty()
  | [x] -> handle_one(x)
  | x :: y :: ys -> handle_two_or_more(x, y, ys)
end

// AVOID — second match buries the arity in nested indentation
match xs do
  | [] -> handle_empty()
  | x :: rest ->
    match rest do
      | [] -> handle_one(x)
      | y :: ys -> handle_two_or_more(x, y, ys)
    end
end
```

Internally the compiler produces the same decision tree either way; this is pure legibility. The pattern says "this arm wants exactly N elements" / "this arm wants at least N elements" — buried in a second `match` it says only "this arm has a head".

Exception: when the inner `match` is on a *different* list (zip-style lockstep traversal), no single-arm flattening exists. Leave the staircase or extract a recursive helper.

See `references/patterns.md` § "List Pattern Flattening" for more.

### 4. Or-patterns to share an arm body across alternatives
Whenever **two or more** arms in the same `match` have **identical RHS**, fuse them with `|` into a single arm. This includes the 3+ case — `p1 -> e | p2 -> e | p3 -> e` collapses to `p1 | p2 | p3 -> e`. All alternatives must bind the same variable names with compatible types (or no bindings at all, which is the common case).

This is a hard preference, not a suggestion: duplicated RHS rots — when one arm's body changes, the others silently drift.

```nexus
// PREFER — one arm, three ctors share the body
match tok do
  | TkPlus | TkMinus | TkStar -> precedence_arith()
  | TkAnd | TkOr -> precedence_logic()
  | _ -> 0
end

// AVOID — three arms with the same body
match tok do
  | TkPlus -> precedence_arith()
  | TkMinus -> precedence_arith()
  | TkStar -> precedence_arith()
  | TkAnd -> precedence_logic()
  | TkOr -> precedence_logic()
  | _ -> 0
end
```

Mixing bindings is fine when the names and types line up:

```nexus
// PREFER — both ctors carry an i64 named `n`
match num do
  | Pos(n) | Neg(n) -> abs(x: n)
  | Zero -> 0
end
```

Skip the fusion when the bodies *look* identical but reference different binders (`Foo(x) -> use(x)` vs `Bar(y) -> use(y)`) — those are genuinely different bodies; either rename one binder or leave the arms separate.

### 5. Punning — drop the label when it matches the local name
When a function call, constructor call, constructor pattern, record literal, or record pattern passes/binds a variable whose name equals the field label, omit `label:`. The parser desugars `f(x)` to `f(x: x)`, `| Ok(v)` to `| Ok(v: v)`, `{name, age}` to `{name: name, age: age}`, and `let {x} = r` to `let {x: x} = r`. Sigils ride along: `f(%v)`, `f(&v)`, `f(~v)`, `f(@v)`, `f(&%v)` all pun to `f(v: ...)`; the same shapes work inside `{ … }`.

```nexus
// PREFER — pun when names coincide
let val = 42
return Mk(val)              // desugars to Mk(val: val)
f(x)                        // desugars to f(x: x)
graph.add(%node)            // desugars to graph.add(node: %node)
ctx.lookup(&env)            // desugars to ctx.lookup(env: &env)
let user = {name, age}      // desugars to {name: name, age: age}
let {name, age} = user      // desugars to {name: name, age: age}
let ctx = {%cap, &env}      // desugars to {cap: %cap, env: &env}
match w do
  | Mk(val) -> return val   // desugars to | Mk(val: val)
  | Box(%inner) -> ...      // desugars to | Box(inner: %inner)
end
match user do
  | {name, age: a} -> ...   // desugars to {name: name, age: a} — mixed
end

// AVOID — redundant `name: name` / `name: %name`
return Mk(val: val)
f(x: x)
graph.add(node: %node)
let user = {name: name, age: age}

// Cannot pun — sigil applied to a non-bare-ident value, or names differ
return Some(val: name)
greet(name: "Bob")
f(arg: g(x))               // value is a call, not a bare variable
f(arg: %x.field)           // value is a field access, not a bare variable
let user = {name: caller.name}     // RHS is field access, keep explicit
```

Record patterns: trailing `_` (alone, no comma RHS) is still the open-rest marker — `{name, age, _} = user` punning two fields and ignoring the rest works as expected.

### 6. Shadowing — reuse the binder when each step replaces the previous value
When a sequence of `let`s threads a value through transformations and each intermediate is consumed exactly once by the next line, **reuse the same name** instead of inventing `r1` / `r2` / `r3` / `%buf2` / `%out2`. Shadowing makes "this is the current value" the obvious reading; numeric suffixes invite the question "is `r2` still alive after this?" that the reader then has to answer by scanning ahead.

Apply only when **every intermediate is dead at the next binding**. If you need two values at once (e.g. 1- vs 2-character lookahead, before/after diff, ring-buffer pairs), keep distinct names.

```nexus
// PREFER — reuse `r` since each step discards the previous one
let r = pcore.skip(toks)
let r = pcore.expect(toks: r, expected: "(")
let ParsedType(typ: inner, rest: r) = parse_type(toks: r)
let r = pcore.expect(toks: r, expected: ")")
return ParsedType(typ: TyRef(inner), rest: r)

// AVOID — numeric-suffix chain when intermediates are never reused
let r = pcore.skip(toks)
let r2 = pcore.expect(toks: r, expected: "(")
let ParsedType(typ: inner, rest: r3) = parse_type(toks: r2)
let r4 = pcore.expect(toks: r3, expected: ")")
return ParsedType(typ: TyRef(inner), rest: r4)

// KEEP DISTINCT — both `c` and `c2` are read in the same expression
let c = peek(st)
let c2 = peek_at(st, offset: 1)
if c == 47 && c2 == 47 then ... end
```

### 7. Narrow `throws` rows — list what actually escapes, not `Exn`
A `throws` row declares the *set of exceptions that may escape this function*. Writing `throws { Exn }` widens to "anything at all" and erases the catch-side type information that makes precise `catch` arms meaningful. Reserve `throws { Exn }` for true boundary functions — top-level error formatters, generic test harnesses, REPL drivers — where the row really is unbounded.

For everything else, name the specific exceptions or an already-declared `exception group`:

```nexus
// PREFER — row enumerates exactly what escapes
let parse_top = fn (toks: [Token]) -> Decl throws { ParseError } do
  throw UnexpectedToken(span: ...)
end

let load_config = fn (path: string) -> Config require { Fs } throws { FileNotFound, ParseError } do
  ...
end

// AVOID — Exn admits any exception, including ones the caller can't reasonably catch
let parse_top = fn (toks: [Token]) -> Decl throws { Exn } do
  throw UnexpectedToken(span: ...)
end
```

When a phase grows several distinct exceptions, declare a group and reference it instead of repeating the alternatives at every call site:

```nexus
exception group HirError = InvalidSymbolTag | EmptyScopeStack | NoActiveScope

let lower_to_hir = fn (...) -> HirProgram throws { HirError } do ...
```

Partial functions (operations defined only on a subset of inputs — `head`, `tail`, `unwrap`, `to_i64`) **must raise a domain-specific exception**, not `RuntimeError(val: "...")`. Catch-all error strings discard the structured information callers need to recover. Declare a real `exception` per failure mode:

Exception names are **unit-global** (they share one namespace across all modules,
imported or not), so don't reuse a name the stdlib already exports — e.g. `EmptyList`
is taken by `std:list` and redeclaring it is E2029. Pick a distinct name:

```nexus
// PREFER
exception EmptyInput(op: string)
let head = fn <T>(xs: [T]) -> T throws { EmptyInput } do
  match xs do
    | [] -> throw EmptyInput(op: "head")
    | v :: _ -> return v
  end
end

// AVOID — RuntimeError is the dumping ground; callers can't pattern-match on intent
let head = fn <T>(xs: [T]) -> T throws { Exn } do
  match xs do
    | [] -> throw RuntimeError(val: "head: empty list")
    | v :: _ -> return v
  end
end
```

## Anti-Patterns to Avoid

| Avoid | Use Instead |
|-------|-------------|
| Positional arguments: `f(1, 2)` | Labeled: `f(a: 1, b: 2)` |
| Brace blocks: `{ ... }` | `do ... end` / `then ... end` |
| `return` omitted for non-unit | Explicit `return` for non-unit functions |
| Capturing `~x` in closure | Only immutable bindings captured |
| `let _ = linear_val` | Consume linear values via function call or pattern match |
| Implicit I/O | Declare via `require { PermConsole }` + inject handler |
| `var x = 5` | `let ~x = 5` for mutable |
| `for x in list` (collection iteration) | No such form exists — use `match`/recursion or `list.fold_left`. The integer-range `for i = lo to hi do ... end` *does* exist (exclusive upper); see `references/patterns.md`. |
| `throws { Exn }` for a function with a known exception set | List the actual exceptions or an `exception group` (see rule #7) |
| Multiple match arms with identical RHS: `p1 -> e \| p2 -> e \| p3 -> e` | Fuse with: `p1 \| p2 \| p3 -> e` (see rule #4) |
| List arm that re-matches its tail: `\| x :: rest -> match rest do ...` | Peel heads on the surface: `\| [x] -> ...` and `\| x :: y :: ys -> ...` (see rule #3 list variant) |
| `throw RuntimeError(val: "...")` from a partial function | Declare a domain-specific `exception` and `throw` that |
| `let r = ...; let r2 = ...; let r3 = ...` when intermediates are dead | Shadow: reuse `r` (see rule #6) |

## Reference Files

- https://nexus-llm-lang.github.io/latest/spec/syntax — Syntax and EBNF grammar
- https://nexus-llm-lang.github.io/latest/spec/types — Type system, linear types, borrowing
- https://nexus-llm-lang.github.io/latest/spec/effects — Caps, handlers, inject, permissions
- https://nexus-llm-lang.github.io/latest/env/stdlib — Standard library API reference
- `./references/stdlib.md` — `std` package module index, capability permissions
- `./references/patterns.md` — Idiomatic code patterns with examples
- `./templates/` — Starter templates for common program structures
