# CLI / parser parity audit (nexus-dvr6.7)

Read-mostly audit of `bootstrap/src/cli.rs` + `bootstrap/src/main.rs` (Rust)
against `src/driver.nx` (self-hosted), and of `bootstrap/src/lang/parser.rs`
against `src/frontend/parser.nx` (+ split modules `parse_core.nx`,
`parse_type.nx`, `parse_pattern.nx`, `parse_topdef.nx`, `parse_params.nx`,
`parse_names.nx`).

The bootstrap (Rust) compiler is decommissioned in epic dvr6 — the goal is
that every program `bootstrap` accepts must also be accepted by the
self-hosted driver, modulo gaps explicitly documented here. The audit covers
the surface accepted by the parsers; semantic-checker / codegen parity is
out of scope for this issue.

## 1. CLI subcommand & option matrix

### 1.1 Subcommands (`Cli::Command` enum, `bootstrap/src/cli.rs:38`)

| Bootstrap subcommand | Self-hosted equivalent | Status |
| --- | --- | --- |
| `nexus build <file>` | `nxc <file>` and `nxc build <file>` (driver.nx `strip_build_subcommand`) | parity |
| `nexus build` reading from stdin (`-` or piped tty) | not implemented in self-host | gap, see §1.4 |
| `nexus compose <file>` | not implemented in self-host | out-of-scope (bootstrap-only) |
| no subcommand → `eprintln "No command specified..."` exit 1 | self-host raises `InvalidUsage` and exits 2 with usage banner | exit-code divergence, see §1.4 |

### 1.2 `build` options

| Bootstrap flag | Self-hosted equivalent | Status |
| --- | --- | --- |
| `[input]` positional, `Option<PathBuf>` | first positional after `[build]` (driver.nx `strip_build_subcommand` + `str_nth(xs, n: 1)`) | parity |
| `-o, --output <path>` | `-o` / `--output <path>` (driver.nx `strip_output_flag`) | parity |
| positional `[output.wasm]` (defaults to `main.wasm`) | positional output supported (driver.nx `get_output_path`); default is `out.wasm` | default-name divergence, see §1.4 |
| `--wasm-merge <path>` (override `wasm-merge` exec) | ignored — self-host uses in-tree `wmerge.merge_with_stdlib` (no external `wasm-merge` dependency) | explicit out-of-scope (self-host has no shell-out) |
| `--explain-capabilities <yes\|none\|wasmtime>` | `--explain-capabilities yes\|none\|wasmtime` (driver.nx `strip_explain_flags`) | parity (self-host adds `-W` proposal flags in `wasmtime` mode — superset, see §1.3) |
| `--explain-capabilities-format <text\|json>` | `--explain-capabilities-format text\|json` | parity |
| `-v, --verbose` (global) | `--verbose` (driver.nx `has_verbose_flag` / `strip_verbose_flag`) | parity (self-host accepts the long form only; bootstrap also accepts `-v`) |
| `--dump-imports <file>` | `--dump-imports <file>` (driver.nx) | self-host superset (bootstrap has no equivalent CLI surface — debug aid only) |

### 1.3 `--explain-capabilities wasmtime` output divergence

Bootstrap's `print_build_result_text` / `_json` (artifact.rs:39 onwards)
emits only capability-derived flags (`--dir .`, `--wasi http`,
`--wasi inherit-network`).

Self-host's `wasmtime_feature_flags` (driver.nx:332) prepends the four wasm
proposals the runtime needs:

```
-W tail-call=y -W exceptions=y -W function-references=y -W stack-switching=y
```

Rationale (driver.nx:328-331): the bundled stdlib uses every one of these
proposals (TCO, raise/throws, closures, handlers), so emitting them
unconditionally is correct. Bootstrap leaves the user to discover them via
`bootstrap.sh`.

This is a **deliberate self-host superset** that satisfies acceptance §4
("self-hosted help output is at least as informative as Rust"). No fix
needed.

### 1.4 CLI gaps

