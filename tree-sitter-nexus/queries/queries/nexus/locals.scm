; ─── Scope definitions ──────────────────────────────────────────────────────

(lambda_expr) @local.scope
(handler_expr) @local.scope
(handler_fn) @local.scope
(inject_stmt) @local.scope
(if_stmt) @local.scope
(match_case) @local.scope
(try_stmt) @local.scope
(conc_stmt) @local.scope
(task_def) @local.scope

; ─── Definitions ────────────────────────────────────────────────────────────

(let_stmt
  name: (identifier) @local.definition)

(let_def
  name: (identifier) @local.definition)

(param
  name: (identifier) @local.definition)

(handler_fn
  name: (identifier) @local.definition)

(try_stmt
  catch_param: (identifier) @local.definition)

(variable_pattern
  name: (identifier) @local.definition)

; ─── References ─────────────────────────────────────────────────────────────

(variable
  name: (identifier) @local.reference)
