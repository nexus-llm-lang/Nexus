# Syntax

Nexus uses keyword-terminated blocks and mandatory labeled arguments to make program structure unambiguous (see [Design](../design.md)). This reference covers all syntactic constructs. For semantic behavior, see [Types](types.md), [Effects](effects.md), and [Semantics](semantics.md).

## Comments

```nexus
// line comment
/* block comment */
```

## Definitions

### Functions

Functions are first-class values bound with `let`:

```nexus
pub let add = fn (a: i64, b: i64) -> i64 do
    return a + b
end
```

- `pub let` makes the function visible to other modules
- All arguments are **labeled** at call sites: `add(a: 1, b: 2)`
- Generic type parameters: `fn <T>(x: T) -> T do ... end`

### Coeffect/Effect Annotations

```nexus
let greet = fn (name: string) -> unit require { Logger, Console } do
    Logger.log(msg: name)
    Console.println(val: name)
    return ()
end
```

Both `require { ... }` and `effect { ... }` are optional; omitted means empty row.

### External Functions

Foreign function declarations bind a Wasm export to a Nexus name:

```nexus
pub external sin = "sin" : (x: float) -> float
pub external length = "array_length" : <T>(arr: &[| T |]) -> i64
```

Generic externals require explicit type parameters with `<T, U, ...>`.

### Type Definitions

```nexus
pub type Point = { x: float, y: float }
pub type Pair<A, B> = { fst: A, snd: B }
pub type Result<T, E> = Ok(val: T) | Err(err: E)
```

Defines either a record type (`{ ... }`) or a sum type (`A(...) | B(...)`).

### Exception Declarations

```nexus
pub exception NotFound(msg: string)
pub exception PermissionDenied(msg: string, code: i64)
```

Extends the builtin `Exn` type with new constructors.

## Expressions

### Literals

| Form | Example | Type |
|---|---|---|
| Integer | `42`, `-7` | `i64` |
| Float | `3.14`, `-0.5` | `f64` |
| Boolean | `true`, `false` | `bool` |
| Unit | `()` | `unit` |
| String | `"hello"` | `string` |

Strings use `"..."` delimiters with escape sequences (`\n`, `\t`, `\\`, `\"`). Raw strings use `[=[ ... ]=]` delimiters (no escape processing).

### Operators

Binary operators with standard precedence (multiplicative binds tighter than additive, comparison, then logical):

| Operators | Domain |
|---|---|
| `*` `/` `*.` `/.` | Multiplicative |
| `+` `-` `+.` `-.` `++` | Additive / string concat |
| `==` `!=` `<` `>` `<=` `>=` | Integer / generic comparison |
| `==.` `!=.` `<.` `>.` `<=.` `>=.` | Float comparison |
| `&&` | Logical AND |
| `\|\|` | Logical OR |

### Function Calls

```nexus
add(a: 1, b: 2)
Console.println(val: "hello")
list.map(xs: items, f: transform)
```

All arguments are labeled. Port method calls use `Port.method(...)` syntax.

### Lambda Expressions

```nexus
let f = fn (x: i64) -> i64 do
    return x + 1
end
```

### Handler Expressions

```nexus
let logger = handler Logger require { Console } do
    fn info(msg: string) -> unit do
        Console.println(val: msg)
        return ()
    end
end
```

### Match Expression

Match can be used in expression position. Each case body produces a value:

```nexus
let code = match color do
    case Red -> 1
    case Green -> 2
    case Blue -> 3
end
```

All case bodies must produce the same type. Cases with `return` statements diverge and do not contribute to the unified type.

### Record and Constructor Expressions

```nexus
let point = { x: 1.0, y: 2.0 }
let result = Ok(val: 42)
```

### Collection Literals

```nexus
let xs = [1, 2, 3]            // list
let %arr = [| 1, 2, 3 |]      // array (linear)
```

### Borrow Expression

```nexus
let b = &arr                   // borrow immutable binding
let b2 = &%resource            // borrow linear binding
```

### Index and Field Access

```nexus
let val = (&%arr)[0]           // array index
%arr[0] <- 42                  // array index assignment
let name = user.name           // record field access
```

### Raise Expression

```nexus
raise NotFound(msg: "key")
```

## Statements

### Let Bindings

```nexus
let x = 10
let name: string = "Nexus"
let ~counter: i64 = 0
let %resource = acquire()
let &view = ~data
```

Sigils: (none) immutable, `~` mutable, `%` linear, `&` borrowed.

Top-level `let` does not allow `~` or `%` sigils.

### Assignment

```nexus
~x <- 42
```

Assigns to a mutable binding.

### Return

```nexus
return expr
```

### If-Else

```nexus
if condition then
    // ...
else
    // ...
end
```

The `else` branch is optional.

### Match

```nexus
match result do
    case Ok(val: v) -> process(v: v)
    case Err(err: e) -> handle_error(e: e)
end
```

### While Loop

```nexus
while condition do
    // body
end
```

Evaluates `condition` before each iteration. If false, exits the loop. Returns `unit`.

```nexus
let ~i = 0
while ~i < 10 do
    ~i <- ~i + 1
end
```

### For Loop

```nexus
for var = start to end_expr do
    // body (var is immutable i64, scoped to body)
end
```

Iterates `var` from `start` (inclusive) to `end_expr` (exclusive). Both must be `i64`. The loop variable is immutable within the body and increments by 1 each iteration. Returns `unit`.

```nexus
let ~sum = 0
for i = 0 to 5 do
    ~sum <- ~sum + i
end
// ~sum is 10
```

