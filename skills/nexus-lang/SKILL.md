---
name: nexus-lang
description: "Write Nexus (.nx) programs — an LLM-friendly language with linear types, coeffects (ports/handlers), and WASM compilation. Use when writing, reviewing, or explaining Nexus code."
---

# Nexus Language

Nexus is a programming language designed for LLM-friendly code generation. Its premise: **"LLMs are strong at literal program constructs but weak at contextual ones."** Every construct is syntactically explicit — no implicit resource cleanup, no hidden aliasing, no ambient I/O.

**Documentation**: https://nexus-llm-lang.github.io/Nexus/latest/

## When to Use This Skill

- Writing new `.nx` source files
- Reviewing or debugging Nexus code
- Explaining Nexus language features
- Porting algorithms to Nexus
- Working with Nexus's type system, ports/handlers, or linear types

## When NOT to Use This Skill

- Non-Nexus programming tasks

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
let %arr = [| 1, 2, 3 |]       // linear — must be consumed exactly once
let view = &%arr                 // borrow — immutable view, does not consume
view[0] <- 42                   // mutation via borrow
let ~x = 10                     // mutable ref, stack-confined
~x <- 20                        // reassignment
let v = ~x                      // dereference
```

### 5. Lazy evaluation with `@` (parallel execution)
```nexus
let @result = expensive_call(data: input)  // deferred, runs in parallel
// ... other work ...
let val = @result                          // force: blocks until ready
```

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
import { MyType } from "src/common/foo.nx"

// FFI binding — declares the WIT interface for subsequent `external` decls
import external "std:string-ops"
external __nx_string_length = "__nx_string_length" : (s: string) -> i64
```

Path forms:
| Form | Resolves to | Use |
|------|-------------|-----|
| `"std:stdio"` | `nxlib/stdlib/stdio.nx`, WIT `nexus:std/stdio` | Standard library module |
| `"pkg:path/module"` | `<pkg-root>/path/module.nx` | Third-party package module |
| `"src/foo.nx"` | Relative file path | Project-local module |
| `import external "std:<iface>"` | WIT interface `nexus:std/<iface>` | Pin FFI imports to a WIT interface |

The `std` package always maps to `nxlib/stdlib/`. Underscore in the file stem stays in the import path (`std:string_ops`), but the WIT interface uses kebab-case (`nexus:std/string-ops`) — convert `_` to `-` for `import external` declarations.

## Effect System (Caps & Handlers)

Nexus uses coeffects for dependency injection, NOT algebraic effects. See https://nexus-llm-lang.github.io/Nexus/latest/spec/effects for details.

```nexus
// 1. Define a cap (interface)
cap Logger do
  fn info(msg: string) -> unit
  fn error(msg: string) -> unit
end

// 2. Implement via handler
let console_logger = handler Logger require { Console } do
  fn info(msg: string) -> unit do
    Console.println(val: "[INFO] " ++ msg)
    return ()
  end
  fn error(msg: string) -> unit do
    Console.println(val: "[ERROR] " ++ msg)
    return ()
  end
end

// 3. Require cap in functions
let greet = fn (name: string) -> unit require { Logger } do
  Logger.info(msg: "Hello, " ++ name)
  return ()
end

// 4. Inject handler at call site
let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    inject console_logger do
      greet(name: "World")
    end
  end
  return ()
end
```

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
| Lazy | `@T` | Deferred thunk, forced with `@x` (parallel) |
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

### Error Handling
```nexus
import { Result, Ok, Err } from "std:result"

exception NotFound(msg: string)

// Narrow `throws` row — list exactly what may escape, not the catch-all `Exn`
let find_user = fn (id: i64) -> string throws { NotFound } do
  if id == 1 then
    return "Alice"
  else
    raise NotFound(msg: "User not found")
  end
end

// Catching exceptions
try
  let name = find_user(id: 42)
  Console.println(val: name)
catch
  | NotFound(msg: m) -> Console.println(val: m)
  | _ -> Console.println(val: "Unknown error")
end
```

For multi-exception phases, declare an `exception group` and use the group name in the row:

