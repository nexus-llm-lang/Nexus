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
    // `&x` — borrow_expr starting with `&` or sigil `&` in variable/let_stmt
    [$.sigil, $.borrow_expr],
    // `match x do ...` — match_stmt (stmt body) vs match_expr (expr body) via expr_stmt
    [$.match_stmt, $.match_expr],
    // `case pat -> expr` — expr_stmt in match_case body vs match_case_expr value
    [$.expr_stmt, $.match_case_expr],
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

    // Uppercase-start: constructor names, type names, exception names, type vars
    uident: (_) => /[A-Z][a-zA-Z0-9_]*/,

    // ─── Top-level definitions ───────────────────────────────────────────────

    _top_level: ($) =>
      choice(
        $.type_def,
        $.exception_def,
        $.import_def,
        $.port_def,
        $.external_def,
        $.let_def,
        $.line_comment,
        $.block_comment
      ),

    // [pub] [opaque] type Name[<T>] = { field: type, ... }
    // [pub] [opaque] type Name[<T>] = A(label: T) | B
    type_def: ($) =>
      seq(
        optional("export"),
        optional("opaque"),
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
        optional("export"),
        "exception",
        field("name", $.uident),
        optional(seq("(", commaSep1($.variant_field), ")"))
      ),

    // import external path/to/lib.wasm
    // import { a, b } from path/to/mod.nx
    // import { a, b }, * as alias from path/to/mod.nx
    // import as alias from path/to/mod.nx
    // import from path/to/mod.nx
    import_def: ($) =>
      seq(
        "import",
        choice(
          seq("external", field("path", $.import_path)),
          seq(
            "{",
            field("items", commaSep1(choice($.identifier, $.uident))),
            "}",
            ",",
            "*",
            "as",
            field("alias", $.identifier),
            "from",
            field("path", $.import_path)
          ),
          seq(
            "{",
            field("items", commaSep1(choice($.identifier, $.uident))),
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

    // Bare import path: alphanumeric segments separated by / with optional .ext
    // e.g. nxlib/stdlib/fs.nx, math.wasm
    import_path: ($) => /[a-zA-Z0-9_\-/.]+/,

    // [pub] port Name do fn sig ... end
    port_def: ($) =>
      seq(
        optional("export"),
        "port",
        field("name", $.uident),
        "do",
        repeat($.fn_signature),
        "end"
      ),

    fn_signature: ($) =>
      seq(
        "fn",
        field("name", $.identifier),
        field("params", $.param_list),
        "->",
        field("ret_type", $._type),
        optional(seq("require", field("requires", $._effect_type))),
        optional(seq("throws", field("throws", $._effect_type)))
      ),

    // fn name[<T>](params) -> ret [require req] [throws eff] do body end
    handler_fn: ($) =>
      seq(
        "fn",
        field("name", $.identifier),
        optional(field("type_params", $.type_params)),
        field("params", $.param_list),
        "->",
        field("ret_type", $._type),
        optional(seq("require", field("requires", $._effect_type))),
        optional(seq("throws", field("throws", $._effect_type))),
        "do",
        field("body", repeat($._stmt)),
        "end"
      ),

    // [pub] let [sigil] name [: type] = expr
    let_def: ($) =>
      seq(
        optional("export"),
        "let",
        optional(field("sigil", $.sigil)),
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

    // ~ = mutable, % = linear, & = borrow, (none) = immutable
    sigil: (_) => choice("~", "%", "&"),

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

    // (label: T, ...) -> ret [require req] [throws eff]
    // prec.right makes the optional require/throws clauses greedy
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
          optional(seq("require", field("require", $._effect_type))),
          optional(seq("throws", field("throws", $._effect_type)))
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
        $.while_stmt,
        $.for_stmt,
        $.try_stmt,
        $.inject_stmt,
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

    // if cond then stmts [else stmts] end
    if_stmt: ($) =>
      seq(
        "if",
        field("cond", $._expr),
        "then",
        field("then_branch", repeat($._stmt)),
        optional(seq("else", field("else_branch", repeat($._stmt)))),
        "end"
      ),

    // match expr do case pat -> stmts ... end
    match_stmt: ($) =>
      seq(
        "match",
        field("target", $._expr),
        "do",
        repeat($.match_case),
        "end"
      ),

    match_case: ($) =>
      seq(
        "case",
        field("pattern", $._pattern),
        "->",
        field("body", repeat($._stmt))
      ),

    // while cond do stmts end
    while_stmt: ($) =>
      seq(
        "while",
        field("cond", $._expr),
        "do",
        field("body", repeat($._stmt)),
        "end"
      ),

    // for var = start to end do stmts end
    for_stmt: ($) =>
      seq(
        "for",
        field("var", $.identifier),
        "=",
        field("start", $._expr),
        "to",
        field("end", $._expr),
        "do",
        field("body", repeat($._stmt)),
        "end"
      ),

    // try stmts catch param -> stmts end
    try_stmt: ($) =>
      seq(
        "try",
        field("body", repeat($._stmt)),
        "catch",
        field("catch_param", $.identifier),
        "->",
        field("catch_body", repeat($._stmt)),
        "end"
      ),

    // inject handler1, mod.handler2 do stmts end
    inject_stmt: ($) =>
      seq(
        "inject",
        sep1(",", field("handler", $.inject_handler)),
        "do",
        field("body", repeat($._stmt)),
        "end"
      ),

    // handler name in inject: plain `name` or dotted `mod.name`
    inject_handler: ($) =>
      seq(
        $.identifier,
        optional(seq(".", $.identifier))
      ),

    // conc do task "name" do stmts end ... end
    conc_stmt: ($) => seq("conc", "do", repeat($.task_def), "end"),

    task_def: ($) =>
      seq(
        "task",
        field("name", $.identifier),
        optional(seq("throws", field("throws", $._effect_type))),
        "do",
        field("body", repeat($._stmt)),
        "end"
      ),

    // drop %x
    drop_stmt: ($) => seq("drop", field("value", $._expr)),

    expr_stmt: ($) => $._expr,

    // ─── Expressions ─────────────────────────────────────────────────────────

    _expr: ($) => choice($.binary_expr, $._postfix_expr),

    // Binary operators with precedence levels (low → high)
    // 1: ||  2: &&  3: comparison  4: additive  5: multiplicative
    binary_expr: ($) =>
      choice(
        // Logical OR (lowest precedence)
        prec.left(
          1,
          seq(
            field("left", $._expr),
            field("operator", "||"),
            field("right", $._expr)
          )
        ),
        // Logical AND
        prec.left(
          2,
          seq(
            field("left", $._expr),
            field("operator", "&&"),
            field("right", $._expr)
          )
        ),
        // Float comparisons (must come before int comparisons to avoid partial matches)
        prec.left(
          3,
          seq(
            field("left", $._expr),
            field("operator", choice("==.", "!=.", "<=.", ">=.", "<.", ">.")),
            field("right", $._expr)
          )
        ),
        // Int/generic comparisons
        prec.left(
          3,
          seq(
            field("left", $._expr),
            field("operator", choice("==", "!=", "<=", ">=", "<", ">")),
            field("right", $._expr)
          )
        ),
        // Float additive
        prec.left(
          4,
          seq(
            field("left", $._expr),
            field("operator", choice("+.", "-.")),
            field("right", $._expr)
          )
        ),
        // Int/string additive (++ = string concat)
        prec.left(
          4,
          seq(
            field("left", $._expr),
            field("operator", choice("++", "+", "-")),
            field("right", $._expr)
          )
        ),
        // Float multiplicative
        prec.left(
          5,
          seq(
            field("left", $._expr),
            field("operator", choice("*.", "/.")),
            field("right", $._expr)
          )
        ),
        // Int multiplicative
        prec.left(
          5,
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
        $.match_expr,
        $.raise_expr,
        $.borrow_expr,
        $.lambda_expr,
        $.handler_expr,
        $.call_expr,
        $.constructor_expr,
        $.record_expr,
        $.array_expr,
        $.list_expr,
        $.literal,
        $.variable
      ),

    // match expr do case pat -> expr ... end  (expression position)
    match_expr: ($) =>
      seq(
        "match",
        field("target", $._expr),
        "do",
        repeat($.match_case_expr),
        "end"
      ),

    match_case_expr: ($) =>
      seq(
        "case",
        field("pattern", $._pattern),
        "->",
        field("value", $._expr)
      ),

    paren_expr: ($) => seq("(", $._expr, ")"),

    // raise expr
    raise_expr: ($) => seq("raise", field("value", $._expr)),

    // & [sigil] name
    borrow_expr: ($) =>
      seq(
        "&",
        optional(field("sigil", $.sigil)),
        field("name", $.identifier)
      ),

    // fn [<T>](params) -> ret [require req] [throws eff] do body end
    lambda_expr: ($) =>
      prec.right(
        seq(
          "fn",
          optional(field("type_params", $.type_params)),
          field("params", $.param_list),
          "->",
          field("ret_type", $._type),
          optional(seq("require", field("requires", $._effect_type))),
          optional(seq("throws", field("throws", $._effect_type))),
          "do",
          field("body", repeat($._stmt)),
          "end"
        )
      ),

    // [pub] external name = [=[wasm_symbol]=] : [<T>] arrow_type
    external_def: ($) =>
      seq(
        optional("export"),
        "external",
        field("name", $.identifier),
        "=",
        field("wasm_name", $.string_literal),
        ":",
        optional(field("type_params", $.type_params)),
        field("type", $.arrow_type)
      ),

    // handler PortName [require { ... }] do handler_fn* end
    handler_expr: ($) =>
      seq(
        "handler",
        field("port_name", $.uident),
        optional(seq("require", field("requires", $._effect_type))),
        "do",
        repeat($.handler_fn),
        "end"
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
        field("value", $._expr)
      ),

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

    // a.b.c or Logger.log — used as function path in calls
    // UIDENT-start requires at least one dot segment to avoid conflict with constructor_expr
    dotted_identifier: ($) =>
      choice(
        sep1(".", $.identifier),
        seq($.uident, ".", sep1(".", $.identifier))
      ),

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

    // "..." or [=[ ... ]=]
    string_literal: (_) =>
      token(
        choice(
          // double-quoted string with escape sequences
          seq(
            '"',
            repeat(choice(/\\[nrt\\""]/, /[^"\\\n]/)),
            '"'
          ),
          // bracket string [=[ ... ]=]
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
