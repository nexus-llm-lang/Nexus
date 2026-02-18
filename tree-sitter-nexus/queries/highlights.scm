; ─── Keywords ──────────────────────────────────────────────────────────────

; Definition keywords
"fn" @keyword.function
"endfn" @keyword.function
"external" @keyword.function
"pub" @keyword.modifier

; Control flow
"if" @keyword.control
"then" @keyword.control
"else" @keyword.control
"endif" @keyword.control
"match" @keyword.control
"case" @keyword.control
"endmatch" @keyword.control
"return" @keyword.control
"raise" @keyword.control

; Effect system
"perform" @keyword
"effect" @keyword
"borrow" @keyword

; Error handling
"try" @keyword.control
"catch" @keyword.control
"endtry" @keyword.control

; Concurrency
"conc" @keyword
"do" @keyword
"endconc" @keyword
"task" @keyword
"endtask" @keyword

; Type/enum definitions
"type" @keyword.type
"enum" @keyword.type

; Import
"import" @keyword.import
"from" @keyword.import
"as" @keyword.import

; Port/handler
"port" @keyword
"endport" @keyword
"handler" @keyword
"for" @keyword
"endhandler" @keyword

"let" @keyword

; ─── Types ──────────────────────────────────────────────────────────────────

(primitive_type) @type.builtin

(type_identifier) @type

(generic_type
  base: (type_identifier) @type)

(type_def
  name: (uident) @type.definition)

(enum_def
  name: (uident) @type.definition)

(variant_def
  name: (uident) @constructor)

; ─── Functions ──────────────────────────────────────────────────────────────

(function_def
  name: (identifier) @function)

(lambda_expr
  ret_type: _ @type)

(external_fn_def
  name: (identifier) @function)

(fn_signature
  name: (identifier) @function)

(call_expr
  func: (dotted_identifier) @function.call)

(perform_call
  func: (dotted_identifier) @function.call)

(constructor_expr
  name: (uident) @constructor)

(constructor_pattern
  name: (uident) @constructor)

; ─── Parameters & Labels ────────────────────────────────────────────────────

(param
  name: (identifier) @variable.parameter)

(labeled_arg
  label: (identifier) @variable.parameter)

(record_type_field
  name: (identifier) @variable.member)

(record_expr_field
  name: (identifier) @variable.member)

; ─── Variables ──────────────────────────────────────────────────────────────

(variable
  name: (identifier) @variable)

(let_stmt
  name: (identifier) @variable)

(variable_pattern
  name: (identifier) @variable)

; ─── Sigils ─────────────────────────────────────────────────────────────────

(sigil) @operator

; ─── Literals ───────────────────────────────────────────────────────────────

(integer_literal) @number

(float_literal) @number.float

(boolean_literal) @boolean

(unit_literal) @constant.builtin

(string_literal) @string

(quoted_string) @string

; ─── Operators ──────────────────────────────────────────────────────────────

(binary_expr
  operator: _ @operator)

"<-" @operator
"->" @operator
"=" @operator
":" @punctuation.delimiter

; ─── Comments ───────────────────────────────────────────────────────────────

(line_comment) @comment @spell
(block_comment) @comment @spell

; ─── Punctuation ────────────────────────────────────────────────────────────

"(" @punctuation.bracket
")" @punctuation.bracket
"{" @punctuation.bracket
"}" @punctuation.bracket
"[" @punctuation.bracket
"]" @punctuation.bracket
"[|" @punctuation.bracket
"|]" @punctuation.bracket
"<" @punctuation.bracket
">" @punctuation.bracket

"," @punctuation.delimiter
"." @punctuation.delimiter
"|" @punctuation.delimiter

; ─── Handler & Port ─────────────────────────────────────────────────────────

(port_def
  name: (uident) @type)

(handler_def
  name: (uident) @type
  port_name: (uident) @type)
