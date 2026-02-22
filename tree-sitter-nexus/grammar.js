/// <reference types="tree-sitter-cli/dsl" />
// @ts-check

/**
 * @param {RuleOrLiteral} sep
 * @param {RuleOrLiteral} rule
 * @returns {SeqRule}
 */
const sep1 = (sep, rule) => seq(rule, repeat(seq(sep, rule)));

/**
 * @param {RuleOrLiteral} rule
 * @returns {ChoiceRule}
 */
const commaSep = (rule) => optional(sep1(",", rule));

/**
 * @param {RuleOrLiteral} rule
 * @returns {SeqRule}
 */
const commaSep1 = (rule) => sep1(",", rule);

module.exports = grammar({
  name: "nexus",

  extras: ($) => [/\s+/, $.line_comment, $.block_comment],

  // identifier is the word token: used for keyword extraction.
  // Restricted to lowercase/underscore-start so uppercase names (uident)
  // are lexically disjoint — no tokenizer conflict.
  word: ($) => $.identifier,

  conflicts: ($) => [
    // `foo` vs `foo.bar` — variable or start of dotted_identifier
    [$.variable, $.dotted_identifier],
    // `T` vs `T<U>` — bare type identifier or start of generic_type
    [$.generic_type, $._type],
    [$.generic_type, $._effect_type],
  ],

  rules: {
    source_file: ($) => repeat($._top_level),

    // ─── Comments ────────────────────────────────────────────────────────────

    line_comment: (_) => token(seq("//", /.*/)),

    block_comment: (_) =>
      token(seq("/*", repeat(choice(/[^*]/, seq("*", /[^/]/))), "*/")),

    // ─── Identifiers ─────────────────────────────────────────────────────────

    // Lowercase/underscore-start: variables, function names, labels, keywords
    identifier: (_) => /[a-z_][a-zA-Z0-9_]*/,

    // Uppercase-start: constructor names, type names, effect names, type vars
    uident: (_) => /[A-Z][a-zA-Z0-9_]*/,

    // ─── Top-level definitions ───────────────────────────────────────────────

    _top_level: ($) =>
      choice(
        $.type_def,
        $.exception_def,
        $.import_def,
        $.port_def,
        $.handler_def,
        $.let_def,
        $.line_comment,
        $.block_comment
      ),

    // [pub] type Name[<T>] = { field: type, ... }
    // [pub] type Name[<T>] = A(label: T) | B
    type_def: ($) =>
      seq(
        optional("pub"),
        "type",
        field("name", $.uident),
        optional(field("type_params", $.type_params)),
        "=",
        field("body", choice($.record_type, $.type_sum_def))
      ),

    // A(label: T) | B(U)
    type_sum_def: ($) => sep1("|", $.variant_def),

    variant_def: ($) =>
      seq(
        field("name", $.uident),
        optional(seq("(", commaSep1($.variant_field), ")"))
      ),

    // type | label: type
    variant_field: ($) =>
      choice(
        seq(field("label", $.identifier), ":", field("type", $._type)),
        field("type", $._type)
      ),

    // [pub] exception NotFound(msg: string)
    exception_def: ($) =>
      seq(
        optional("pub"),
        "exception",
        field("name", $.uident),
        optional(seq("(", commaSep1($.variant_field), ")"))
      ),

    // import external path/to/lib.wasm
    // import { a, b } from path/to/mod.nx
    // import as alias from path/to/mod.nx
    // import from path/to/mod.nx
    import_def: ($) =>
      seq(
        "import",
        choice(
          seq("external", field("path", $.import_path)),
          seq(
            "{",
            field("items", commaSep1($.identifier)),
            "}",
            "from",
            field("path", $.import_path)
          ),
          seq(
            "as",
            field("alias", $.identifier),
            "from",
            field("path", $.import_path)
          ),
          seq("from", field("path", $.import_path))
        )
      ),

    // Import path uses bracket string literal: [=[path/to/module.nx]=]
    import_path: ($) => $.string_literal,

    // [pub] port Name do fn sig ... endport
    port_def: ($) =>
      seq(
        optional("pub"),
        "port",
        field("name", $.uident),
        "do",
        repeat($.fn_signature),
        "endport"
      ),

    fn_signature: ($) =>
      seq(
        "fn",
        field("name", $.identifier),
        field("params", $.param_list),
        "->",
        field("ret_type", $._type),
        optional(seq("effect", field("effects", $._effect_type)))
      ),

    // [pub] handler Name for PortName do handler_fn* endhandler
    handler_def: ($) =>
      seq(
        optional("pub"),
        "handler",
        field("name", $.uident),
        "for",
        field("port_name", $.uident),
        "do",
        repeat($.handler_fn),
        "endhandler"
      ),

    // fn name[<T>](params) -> ret [effect eff] do body endfn
    handler_fn: ($) =>
      seq(
        "fn",
        field("name", $.identifier),
        optional(field("type_params", $.type_params)),
        field("params", $.param_list),
        "->",
        field("ret_type", $._type),
        optional(seq("effect", field("effects", $._effect_type))),
        "do",
        field("body", repeat($._stmt)),
        "endfn"
      ),

    // [pub] let name [: type] = expr
    let_def: ($) =>
      seq(
        optional("pub"),
        "let",
        field("name", $.identifier),
        optional(seq(":", field("type", $._type))),
        "=",
        field("value", $._expr)
      ),

    // ─── Parameters ──────────────────────────────────────────────────────────

    type_params: ($) => seq("<", commaSep1($.uident), ">"),

    param_list: ($) => seq("(", commaSep($.param), ")"),

    param: ($) =>
      seq(
        optional(field("sigil", $.sigil)),
        field("name", $.identifier),
        ":",
        field("type", $._type)
      ),

    // ~ = mutable, % = linear, (none) = immutable
    sigil: (_) => choice("~", "%"),

    // ─── Types ───────────────────────────────────────────────────────────────

    _type: ($) =>
      choice(
        $.arrow_type,
        $.generic_type,
        $.primitive_type,
        $.ref_type,
        $.borrow_type,
        $.linear_type,
        $.record_type,
        $.list_type,
        $.array_type,
        $.row_type,
        alias($.uident, $.type_identifier) // type variable or user-defined monotype
      ),

    primitive_type: (_) =>
      choice("i32", "i64", "f32", "f64", "float", "bool", "string", "unit"),

    // ref(T)
    ref_type: ($) => seq("ref", "(", field("inner", $._type), ")"),

    // &T
    borrow_type: ($) => seq("&", field("inner", $._type)),

    // %T
    linear_type: ($) => seq("%", field("inner", $._type)),

    // { x: T, y: U }
    record_type: ($) => seq("{", commaSep1($.record_type_field), "}"),

    record_type_field: ($) =>
      seq(field("name", $.identifier), ":", field("type", $._type)),

    // { E1, E2 | r }  or  { E1, E2 }
    row_type: ($) =>
      seq(
        "{",
        commaSep1($._type),
        optional(seq("|", field("tail", $._type))),
        "}"
      ),

    // [T]
    list_type: ($) => seq("[", field("element", $._type), "]"),

    // [| T |]
    array_type: ($) =>
      seq(
        alias(token("[|"), "[|"),
        field("element", $._type),
        alias(token("|]"), "|]")
      ),

    // Name<T, U>  or  Result<T, E>
    generic_type: ($) =>
      seq(
        field("base", alias($.uident, $.type_identifier)),
        "<",
        field("args", commaSep1($._type)),
        ">"
      ),

    // (label: T, ...) -> ret [effect eff]
    // prec.right makes the optional 'effect' clause greedy (prefer consuming it)
    arrow_type: ($) =>
      prec.right(
        seq(
          "(",
          commaSep(
            choice(
              seq(
                field("param_label", $.identifier),
                ":",
                field("param_type", $._type)
              ),
              field("param_type", $._type)
            )
          ),
          ")",
          "->",
          field("ret", $._type),
          optional(seq("effect", field("effect", $._effect_type)))
        )
      ),

    _effect_type: ($) =>
      choice(
        $.row_type,
        $.generic_type,
        alias($.uident, $.type_identifier)
      ),

    // ─── Statements ──────────────────────────────────────────────────────────

    _stmt: ($) =>
      choice(
        $.let_stmt,
        $.return_stmt,
        $.assign_stmt,
        $.drop_stmt,
        $.if_stmt,
        $.match_stmt,
        $.try_stmt,
        $.conc_stmt,
        $.line_comment,
        $.block_comment,
        $.expr_stmt
      ),

    // let [sigil] name [: type] = expr
    let_stmt: ($) =>
      seq(
        "let",
        optional(field("sigil", $.sigil)),
        field("name", $.identifier),
        optional(seq(":", field("type", $._type))),
        "=",
        field("value", $._expr)
      ),

    return_stmt: ($) => seq("return", field("value", $._expr)),

    // target <- value
    assign_stmt: ($) =>
      seq(field("target", $._expr), "<-", field("value", $._expr)),

    // drop [sigil] name
    drop_stmt: ($) =>
      seq(
        "drop",
        optional(field("sigil", $.sigil)),
        field("name", $.identifier)
      ),

    // if cond then stmts [else stmts] endif
    if_stmt: ($) =>
      seq(
        "if",
        field("cond", $._expr),
        "then",
        field("then_branch", repeat($._stmt)),
        optional(seq("else", field("else_branch", repeat($._stmt)))),
        "endif"
      ),

    // match expr do case pat -> stmts ... endmatch
    match_stmt: ($) =>
      seq(
        "match",
        field("target", $._expr),
        "do",
        repeat($.match_case),
        "endmatch"
      ),

    match_case: ($) =>
      seq(
        "case",
        field("pattern", $._pattern),
        "->",
        field("body", repeat($._stmt))
      ),

    // try stmts catch param -> stmts endtry
    try_stmt: ($) =>
      seq(
        "try",
        field("body", repeat($._stmt)),
        "catch",
        field("catch_param", $.identifier),
        "->",
        field("catch_body", repeat($._stmt)),
        "endtry"
      ),

    // conc do task "name" do stmts endtask ... endconc
    conc_stmt: ($) => seq("conc", "do", repeat($.task_def), "endconc"),

    task_def: ($) =>
      seq(
        "task",
        field("name", $.identifier),
        optional(seq("effect", field("effects", $._effect_type))),
        "do",
        field("body", repeat($._stmt)),
        "endtask"
      ),

    expr_stmt: ($) => $._expr,

    // ─── Expressions ─────────────────────────────────────────────────────────

    _expr: ($) => choice($.binary_expr, $._postfix_expr),

    // Binary operators with precedence levels
    // Level 1: comparison  Level 2: additive  Level 3: multiplicative
    binary_expr: ($) =>
      choice(
        // Float comparisons (must come before int comparisons to avoid partial matches)
        prec.left(
          1,
          seq(
            field("left", $._expr),
            field("operator", choice("==.", "!=.", "<=.", ">=.", "<.", ">.")),
            field("right", $._expr)
          )
        ),
        // Int/generic comparisons
        prec.left(
          1,
          seq(
            field("left", $._expr),
            field("operator", choice("==", "!=", "<=", ">=", "<", ">")),
            field("right", $._expr)
          )
        ),
        // Float additive
        prec.left(
          2,
          seq(
            field("left", $._expr),
            field("operator", choice("+.", "-.")),
            field("right", $._expr)
          )
        ),
        // Int/string additive (++ = string concat)
        prec.left(
          2,
          seq(
            field("left", $._expr),
            field("operator", choice("++", "+", "-")),
            field("right", $._expr)
          )
        ),
        // Float multiplicative
        prec.left(
          3,
          seq(
            field("left", $._expr),
            field("operator", choice("*.", "/.")),
            field("right", $._expr)
          )
        ),
        // Int multiplicative
        prec.left(
          3,
          seq(
            field("left", $._expr),
            field("operator", choice("*", "/")),
            field("right", $._expr)
          )
        )
      ),

    _postfix_expr: ($) =>
      choice($.field_access, $.index_expr, $._atom_expr),

    // expr.field  (highest precedence postfix)
    field_access: ($) =>
      prec.left(
        10,
        seq(
          field("object", $._postfix_expr),
          ".",
          field("field", $.identifier)
        )
      ),

    // expr[index]
    index_expr: ($) =>
      prec.left(
        10,
        seq(
          field("object", $._postfix_expr),
          "[",
          field("index", $._expr),
          "]"
        )
      ),

    _atom_expr: ($) =>
      choice(
        $.paren_expr,
        $.raise_expr,
        $.borrow_expr,
        $.lambda_expr,
        $.external_expr,
        $.perform_call,
        $.call_expr,
        $.constructor_expr,
        $.record_expr,
        $.array_expr,
        $.list_expr,
        $.literal,
        $.variable
      ),

    paren_expr: ($) => seq("(", $._expr, ")"),

    // raise expr
    raise_expr: ($) => seq("raise", field("value", $._expr)),

    // borrow [sigil] name
    borrow_expr: ($) =>
      seq(
        "borrow",
        optional(field("sigil", $.sigil)),
        field("name", $.identifier)
      ),

    // fn [<T>](params) -> ret [effect eff] do body endfn
    lambda_expr: ($) =>
      prec.right(
        seq(
          "fn",
          optional(field("type_params", $.type_params)),
          field("params", $.param_list),
          "->",
          field("ret_type", $._type),
          optional(seq("effect", field("effects", $._effect_type))),
          "do",
          field("body", repeat($._stmt)),
          "endfn"
        )
      ),

    // external [=[wasm_symbol]=] : arrow_type
    external_expr: ($) =>
      seq(
        "external",
        field("wasm_name", $.string_literal),
        ":",
        field("type", $.arrow_type)
      ),

    // perform path(label: value, ...)
    // The path can start with UIDENT for port-qualified calls: perform Logger.log(msg)
    perform_call: ($) =>
      seq(
        "perform",
        field("func", $.perform_path),
        "(",
        field("args", commaSep($.labeled_arg)),
        ")"
      ),

    // Path used in perform calls — first segment can be UIDENT (port name) or IDENT (module/function)
    perform_path: ($) =>
      seq(
        choice($.identifier, $.uident),
        repeat(seq(".", $.identifier))
      ),

    // path(label: value, ...)
    call_expr: ($) =>
      seq(
        field("func", $.dotted_identifier),
        "(",
        field("args", commaSep($.labeled_arg)),
        ")"
      ),

    // label: value
    labeled_arg: ($) =>
      seq(
        field("label", $.identifier),
        ":",
        field("value", $._simple_arg)
      ),

    _simple_arg: ($) => choice($.literal, $.variable),

    // Constructor(label: value, ...)  — optional labels, UIDENT name
    constructor_expr: ($) =>
      seq(
        field("name", $.uident),
        "(",
        field("args", commaSep($.ctor_arg)),
        ")"
      ),

    // [ label ":" ] expr
    ctor_arg: ($) =>
      choice(
        seq(field("label", $.identifier), ":", field("value", $._expr)),
        field("value", $._expr)
      ),

    // { field: value, ... }
    record_expr: ($) => seq("{", commaSep($.record_expr_field), "}"),

    record_expr_field: ($) =>
      seq(field("name", $.identifier), ":", field("value", $._expr)),

    // [e1, e2, ...]  — trailing comma allowed per spec
    list_expr: ($) =>
      seq("[", field("elements", commaSep($._expr)), optional(","), "]"),

    // [| e1, e2, ... |]  — trailing comma allowed per spec
    array_expr: ($) =>
      seq(
        alias(token("[|"), "[|"),
        field("elements", commaSep($._expr)),
        optional(","),
        alias(token("|]"), "|]")
      ),

    // [sigil]name   e.g.  x  ~x  %x
    variable: ($) =>
      seq(
        optional(field("sigil", $.sigil)),
        field("name", $.identifier)
      ),

    // a.b.c  — used as function path in calls
    dotted_identifier: ($) => sep1(".", $.identifier),

    // ─── Literals ────────────────────────────────────────────────────────────

    literal: ($) =>
      choice(
        $.float_literal,
        $.integer_literal,
        $.boolean_literal,
        $.unit_literal,
        $.string_literal
      ),

    // Must come before integer_literal to consume the decimal part
    float_literal: (_) => token(prec(2, /-?[0-9]+\.[0-9]+/)),

    integer_literal: (_) => token(prec(1, /-?[0-9]+/)),

    boolean_literal: (_) => choice("true", "false"),

    // ()
    unit_literal: (_) => token("()"),

    // [=[ content ]=]
    // Escape: \]=] represents a literal ]=] inside the string.
    string_literal: (_) =>
      token(
        seq(
          "[=[",
          repeat(
            choice(
              seq("\\", "]=]"), // escape sequence: \]=] → ]=]
              seq("\\", /[^\]]/), // backslash + non-] character
              seq("\\", "]", /[^=]/), // \] not starting \]=]
              seq("\\", "]=", /[^\]]/), // \]= not completing \]=]
              seq("]", /[^=]/), // ] not starting ]=]
              seq("]=", /[^\]]/), // ]= not completing ]=]
              /[^\]\\]/ // any char except ] and \
            )
          ),
          "]=]"
        )
      ),

    // ─── Patterns ────────────────────────────────────────────────────────────

    _pattern: ($) =>
      choice(
        $.literal_pattern,
        $.constructor_pattern,
        $.record_pattern,
        $.wildcard_pattern,
        $.variable_pattern
      ),

    literal_pattern: ($) => $.literal,

    // [sigil]name
    variable_pattern: ($) =>
      seq(
        optional(field("sigil", $.sigil)),
        field("name", $.identifier)
      ),

    // Constructor([ label ":" ] pat, ...)  — optional labels, UIDENT name
    constructor_pattern: ($) =>
      seq(
        field("name", $.uident),
        "(",
        commaSep($.ctor_pat_arg),
        ")"
      ),

    // [ label ":" ] pattern
    ctor_pat_arg: ($) =>
      choice(
        seq(field("label", $.identifier), ":", field("pattern", $._pattern)),
        field("pattern", $._pattern)
      ),

    // { field: pat, ..., _ }
    record_pattern: ($) =>
      seq(
        "{",
        commaSep(
          choice(
            field("wildcard", alias("_", $.wildcard_pattern)),
            seq(
              field("field_name", $.identifier),
              ":",
              field("field_pattern", $._pattern)
            )
          )
        ),
        optional(","),
        "}"
      ),

    wildcard_pattern: (_) => "_",
  },
});
