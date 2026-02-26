# Syntax

Nexus uses a clean, keyword-oriented syntax designed for readability and clarity for both humans and AI.

## Comments

Nexus supports C-style comments:

```nexus
// line comment
/* block comment */
```

## Functions

Functions are first-class values in Nexus. At the top level, they are defined by binding a lambda expression to a name using `let`.

```nexus
pub let add = fn (a: i64, b: i64) -> i64 do
  return a + b
endfn
```

- `pub let` makes the function visible to other modules.
- All arguments are **labeled** at call sites: `add(a: 1, b: 2)`.
- Generic type parameters can be declared with `<T, U>`:

```nexus
let identity = fn <T>(x: T) -> T do
  return x
endfn
```

### Function values and lambdas

```nexus
let add1 = fn (x: i64) -> i64 do
  return x + 1
endfn

let main = fn () -> unit do
  let f = add1
  let g = fn (x: i64) -> i64 do
    return x * 2
  endfn
  let _ = g(x: f(x: 10))
  return ()
endfn
```

- Local lambda literals use `fn (...) -> T do ... endfn`.
- A local recursive lambda must use an immutable binding with an explicit type annotation.

### Coeffect/effect annotation

Functions may declare:

- `require { ... }` for coeffects (ports).
- `effect { ... }` for builtin effects.

Both are optional; omitted means an empty row.

```nexus
let greet = fn (name: string) -> unit require { Logger } effect { Console } do
  Logger.log(msg: name)
  print(val: name)
  return ()
endfn
```

### External functions

Foreign functions are declared with the `external` keyword as a dedicated top-level statement.
The Wasm export name is given as a string after `=`, followed by `:` and the type:

```nexus
pub external sin = [=[sin]=] : (x: float) -> float
pub external add_ints = [=[add]=] : (a: i64, b: i64) -> i64
pub external length = [=[array_length]=] : <T>(arr: &[| T |]) -> i64
```

Generic external bindings require explicit type parameters with `<T, U, ...>` before the arrow type.
Using an unknown type name without declaring it as a type parameter is a type error.

## Variables and Constants (`let`)

Variables are defined with `let`. A type annotation is optional.

### Top-level `let`

At the top level, `let` defines a module-level constant or global variable. Sigils (`~`, `%`) are **not allowed** at the top level.

```nexus
pub let PI = 3.14159
let internal_config = [=[ debug ]=]
```

- `pub let` makes the value visible to other modules.

### Local `let`

Inside functions, a **sigil** controls linearity and mutability:

| Sigil | Meaning |
|---|---|
| (none) | Immutable binding |
| `~` | Mutable binding |
| `%` | Linear binding (must be consumed exactly once) |
| `&` | Borrowed reference (read-only access without consumption) |


```nexus
let main = fn() -> unit do
  let x = 10
  let name: string = [=[ Nexus ]=]
  let ~counter: i64 = 0
  let %resource: %Handle = acquire()
  // ...
  return ()
endfn
```

Variables are lexically scoped.

## Operators

### Binary operators

All binary operators are left-associative. Precedence from lowest to highest:

| Level | Operators | Domain |
|---|---|---|
| 1 | `==` `!=` `<=` `>=` `<` `>` | Integer / generic comparison |
| 1 | `==.` `!=.` `<=.` `>=.` `<.` `>.` | Float comparison |
| 2 | `+` `-` | Integer arithmetic |
| 2 | `++` | String concatenation |
| 2 | `+.` `-.` | Float arithmetic |
| 3 | `*` `/` | Integer arithmetic |
| 3 | `*.` `/.` | Float arithmetic |

### Assignment

```nexus
~x <- 42
```

Assigns a new value to a mutable binding. The left-hand side is any expression that resolves to a mutable location.

## Types

### Primitive types

| Type | Description |
|---|---|
| `i32` | 32-bit signed integer |
| `i64` | 64-bit signed integer |
| `f32` | 32-bit floating-point |
| `f64` | 64-bit floating-point |
| `float` | Alias of `f64` |
| `bool` | Boolean (`true` / `false`) |
| `string` | Immutable UTF-8 string |
| `unit` | The unit type, written `()` as a value |

### Composite types

| Syntax | Description |
|---|---|
| `{ x: T, y: U }` | Record type |
| `[T]` | Immutable list |
| `[| T |]` | Linear array |
| `Name<T, U>` | Generic user-defined type |
| `Result<T, E>` | `Result` sum type from stdlib |

### Pointer / ownership types

| Syntax | Description |
|---|---|
| `ref(T)` | Mutable reference |
| `&T` | Borrowed reference |
| `%T` | Linear type (must be consumed once) |