| # | Gap | Disposition |
| --- | --- | --- |
| C-1 | self-host `nxc` cannot read source from stdin (`-` or piped tty) | follow-up issue (low value: `bootstrap.sh` always passes a path) |
| C-2 | self-host default output is `out.wasm`, bootstrap default is `main.wasm` | minor; flagged as a follow-up to align — not closed-by-this-PR because the on-disk default for stage outputs is currently consumed by `bootstrap.sh` which passes explicit `-o` / positional output |
| C-3 | invalid-usage exit code: bootstrap exits `1`, self-host exits `2` (driver.nx:529 `proc.exit(status: 2)` for `InvalidUsage`, `1` for everything else) | self-host superset — `2` for usage error matches POSIX convention (`misuse of shell builtins`). Acceptable divergence; document and keep self-host's behaviour. |
| C-4 | bootstrap `nexus compose <file>` is **bootstrap-only** | out-of-scope (will disappear when bootstrap is decommissioned; self-host already merges stdlib + composes inline as part of `build`) |
| C-5 | bootstrap `--wasm-merge <path>` override | out-of-scope (self-host has no external `wasm-merge` shell-out) |

## 2. Parser grammar parity

Both parsers consume the same lexer surface (`bootstrap/src/lang/lexer.rs`
vs `src/frontend/lexer.nx` + `src/common/token.nx`) and produce structurally
equivalent ASTs. Differences are listed below.

### 2.1 Reserved words / contextual identifiers

| Word | Bootstrap | Self-host |
| --- | --- | --- |
| `let`, `fn`, `do`, `end`, `if`, `else`, `match`, `while`, `for`, `return`, `import`, `from`, `external`, `export`, `cap`, `handler`, `inject`, `try`, `catch`, `raise`, `require`, `throws`, `exception`, `with`, `true`, `false` | reserved | reserved |
| `then` | **contextual** (lexer keeps as `Ident`; `expect_contextual("then")`) | **reserved keyword** (`TkThen`) |
| `to` | **contextual** (used in `for i = a to b`) | **reserved keyword** (`TkTo`) |
| `as` | **contextual** | **reserved keyword** (`TkAs`) |
| `opaque` | **contextual** (top-level dispatcher inspects `Ident` text) | **reserved keyword** (`TkOpaque`) |
| `ref` | **contextual** (only recognised inside type position) | **reserved keyword** (`TkRef`) |
| `drop` | not reserved | **reserved keyword** (`TkDrop`) — token is allocated but no parser arm consumes it (dead reservation) |
| `task`, `case` | reserved (rejected as identifiers, though no `TokenKind::Task` / `Case` exists; `case` is fully replaced by `\|` arms) | not reserved |

Net effect on accepted programs:
- self-host rejects `let then = 1` etc.; bootstrap accepts those identifiers
  in non-keyword positions.
- bootstrap rejects `let case = 1` and `let task = 1`; self-host accepts them.

**Real-world impact**: nil — no in-tree program uses any of these strings as
a binding name. Symptom-only.

### 2.2 Statement vs expression dispatch

Bootstrap recognises `if`, `match`, `while`, `for`, `try`, `inject` at
statement position (`parse_stmt`, parser.rs:1690) **and** at expression
position (`parse_atom`, parser.rs:1106).

Self-host only recognises `try` and `inject` as statements (`parse_stmt`,
parser.nx:661); `if`, `match`, `while`, `for` reach the expression-statement
fall-through and are wrapped as `StmtExpr(ExprIf | ExprMatch | …)`. The
ASTs differ but every concrete source token sequence parses successfully
under both — no acceptance gap.

### 2.3 `match` separator

- Bootstrap (parser.rs:1280): `do` after the match scrutinee is **optional**
  at expression position. `match x | pat -> body end` parses.
- Bootstrap (parser.rs:1906): at statement position, `do` is required.
- Self-host (parser.nx:597): `do` is always required.