```nexus
exception UnexpectedToken(span: Span)
exception UnexpectedEof(span: Span)
exception group ParseError = UnexpectedToken | UnexpectedEof

let parse_top = fn (toks: [Token]) -> Decl throws { ParseError } do
  // raises either UnexpectedToken or UnexpectedEof
end
```

### List Recursion
```nexus
import * as list from "std:list"

let sum = fn (xs: [ i64 ]) -> i64 do
  match xs do
    | [] -> return 0
    | v :: rest -> return v + sum(xs: rest)
  end
end

// With fold
let sum2 = fn (xs: [ i64 ]) -> i64 do
  return list.fold_left(xs: xs, init: 0, f: fn (acc: i64, val: i64) -> i64 do
    return acc + val
  end)
end
```

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
  raise UnexpectedToken(expected: "string literal", ...)
end

// Expression form
let arr_elem = if let TyArray(elem: e) = arr_t then e else TyI64 end

// AVOID — two-arm match on a single binding constructor
match peek_token(toks: toks) do
  | TkString(val: s) -> return ParsedIdent(name: s, rest: skip(toks: toks))
  | _ -> raise UnexpectedToken(expected: "string literal", ...)
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

### 4. Or-patterns to share an arm body across alternatives
When two arms run the same body, fuse them with `|` instead of duplicating. All alternatives must bind the same variable names with compatible types.

```nexus
// PREFER — one arm, two ctors share the body
match sign do
  | Pos | Neg -> 1
  | Zero -> 0
end

// AVOID — duplicated body
match sign do
  | Pos -> 1
  | Neg -> 1
  | Zero -> 0
end
```

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
  raise UnexpectedToken(span: ...)
end

let load_config = fn (path: string) -> Config require { Fs } throws { FileNotFound, ParseError } do
  ...
end

// AVOID — Exn admits any exception, including ones the caller can't reasonably catch
let parse_top = fn (toks: [Token]) -> Decl throws { Exn } do
  raise UnexpectedToken(span: ...)
end
```

When a phase grows several distinct exceptions, declare a group and reference it instead of repeating the alternatives at every call site:

```nexus
exception group HirError = InvalidSymbolTag | EmptyScopeStack | NoActiveScope

let lower_to_hir = fn (...) -> HirProgram throws { HirError } do ...
```

Partial functions (operations defined only on a subset of inputs — `head`, `tail`, `unwrap`, `to_i64`) **must raise a domain-specific exception**, not `RuntimeError(val: "...")`. Catch-all error strings discard the structured information callers need to recover. Declare a real `exception` per failure mode:

```nexus
// PREFER
exception EmptyList(op: string)
let head = fn <T>(xs: [T]) -> T throws { EmptyList } do
  match xs do
    | [] -> raise EmptyList(op: "head")
    | v :: _ -> return v
  end
end

// AVOID — RuntimeError is the dumping ground; callers can't pattern-match on intent
let head = fn <T>(xs: [T]) -> T throws { Exn } do
  match xs do
    | [] -> raise RuntimeError(val: "head: empty list")
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
| `for x in list` | Use `match`/recursion or `list.fold_left` |
| `throws { Exn }` for a function with a known exception set | List the actual exceptions or an `exception group` (see rule #7) |
| `raise RuntimeError(val: "...")` from a partial function | Declare a domain-specific `exception` and raise that |
| `let r = ...; let r2 = ...; let r3 = ...` when intermediates are dead | Shadow: reuse `r` (see rule #6) |

## Reference Files

- https://nexus-llm-lang.github.io/Nexus/latest/spec/syntax — Syntax and EBNF grammar
- https://nexus-llm-lang.github.io/Nexus/latest/spec/types — Type system, linear types, borrowing
- https://nexus-llm-lang.github.io/Nexus/latest/spec/effects — Caps, handlers, inject, permissions
- https://nexus-llm-lang.github.io/Nexus/latest/env/stdlib — Standard library API reference
- `./references/stdlib.md` — `std` package module index, capability permissions, WIT naming
- `./references/patterns.md` — Idiomatic code patterns with examples
- `./templates/` — Starter templates for common program structures