### Try / Catch

```nexus
try
    risky_operation()
catch err ->
    match err do
        case NotFound(msg: m) -> ()
        case _ -> ()
    end
end
```

Single `catch` clause binding the `Exn` value.

### Inject

```nexus
inject stdio.system_handler do
    inject console_logger do
        program()
    end
end
```

Multiple handlers: `inject h1, h2 do ... end`.

### Conc

```nexus
conc do
    task worker1 do
        // ...
    end
    task worker2 do
        // ...
    end
end
```

## Patterns

| Pattern | Example | Description |
|---|---|---|
| Literal | `1`, `true`, `"hi"` | Matches exact value |
| Variable | `x`, `~x`, `%x` | Binds with optional sigil |
| Constructor | `Ok(val: v)`, `None()` | Destructures variant |
| Record (exact) | `{ x: p1, y: p2 }` | All fields must match |
| Record (partial) | `{ x: p1, _ }` | `_` must be last; remaining fields ignored |
| Wildcard | `_` | Matches anything, no binding |

## Imports

```nexus
import from path/to/module.nx                        // anonymous
import as math from path/to/math.nx                  // namespace alias
import { add, sub } from path/to/math.nx             // named items
import { add, sub }, * as math from path/to/math.nx  // named + namespace
import external path/to/lib.wasm                     // Wasm module
```

## Ports and Handlers

```nexus
pub port Logger do
    fn info(msg: string) -> unit
    fn warn(msg: string) -> unit
end

let console_logger = handler Logger require { Console } do
    fn info(msg: string) -> unit do
        Console.println(val: msg)
        return ()
    end
    fn warn(msg: string) -> unit do
        Console.println(val: msg)
        return ()
    end
end
```

## Concurrency

```nexus
conc do
    task name1 do
        // ...
    end
    task name2 do
        // ...
    end
end
```

Task names are identifiers. `conc` blocks wait for all tasks.

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

type_def      ::= [ "pub" ] [ "opaque" ] "type" UIDENT [ type_params ] "=" record_type
                | [ "pub" ] [ "opaque" ] "type" UIDENT [ type_params ] "=" type_sum_def

type_sum_def  ::= variant_def ( "|" variant_def )*
variant_def   ::= UIDENT [ "(" variant_field ( "," variant_field )* ")" ]
variant_field ::= type | IDENT ":" type
exception_def ::= [ "pub" ] "exception" UIDENT [ "(" variant_field ( "," variant_field )* ")" ]

import_def    ::= "import" "external" import_path
                | "import" "{" IDENT ( "," IDENT )* "}" [ "," "*" "as" IDENT ] "from" import_path
                | "import" "as" IDENT "from" import_path
                | "import" "from" import_path
import_path   ::= ( ALPHA | DIGIT | "_" | "-" | "/" | "." )+

port_def      ::= [ "pub" ] "port" UIDENT "do" fn_signature* "end"
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
                | while_stmt
                | for_stmt
                | comment
                | expr_stmt

let_stmt      ::= "let" [ sigil ] IDENT [ ":" type ] "=" expr
return_stmt   ::= "return" expr
assign_stmt   ::= expr "<-" expr

if_stmt       ::= "if" expr "then" stmt* [ "else" stmt* ] "end"

match_stmt    ::= "match" expr "do" match_case* "end"
match_case    ::= "case" pattern "->" stmt*

try_stmt      ::= "try" stmt* "catch" IDENT "->" stmt* "end"
inject_stmt   ::= "inject" dotted_ident ( "," dotted_ident )* "do" stmt* "end"

conc_stmt     ::= "conc" "do" task_def* "end"
task_def      ::= "task" IDENT [ "effect" effect_type ] "do" stmt* "end"

while_stmt    ::= "while" expr "do" stmt* "end"
for_stmt      ::= "for" IDENT "=" expr "to" expr "do" stmt* "end"

expr_stmt     ::= expr

(* ── Expressions (precedence: low → high) ───────────────────── *)

expr          ::= expr binary_op expr     (* left-associative *)
                | match_expr
                | postfix_expr

binary_op     ::=                         (* logical — lowest *)
                  "||"
                | "&&"
                |                         (* comparison *)
                  "==" | "!=" | "<=" | ">=" | "<" | ">"
                | "==." | "!=." | "<=." | ">=." | "<." | ">."
                |                         (* additive *)
                  "+" | "-" | "++"
                | "+." | "-."
                |                         (* multiplicative — highest *)
                  "*" | "/"
                | "*." | "/."

postfix_expr  ::= postfix_expr "." IDENT  (* field access *)
                | postfix_expr "[" expr "]"  (* index *)
                | atom_expr

match_expr    ::= "match" expr "do" match_case_expr* "end"
match_case_expr ::= "case" pattern "->" expr

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
                      "do" stmt* "end"
handler_expr      ::= "handler" UIDENT [ "require" row_type ] "do" handler_fn* "end"
handler_fn        ::= "fn" IDENT [ type_params ] "(" [ param ( "," param )* ] ")"
                      "->" type [ "require" effect_type ] [ "effect" effect_type ]
                      "do" stmt* "end"
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
string_literal    ::= '"' string_char* '"'
                    | "[=[" raw_char* "]=]"
string_char       ::= '\"'               (* escaped double quote *)
                    | '\n' | '\t' | '\\'  (* escape sequences *)
                    | NON_QUOTE NON_BACKSLASH
raw_char          ::= ANY - "]=]"

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
