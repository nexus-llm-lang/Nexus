# Nexus Syntax Reference

## Comments
```nexus
// line comment
/* block comment */
/// doc comment (convention)
/** multi-line doc comment */
```

## Complete EBNF Grammar

```ebnf
(* ── Top-level ─────────────────────────────────────────────── *)

program     ::= top_level*
top_level   ::= type_def
        | exception_def
        | import_def
        | port_def
        | external_def
        | let_def
        | comment

(* ── Definitions ───────────────────────────────────────────── *)

type_def    ::= [ "export" ] [ "opaque" ] "type" UIDENT [ type_params ] "=" record_type
        | [ "export" ] [ "opaque" ] "type" UIDENT [ type_params ] "=" type_sum_def

type_sum_def  ::= variant_def ( "|" variant_def )*
variant_def   ::= UIDENT [ "(" variant_field ( "," variant_field )* ")" ]
variant_field ::= type | IDENT ":" type
exception_def ::= [ "export" ] "exception" UIDENT [ "(" variant_field ( "," variant_field )* ")" ]

import_def  ::= "import" "external" import_path
        | "import" "{" IDENT ( "," IDENT )* "}" [ "," "*" "as" IDENT ] "from" import_path
        | "import" "*" "as" IDENT "from" import_path
        | "import" "from" import_path
import_path   ::= ( ALPHA | DIGIT | "_" | "-" | "/" | "." )+

port_def    ::= [ "export" ] "port" UIDENT "do" fn_signature* "end"
fn_signature  ::= "fn" IDENT param_list "->" type [ "require" throws_type ] [ "throws" throws_type ]

let_def     ::= [ "export" ] "let" IDENT [ ":" type ] "=" expr
external_def  ::= [ "export" ] "external" IDENT "=" STRING_LITERAL ":" [ type_params ] arrow_type

(* ── Parameters ─────────────────────────────────────────────── *)

type_params   ::= "<" UIDENT ( "," UIDENT )* ">"
param_list  ::= "(" [ param ( "," param )* ] ")"
param     ::= [ sigil ] IDENT ":" type
sigil     ::= "~" | "%"

(* ── Types ──────────────────────────────────────────────────── *)

type      ::= arrow_type
        | generic_type
        | primitive_type
        | ref_type
        | borrow_type
        | linear_type
        | record_type
        | list_type
        | array_type
        | row_type
        | UIDENT        (* type variable or monotype *)

primitive_type ::= "i32" | "i64" | "f32" | "f64" | "float" | "bool" | "char" | "string" | "unit"

ref_type    ::= "ref" "(" type ")"
borrow_type   ::= "&" type
linear_type   ::= "%" type

record_type   ::= "{" IDENT ":" type ( "," IDENT ":" type )* "}"

list_type   ::= "[" type "]"
array_type  ::= "[|" type "|]"

generic_type  ::= UIDENT "<" type ( "," type )* ">"

row_type    ::= "{" type ( "," type )* [ "|" type ] "}"

arrow_type  ::= "(" [ arrow_param ( "," arrow_param )* ] ")"
          "->" type [ "require" throws_type ] [ "throws" throws_type ]
arrow_param   ::= IDENT ":" type | type

throws_type   ::= row_type | generic_type | UIDENT

(* ── Statements ─────────────────────────────────────────────── *)

stmt      ::= let_stmt
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

let_stmt    ::= "let" [ sigil ] IDENT [ ":" type ] "=" expr
              | "let" pattern "=" expr
return_stmt   ::= "return" expr
assign_stmt   ::= expr "<-" expr

if_stmt     ::= "if" expr "then" stmt* [ "else" stmt* ] "end"

match_stmt  ::= "match" expr "do" match_case* "end"
match_case  ::= "case" pattern "->" stmt*

try_stmt    ::= "try" stmt* "catch" IDENT "->" stmt* "end"
inject_stmt   ::= "inject" dotted_ident ( "," dotted_ident )* "do" stmt* "end"

conc_stmt   ::= "conc" "do" task_def* "end"
task_def    ::= "task" IDENT [ "throws" throws_type ] "do" stmt* "end"

while_stmt  ::= "while" expr "do" stmt* "end"
for_stmt    ::= "for" IDENT "=" expr "to" expr "do" stmt* "end"

expr_stmt   ::= expr

(* ── Expressions (precedence: low → high) ───────────────────── *)

expr      ::= expr binary_op expr   (* left-associative *)
        | match_expr
        | postfix_expr

binary_op   ::=             (* logical — lowest *)
          "||"
        | "&&"
        |             (* comparison *)
          "==" | "!=" | "<=" | ">=" | "<" | ">"
        | "==." | "!=." | "<=." | ">=." | "<." | ">."
        |             (* additive *)
          "+" | "-" | "++"
        | "+." | "-."
        |             (* multiplicative — highest *)
          "*" | "/"
        | "*." | "/."

postfix_expr  ::= postfix_expr "." IDENT  (* field access *)
        | postfix_expr "[" expr "]"  (* index *)
        | atom_expr

match_expr  ::= "match" expr "do" match_case_expr* "end"
match_case_expr ::= "case" pattern "->" expr

atom_expr   ::= "(" expr ")"
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

raise_expr    ::= "raise" expr
borrow_expr     ::= "&" [ sigil ] IDENT
lambda_expr     ::= "fn" [ type_params ] "(" [ param ( "," param )* ] ")"
            "->" type [ "require" throws_type ] [ "throws" throws_type ]
            "do" stmt* "end"
handler_expr    ::= "handler" UIDENT [ "require" row_type ] "do" handler_fn* "end"
handler_fn    ::= "fn" IDENT [ type_params ] "(" [ param ( "," param )* ] ")"
            "->" type [ "require" throws_type ] [ "throws" throws_type ]
            "do" stmt* "end"
call_expr     ::= dotted_ident "(" [ labeled_arg ( "," labeled_arg )* ] ")"
labeled_arg     ::= IDENT ":" expr
constructor_expr  ::= UIDENT "(" [ ctor_arg ( "," ctor_arg )* ] ")"
ctor_arg      ::= [ IDENT ":" ] expr
record_expr     ::= "{" [ IDENT ":" expr ( "," IDENT ":" expr )* ] "}"
list_expr     ::= "[" [ expr ( "," expr )* [ "," ] ] "]"
array_expr    ::= "[|" [ expr ( "," expr )* [ "," ] ] "|]"
variable      ::= [ sigil ] IDENT
dotted_ident    ::= IDENT ( "." IDENT )*

(* ── Patterns ───────────────────────────────────────────────── *)

pattern       ::= literal_pattern
          | constructor_pattern
          | record_pattern
          | wildcard_pattern
          | variable_pattern

literal_pattern   ::= literal
variable_pattern  ::= [ sigil ] IDENT
constructor_pattern
          ::= UIDENT "(" [ ctor_pat_arg ( "," ctor_pat_arg )* ] ")"
ctor_pat_arg    ::= [ IDENT ":" ] pattern
record_pattern  ::= "{" [ rec_pat_field ( "," rec_pat_field )* [ "," ] ] "}"
rec_pat_field   ::= "_" | IDENT ":" pattern
            (* "_" must be the last element; enables partial matching *)
wildcard_pattern  ::= "_"

(* ── Literals ───────────────────────────────────────────────── *)

literal       ::= float_literal | integer_literal | boolean_literal
          | unit_literal  | string_literal | char_literal

float_literal   ::= [ "-" ] DIGIT+ "." DIGIT+
integer_literal   ::= [ "-" ] DIGIT+
boolean_literal   ::= "true" | "false"
unit_literal    ::= "()"
string_literal  ::= '"' string_char* '"'
          | "[=[" raw_char* "]=]"
string_char     ::= escape_seq | NON_QUOTE NON_BACKSLASH
raw_char      ::= ANY - "]=]"

char_literal    ::= "'" char_body "'"
char_body       ::= escape_seq | NON_QUOTE NON_BACKSLASH

escape_seq      ::= '\' named_esc
          | '\' OCTAL_DIGIT OCTAL_DIGIT? OCTAL_DIGIT?
          | '\x' HEX_DIGIT HEX_DIGIT
          | '\u{' HEX_DIGIT+ '}'
named_esc       ::= 'a' | 'b' | 't' | 'n' | 'v' | 'f' | 'r' | 'e'
          | '\\' | '\'' | '"'
OCTAL_DIGIT     ::= '0'..'7'
HEX_DIGIT       ::= '0'..'9' | 'a'..'f' | 'A'..'F'

(* ── Comments & Terminals ───────────────────────────────────── *)

comment       ::= line_comment | block_comment
line_comment    ::= "//" ANY* ( NEWLINE | EOF )
block_comment   ::= "/*" ANY* "*/"

STRING_LITERAL  ::= string_literal
IDENT       ::= [a-z_] [a-zA-Z0-9_]*     (* not a keyword *)
UIDENT      ::= [A-Z] [a-zA-Z0-9_]*     (* constructor/type names *)
DIGIT       ::= [0-9]
```

## Operator Precedence (low to high)

| Precedence | Operators | Associativity |
|-----------|-----------|---------------|
| 1 (lowest) | `\|\|` | Left |
| 2 | `&&` | Left |
| 3 | `==` `!=` `<` `>` `<=` `>=` (and `.` variants) | Left |
| 4 | `+` `-` `++` (and `.` variants) | Left |
| 5 (highest) | `*` `/` (and `.` variants) | Left |

Float operators use `.` suffix: `+.`, `-.`, `*.`, `/.`, `==.`, `<.`, etc.

String concatenation: `++`

## Import Forms

```nexus
// Import specific names
import { Console, println } from stdlib/stdio.nx

// Import specific names + module alias
import { Console }, * as stdio from stdlib/stdio.nx

// Import as module alias only
import * as list from stdlib/list.nx

// Import for side effects (rare)
import from some/module.nx

// Import WASM module
import external stdlib/stdlib.wasm
```

## Keywords

```
export, opaque, type, exception, import, external, from, as,
port, handler, inject, require, throws,
let, fn, return, do, end,
if, then, else, match, case,
try, catch, raise,
while, for, to,
conc, task,
true, false
```