### Function / signature types

```nexus
(label: T) -> R                       // pure function
(a: i64, b: i64) -> i64               // labeled parameters
() -> unit effect { Console }         // builtin effects
() -> string require { Net }          // coeffect requirements
(x: T) -> R require { C | r } effect { E | e } // open rows
```

An unlabeled parameter uses `_` as an internal label.

### Signature rows

`require`/`effect` rows use `{}` with optional tail variables:

```nexus
{ Console }             // single entry
{ Net, Fs }             // multiple entries
{ Console | e }         // open row with tail variable
```

### Type definitions

```nexus
pub type Point = { x: float, y: float }
pub type Pair<A, B> = { fst: A, snd: B }
pub type Result<T, E> = Ok(val: T) | Err(err: E)
```

- `pub` makes the type visible to other modules.
- `type` can define either:
    - A record type (`{ ... }`)
    - A sum/variant type (`A(...) | B(...)`)

> **Note:** Constructors with no fields still require `()` in patterns and expressions (e.g., `Red()`).

### Exception declarations

`exception` declarations extend the built-in `Exn` type with constructors.

```nexus
pub exception NotFound(msg: string)
pub exception PermissionDenied(msg: string, code: i64)
```

- `pub` makes the exception constructor visible to other modules.

## Control Flow

### If-Else

```nexus
if condition then
  // ...
else
  // ...
endif
```

The `else` branch is optional.

### Match

Pattern matching is supported via `match`.

```nexus
match result do
  case Ok(val) ->
    // ...
  case Err(e) ->
    // ...
endmatch
```

Supported patterns:

| Pattern | Example | Description |
|---|---|---|
| Literal | `1`, `true`, `[=[ hi ]=]` | Matches an exact value |
| Constructor | `Ok(x)`, `Err(e)` | Destructures a sum-type variant |
| Record (exact) | `{ x: p1, y: p2 }` | All fields must match |
| Record (partial) | `{ x: p1, _ }` | Remaining fields ignored; `_` must be last |
| Wildcard | `_` | Matches anything without binding |
| Variable | `x`, `~x`, `%x` | Binds the matched value with optional sigil |

### Try / Catch

```nexus
try
  let result = risky_operation()
  return result
catch err ->
  match err do
    case NotFound(msg) -> print(val: msg)
  endmatch
  return ()
endtry
```

`catch` binds the raised `Exn` value to a single identifier. Only one catch clause is allowed.

### Raise

```nexus
raise NotFound(msg: [=[ something went wrong ]=])
```

`raise` is an expression. It terminates the current computation and propagates to the nearest `catch`.
Runtime errors are represented as built-in exceptions such as `RuntimeError(string)` and
`InvalidIndex(i64)` and can be handled in `catch`.

### Exception constructors

Exception values are created with normal constructor syntax:

```nexus
let err = PermissionDenied(msg: [=[/tmp/data]=], code: 13)
raise err
```

## Function Calls and Signatures

Function calls are direct:

```nexus
print(val: [=[ hello ]=])
Logger.log(msg: text)
```

The type checker validates each call against the surrounding `require` and `effect` signatures.

## Borrow

The `&` sigil creates a borrowed reference to a binding without consuming it. It can be used as a prefix operator in expressions or as a sigil in `let` bindings:

```nexus
let borrowed = &arr         // &immutable
let &b2 = ~x                // &mutable using let sigil
let b3 = &%resource         // &linear using prefix operator
```

> **Note:** `&` serves both as a prefix operator (`&%arr`) and as a let-binding sigil (`let &b = ...`).

## Ports and Handlers

### Port

A `port` defines a coeffect interface.

```nexus
pub port Logger do
  fn log(msg: string) -> unit
  fn warn(msg: string) -> unit
endport
```

### Handler and Inject

A handler is an expression that implements one port.

```nexus
let stdout_logger = handler Logger do
  fn log(msg: string) -> unit effect { Console } do
    print(val: msg)
    return ()
  endfn
  fn warn(msg: string) -> unit effect { Console } do
    print(val: msg)
    return ()
  endfn
endhandler

inject stdout_logger do
  Logger.log(msg: [=[from handler]=])
endinject
```

## Imports

Three import forms are available:

```nexus
import from path/to/module.nx              // anonymous import
import as math from path/to/math.nx        // namespace alias
import { add, sub } from path/to/math.nx   // named items
import external path/to/lib.wasm           // Wasm module
```

## Concurrency (`conc`)

Structured concurrency via `conc` blocks. Task names are identifiers:

