; ─── Keywords ──────────────────────────────────────────────────────────────

; Definition keywords
"fn" @keyword.function
"external" @keyword.function
"pub" @keyword.modifier.pub

; Unified block-end keyword
"end" @keyword

; Control flow
"if" @keyword.if
"then" @keyword.if
"else" @keyword.if
"match" @keyword.match
"case" @keyword.match.case
"return" @keyword.return
"raise" @keyword.raise

; Effect/coeffect system
"effect" @keyword.effect
"require" @keyword.require
"inject" @keyword.inject

; Error handling
"try" @keyword.try
"catch" @keyword.try

; Concurrency
"conc" @keyword.conc
"do" @keyword
"task" @keyword.task

; Type/exception definitions
"type" @keyword.type
"exception" @keyword.type

; Import
"import" @keyword.import
"from" @keyword.import
"as" @keyword.import

; Port/handler
"port" @keyword.port
"handler" @keyword.handler

"let" @keyword.let

; ─── Types ──────────────────────────────────────────────────────────────────

(primitive_type) @type.builtin

(type_identifier) @type

(generic_type
  base: (type_identifier) @type)

(type_def
  name: (uident) @type.definition)

(exception_def
  name: (uident) @type.definition)

(variant_def
  name: (uident) @constructor)

(variant_field
  label: (identifier) @variable.member)

; ─── Functions ──────────────────────────────────────────────────────────────

(handler_fn
  name: (identifier) @function)

(lambda_expr
  ret_type: _ @type)

(fn_signature
  name: (identifier) @function)

(call_expr
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

(ctor_arg
  label: (identifier) @variable.parameter)

(ctor_pat_arg
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

(let_def
  name: (identifier) @variable)

(external_def
  name: (identifier) @variable)

(variable_pattern
  name: (identifier) @variable)

; ─── Sigils & Type modifiers ────────────────────────────────────────────────

(sigil) @operator

"opaque" @keyword.modifier

(linear_type
  "%" @operator)

(borrow_type
  "&" @operator)

; ─── Literals ───────────────────────────────────────────────────────────────

(integer_literal) @number

(float_literal) @number.float

(boolean_literal) @boolean

(unit_literal) @constant.builtin

(string_literal) @string

(import_path) @string.special.path

; ─── Operators ──────────────────────────────────────────────────────────────

(binary_expr
  operator: _ @operator)

"<-" @operator
"->" @operator
"=" @operator
":" @punctuation.delimiter

; ─── Comments ───────────────────────────────────────────────────────────────

(line_comment) @comment
(block_comment) @comment

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

(handler_expr
  port_name: (uident) @type)

(inject_stmt
  handler: (inject_handler
    (identifier) @variable))