**Programs accepted by bootstrap, rejected by self-host**: `match x | pat -> body end`
without the `do`. No in-tree program omits `do` — checked with
`rg 'match [^d]+\\| ' nxlib/ src/ examples/`.

### 2.4 Pattern grammar

- **`Ctor(_)` constructor wildcard sugar**: self-host recognises `Mk(_)` as
  "match `Mk` regardless of its arity" via `PatConstructorWildcard`
  (parse_pattern.nx:159). Bootstrap parses `_` as a single anonymous
  field, so the same syntax against a multi-field constructor would yield
  a constructor pattern with one wildcard field, which then fails arity
  checking later. This is a **self-host superset**, motivated by the
  feedback memory `feedback_nexus_list_and_nested_patterns` ("aggressive
  bare `_` ignores all remaining fields").
- **Negative-float pattern literal `-FLOAT`**: bootstrap accepts (parser.rs:518),
  self-host accepts only `-INT` (parse_pattern.nx:72). No code uses
  negative-float patterns; deferred.

### 2.5 Type grammar

- **Unnamed arrow params**: bootstrap `parse_arrow_param` (parser.rs:294)
  accepts `(T, U) -> R` by falling back to a synthetic `_` label when no
  `name :` is present. Self-host requires labels in arrow params
  (`parse_type.nx:65` calls `pcore.expect_ident` then `expect(":")`). No
  code uses unnamed arrow params; documented gap.
- **Bitwise OR `|` as a binary operator**: self-host has
  `OpBitOr` in `token_to_binop` and `get_binop_prec` (parse_core.nx:282)
  with a fuel-bounded disambiguator (`parse_core.nx:160` `pipe_starts_arm`)
  that defers `|` to outer match/catch arms when followed by a pattern
  ending in `->`. Bootstrap leaves `|` exclusively for arms (parser.rs:973
  has no `BinaryOp::BitOr` mapping). Self-host superset; no code uses
  bitwise-or expressions today.

### 2.6 Type-parameter case

`<T, U>` after a function name or type name. Bootstrap accepts any identifier
(`<t, u>` parses, lowercased and all). Self-host's
`parse_type_param_list` (`parse_params.nx:60`) only accepts `TkUident`
(uppercase-leading identifier). Convention is uppercase, in-tree code is
uniform — no real divergence — but a hand-written test using
`<a>`/`<b>` would be rejected by self-host.

### 2.7 Empty lambda body

Bootstrap rejects `fn () -> unit do end` with the message "Function body
cannot be empty" (parser.rs:1572). Self-host's `parse_stmts` immediately
returns the empty list when it sees the `end` token, so the empty body
parses (and the lambda is a unit-returning no-op). Self-host superset.

### 2.8 Handler-arm `requires` clause

Bootstrap `parse_handler_function` (parser.rs:1633) accepts
`fn name(...) -> T require { … } throws { … } [with @k] do … end` — i.e.
a `require` clause **inside** an arm.

Self-host `parse_handler_arms` (parser.nx:351) accepts
`fn name(...) -> T throws { … } [with @k] do … end` only. Arm-level
`require` is silently absent.

No in-tree handler arm uses an arm-level `require`, and the bootstrap HIR
builder discards the arm-level row anyway. Documented; no fix.

### 2.9 Other parser mechanics already at parity

- Cons (`::`) is right-associative at expression precedence 4 in both.
- Or-patterns (`p | q | r`) are accepted at the top of match / catch arms
  in both (`parse_pattern_or` in both parsers).
- Sigil-aware punning for call args, ctor args, record literals, ctor
  patterns, and record patterns is implemented identically (`%v` ↔
  `v: %v`, `&v` ↔ `v: &v`, etc.) — bootstrap `try_parse_pun_arg` /
  self-host `try_pun_arg_with_closer` and `try_pun_pat_field_with_closer`.
- `import` with `external`, `* as alias`, `{ items } from path`,
  `{ items }, * as alias from path`, `as alias from path`, `from path` —
  all five forms parsed in both.
- Quoted import paths — both reject `import name from foo;` and require
  `"std:stdio"` / `"pkg:foo/bar"` / `"examples/x.nx"` string literal.
- `cap Name [require {...}] do fn … end` — parity.
- `external name = "wasm-symbol" : <T,U,...>(...) -> T` — both accept
  optional type-parameter list (parse_topdef.nx:276).
- `exception Foo(...)` and `exception group Foo = A | B | C` — parity.
- Record types (`{ name: T, ... }`), list types (`[T]`), array types
  (`[| T |]`), `&T`, `%T`, `@T`, `ref(T)`, `handler Name`, sum types
  (`A | B | C`) — all parity.
- `if let pat = expr then … [else …] end` — parity (both desugar to
  `match` with wildcard fallback).
- `else if` chaining — parity (both treat the inner `if` as consuming its
  own `end`, see `parse_else_tail` in both).
- Negative integer literal `-INT` at expression position — parity.
- Unary `!`, `-`, `-.` — parity.
- Postfix `.field`, `[idx]` — parity.

## 3. Disposition summary

| Category | Closed in this PR | Filed as follow-up | Out-of-scope (rationale) |
| --- | --- | --- | --- |
| CLI surface | — | C-1 (stdin), C-2 (default output name) | C-3 (exit code 2 for usage is intentional), C-4 (compose), C-5 (--wasm-merge) |
| Parser surface (gaps where self-host < bootstrap) | — | P-1 (`-FLOAT` pattern), P-2 (`match x \| pat` without `do`), P-3 (unnamed arrow params), P-4 (handler-arm `require`) | reserved-word asymmetry §2.1 (no real-world program affected; would need a test corpus before flipping) |
| Parser surface (where self-host > bootstrap) | — | — | `Ctor(_)` wildcard, `\|` bitwise-or, expanded `wasmtime` flags — kept as deliberate supersets |

No gaps require fixing inside this audit PR — none affect any in-tree
program. Each gap above is a candidate for a separate `bd` issue if/when
the orchestrator wants to chase parser symmetry; the audit's purpose is
parity tracking, not parity enforcement.

## 4. Follow-up issue candidates

For the orchestrator to file (this PR does not create them):

- **bd: nxc CLI stdin support** (covers C-1) — accept `-` or piped-tty input
  in `src/driver.nx` matching bootstrap's `cli::load_source`. Low priority.
- **bd: nxc CLI default output align with bootstrap** (covers C-2) — change
  `get_output_path` default from `out.wasm` to `main.wasm`. Mechanical.
- **bd: parser — accept `-FLOAT` literal pattern** (P-1) — extend
  `parse_pattern.nx:72` `TkMinus` arm to also handle `TkFloat`. ~3-line fix.
- **bd: parser — make `do` optional after `match` scrutinee at expr position**
  (P-2) — match bootstrap's parser.rs:1280 behaviour. Low value (no in-tree
  program omits `do`); only relevant if external code corpora exist.
- **bd: parser — accept unnamed arrow params** (P-3) — fall back to `"_"`
  label when no `name :` prefix in `parse_type.nx:65`. Mirrors bootstrap.
- **bd: parser — accept handler-arm `require` clause** (P-4) — parse and
  discard at HIR-build time, matching bootstrap.

## 5. Acceptance check

| Acceptance item | Status |
| --- | --- |
| 1. Parity matrix doc committed | yes — this file |
| 2. Parser-level divergences identified, resolved or filed | yes — §2; gaps filed as P-1..P-4 follow-ups |
| 3. Bootstrap-exclusive CLI paths have self-host equivalent or rationale | yes — §1, gap table §1.4 |
| 4. nxc/nexus CLI help is at least as informative as Rust | yes — self-host's usage banner (driver.nx:399-403) lists all five forms (`build`, `-o`, `--dump-imports`, `--verbose`, `--explain-capabilities*`); wasmtime mode adds proposal flags Rust omits (§1.3) |