```nexus
conc do
  task worker1 do
    // ...
  endtask
  task worker2 do
    // ...
  endtask
endconc
```

`conc` waits for all tasks to complete.

> **Note:** In the current reference interpreter, tasks execute sequentially for deterministic debugging.

## Literals

| Form | Example | Type |
|---|---|---|
| Integer | `42`, `-7` | `i64` |
| Float | `3.14`, `-0.5` | `f64` |
| Boolean | `true`, `false` | `bool` |
| Unit | `()` | `unit` |
| String | `[=[ hello ]=]` | `string` |

Strings use `[=[ ... ]=]` delimiters. To include `]=]` literally, escape it as `\]=]`.
Numeric literals default to `i64` (integers) and `f64` (floats) unless constrained by annotations or context.

Bracket strings are also used for import paths and Wasm binding names.

---

## Full Syntax (EBNF)

```ebnf
(* ── Top-level ─────────────────────────────────────────────── *)

program       ::= top_level*
top_level     ::= type_def
                | exception_def
                | import_def
                | port_def
                | external_def
                | let_def
                | comment

(* ── Definitions ───────────────────────────────────────────── *)

type_def      ::= [ "pub" ] "type" UIDENT [ type_params ] "=" record_type
                | [ "pub" ] "type" UIDENT [ type_params ] "=" type_sum_def

type_sum_def  ::= variant_def ( "|" variant_def )*
variant_def   ::= UIDENT [ "(" variant_field ( "," variant_field )* ")" ]
variant_field ::= type | IDENT ":" type
exception_def ::= [ "pub" ] "exception" UIDENT [ "(" variant_field ( "," variant_field )* ")" ]

import_def    ::= "import" "external" import_path
                | "import" "{" IDENT ( "," IDENT )* "}" "from" import_path
                | "import" "as" IDENT "from" import_path
                | "import" "from" import_path
import_path   ::= ( ALPHA | DIGIT | "_" | "-" | "/" | "." )+

port_def      ::= [ "pub" ] "port" UIDENT "do" fn_signature* "endport"
fn_signature  ::= "fn" IDENT param_list "->" type [ "require" effect_type ] [ "effect" effect_type ]

let_def       ::= [ "pub" ] "let" IDENT [ ":" type ] "=" expr
external_def  ::= [ "pub" ] "external" IDENT "=" STRING_LITERAL ":" [ type_params ] arrow_type

(* ── Parameters ─────────────────────────────────────────────── *)

type_params   ::= "<" UIDENT ( "," UIDENT )* ">"
param_list    ::= "(" [ param ( "," param )* ] ")"
param         ::= [ sigil ] IDENT ":" type
sigil         ::= "~" | "%"

(* ── Types ──────────────────────────────────────────────────── *)

type          ::= arrow_type
                | generic_type
                | primitive_type
                | ref_type
                | borrow_type
                | linear_type
                | record_type
                | list_type
                | array_type
                | row_type
                | UIDENT              (* type variable or monotype *)

primitive_type ::= "i32" | "i64" | "f32" | "f64" | "float" | "bool" | "string" | "unit"

ref_type      ::= "ref" "(" type ")"
borrow_type   ::= "&" type
linear_type   ::= "%" type

record_type   ::= "{" IDENT ":" type ( "," IDENT ":" type )* "}"

list_type     ::= "[" type "]"
array_type    ::= "[|" type "|]"

generic_type  ::= UIDENT "<" type ( "," type )* ">"

row_type      ::= "{" type ( "," type )* [ "|" type ] "}"
                  (* used as require/effect rows *)

arrow_type    ::= "(" [ arrow_param ( "," arrow_param )* ] ")"
                  "->" type [ "require" effect_type ] [ "effect" effect_type ]
arrow_param   ::= IDENT ":" type | type

effect_type   ::= row_type | generic_type | UIDENT

(* ── Statements ─────────────────────────────────────────────── *)

stmt          ::= let_stmt
                | return_stmt
                | assign_stmt
                | if_stmt
                | match_stmt
                | try_stmt
                | inject_stmt
                | conc_stmt
                | comment
                | expr_stmt

let_stmt      ::= "let" [ sigil ] IDENT [ ":" type ] "=" expr
return_stmt   ::= "return" expr
assign_stmt   ::= expr "<-" expr

if_stmt       ::= "if" expr "then" stmt* [ "else" stmt* ] "endif"

match_stmt    ::= "match" expr "do" match_case* "endmatch"
match_case    ::= "case" pattern "->" stmt*

try_stmt      ::= "try" stmt* "catch" IDENT "->" stmt* "endtry"
inject_stmt   ::= "inject" IDENT ( "," IDENT )* "do" stmt* "endinject"

conc_stmt     ::= "conc" "do" task_def* "endconc"
task_def      ::= "task" IDENT [ "effect" effect_type ] "do" stmt* "endtask"

expr_stmt     ::= expr

(* ── Expressions (precedence: low → high) ───────────────────── *)

expr          ::= expr binary_op expr     (* left-associative *)
                | postfix_expr

binary_op     ::=                         (* precedence level 1 — comparison *)
                  "==" | "!=" | "<=" | ">=" | "<" | ">"
                | "==." | "!=." | "<=." | ">=." | "<." | ">."
                |                         (* precedence level 2 — additive *)
                  "+" | "-" | "++"
                | "+." | "-."
                |                         (* precedence level 3 — multiplicative *)
                  "*" | "/"
                | "*." | "/."

postfix_expr  ::= postfix_expr "." IDENT  (* field access *)
                | postfix_expr "[" expr "]"  (* index *)
                | atom_expr

atom_expr     ::= "(" expr ")"
                | raise_expr
                | borrow_expr
                | lambda_expr
                | handler_expr
                | call_expr
                | constructor_expr
                | record_expr
                | array_expr
                | list_expr
                | literal
                | variable

raise_expr        ::= "raise" expr
borrow_expr       ::= "&" [ sigil ] IDENT
lambda_expr       ::= "fn" [ type_params ] "(" [ param ( "," param )* ] ")"
                      "->" type [ "require" effect_type ] [ "effect" effect_type ]
                      "do" stmt* "endfn"
handler_expr      ::= "handler" UIDENT "do" handler_fn* "endhandler"
handler_fn        ::= "fn" IDENT [ type_params ] "(" [ param ( "," param )* ] ")"
                      "->" type [ "require" effect_type ] [ "effect" effect_type ]
                      "do" stmt* "endfn"
call_expr         ::= dotted_ident "(" [ labeled_arg ( "," labeled_arg )* ] ")"
labeled_arg       ::= IDENT ":" expr
constructor_expr  ::= UIDENT "(" [ ctor_arg ( "," ctor_arg )* ] ")"
ctor_arg          ::= [ IDENT ":" ] expr
record_expr       ::= "{" [ IDENT ":" expr ( "," IDENT ":" expr )* ] "}"
list_expr         ::= "[" [ expr ( "," expr )* [ "," ] ] "]"
array_expr        ::= "[|" [ expr ( "," expr )* [ "," ] ] "|]"
variable          ::= [ sigil ] IDENT
dotted_ident      ::= IDENT ( "." IDENT )*

(* ── Patterns ───────────────────────────────────────────────── *)

pattern           ::= literal_pattern
                    | constructor_pattern
                    | record_pattern
                    | wildcard_pattern
                    | variable_pattern

literal_pattern   ::= literal
variable_pattern  ::= [ sigil ] IDENT
constructor_pattern
                  ::= UIDENT "(" [ ctor_pat_arg ( "," ctor_pat_arg )* ] ")"
ctor_pat_arg      ::= [ IDENT ":" ] pattern
record_pattern    ::= "{" [ rec_pat_field ( "," rec_pat_field )* [ "," ] ] "}"
rec_pat_field     ::= "_" | IDENT ":" pattern
                      (* "_" must be the last element; enables partial matching *)
wildcard_pattern  ::= "_"

(* ── Literals ───────────────────────────────────────────────── *)

literal           ::= float_literal | integer_literal | boolean_literal
                    | unit_literal  | string_literal

float_literal     ::= [ "-" ] DIGIT+ "." DIGIT+
integer_literal   ::= [ "-" ] DIGIT+
boolean_literal   ::= "true" | "false"
unit_literal      ::= "()"
string_literal    ::= "[=[" string_char* "]=]"
string_char       ::= "\]=]"               (* escaped terminator *)
                    | "\" NON_CLOSE_BRACKET
                    | NON_BRACKET NON_BACKSLASH

(* ── Comments & Terminals ───────────────────────────────────── *)

comment           ::= line_comment | block_comment
line_comment      ::= "//" ANY* ( NEWLINE | EOF )
block_comment     ::= "/*" ANY* "*/"

STRING_LITERAL    ::= string_literal
                      (* used in: string values, import paths, external wasm names *)

IDENT             ::= [a-z_] [a-zA-Z0-9_]*       (* not a keyword *)
UIDENT            ::= [A-Z] [a-zA-Z0-9_]*       (* constructor names *)
DIGIT             ::= [0-9]
```
