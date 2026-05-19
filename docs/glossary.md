# Nexus Codebase Glossary

Acronyms and abbreviations used in `src/`. Each entry lists: expansion,
concept summary, primary file references, and relevant ADRs or spec docs.

---

## HIR — High-level IR

**Expansion:** High-level Intermediate Representation

**Summary:** The first IR produced from the surface AST. HIR resolves names
(`RdrName` → canonical symbol), desugars handler bindings and lambda
expressions, and attaches span information. It is the representation consumed
by the typechecker and linearity analysis. Structurally close to the source;
does not perform A-normal-form conversion.

**Primary files:**
- `src/ir/hir/types.nx` — `HirFunction`, `HirProgram`, `HirParam`, etc.
- `src/ir/hir/hir.nx` — documentation pointer to submodules
- `src/ir/hir/build/` — AST → HIR conversion
- `src/ir/hir/pattern.nx` — pattern-shape conversion

**See also:** ADR-0001 (`docs/adr/0001-hir-lir-cli-surface.md`)

---

## MIR — Mid-level IR

**Expansion:** Mid-level Intermediate Representation

**Summary:** Produced from HIR after typechecking. MIR resolves cap-call
dispatch (selecting handler methods), inlines `inject` scopes, and desugars
`for` loops into `while` loops. Explicit lambda lifting is also orchestrated
at this level. MIR is the representation consumed by the LIR lowering pass.

**Primary files:**
- `src/ir/mir/types.nx` — `MirStmt`, `MirPattern`, `MirMatchCase`, `MirCtx`, etc.
- `src/ir/mir/mir.nx` — HIR → MIR lowering
- `src/ir/mir/cap.nx` — cap-call detection and resolution
- `src/ir/mir/lambda_lift.nx` — free-variable collection and lambda lifting

---

## LIR — Low-level IR

**Expansion:** Low-level Intermediate Representation

**Summary:** The final IR before WASM code generation. LIR converts the
program to A-Normal Form (ANF): every sub-expression is either an atom
(variable, literal) or a named temporary. Match expressions are compiled to
Maranget decision trees (`DecTree`). LIR also carries semantic-type
annotations that the WASM emitter uses to select the right store/load
instructions.

**Primary files:**
- `src/ir/lir/types.nx` — `LirFunction`, `LirStmt`, `LirAtom`, `LirExpr`, etc.
- `src/ir/lir/lir.nx` — MIR → LIR lowering (ANF, decision-tree emission)
- `src/ir/lir/match_tree.nx` — Maranget pattern-matrix → `DecTree` construction
- `src/ir/lir/func_lower.nx` — function/external lowering entry point
- `src/ir/lir_opt/` — LIR-level optimisation passes (DCE, constant folding, autovec)

**See also:** ADR-0001 (`docs/adr/0001-hir-lir-cli-surface.md`)

---

## TCMC — Tail-Call Modulo Constructor

**Expansion:** Tail-Call Modulo Constructor

**Summary:** A codegen optimisation that avoids stack growth for recursive
functions whose only non-tail work is constructing a linked-list cons cell.
Instead of allocating each cons on the way out of recursion, TCMC builds the
list in place by keeping a pointer to the tail slot (`prev_local`) and
back-patching it as the loop iterates. The result is a WASM loop (`op_loop`)
rather than a call chain.

Detection (`detect_tcmc`) identifies a matching call + constructor pair in
the function body. Emission (`emit_tcmc_cons_and_loop`) then generates the
in-place build loop and its initialisation preamble.

**Primary files:**
- `src/backend/codegen/tcmc.nx` — all TCMC types, detection, and emission
- `src/backend/codegen.nx` — interception point (`emit_stmt`) and probe in
  `compile_function_body`

---

## TCWF — Type-Check Well-Formedness

**Expansion:** Type-Check Well-Formedness (the `tcwf` module)

**Summary:** Enforces the spec-mandated `wfCap` and `wfThrow` predicates at
every row-introduction site (function arrows, cap method signatures, handler
annotations). The module checks that every entry in a `require { ... }` row
names a known capability, and that every entry in a `throws { ... }` row
names a declared exception variant. Use sites are not re-checked; once a row
passes introduction-site validation the downstream unification pipeline
inherits the invariant.

**Primary files:**
- `src/typecheck/tcwf.nx` — `wf_cap`, `wf_throw`, `check_program_wf_rows`
- `src/typecheck/check/main_validation.nx` — calls `tcwf` as part of the
  pre-typecheck whole-program walk

**See also:** `docs/spec/type-system-formal.md` §1 (SysCaps / wfCap / wfThrow)

---

## WF — Well-Formedness

**Expansion:** Well-Formedness

**Summary:** A broad predicate family asserting that a type or row expression
is structurally valid — e.g. every referenced capability or exception variant
is declared, quantifier kinds are consistent, and reference types do not
capture linear values. In the codebase `wf` appears as a prefix on functions
(`wf_cap`, `wf_throw`, `enforce_wf_ref_params`) and in commentary referencing
the formal spec predicates `wfCap(ρ_q)` and `wfThrow(ρ_e)`.

The `tcwf` module (above) is the primary enforcement point; individual helpers
also appear in the typechecker for reference-type well-formedness.

**Primary files:**
- `src/typecheck/tcwf.nx` — canonical enforcement
- `src/ir/shared/type_pred.nx` — `is_linear` and related structural predicates

**See also:** `docs/spec/type-system-formal.md` §1

---

## RDR / RdrName — Reader Name

**Expansion:** Reader Name (pre-resolution identifier)

**Summary:** A `RdrName` is an identifier as it appears in source text, before
import-alias resolution maps it to a canonical module-qualified symbol. It is
either `Unqual(name)` (a bare identifier like `foo`) or `Qual(alias, name)`
(a dotted reference like `list.map` where `list` is an import alias). The term
"reader name" is inherited from GHC Haskell's naming pipeline where
`RdrName` is the pre-renaming representation. After resolution, all `RdrName`
occurrences become canonical HIR names of the form `sanitized_path#name`.

The `rdr` prefix on functions (`rdr_from_dotted`, `rdr_to_string`, `rdr_occ`)
indicates they operate on this pre-resolution representation.

**Primary files:**
- `src/common/ast.nx` — `RdrName`, `Unqual`, `Qual` type definition
- `src/ir/shared/rdrname.nx` — `RdrName` ↔ string conversion, `canonical_name`
- `src/ir/shared/resolve.nx` — import-alias resolution using `rdrname`

---

## FB — Fallback

**Expansion:** Fallback (in pattern-match decision trees)

**Summary:** In the Maranget decision-tree algorithm used by the LIR lowering
pass, a _fallback_ is the sub-tree that handles all scrutinee values not
covered by any constructor or literal branch at a given switch node.
`DecCtorSwitch` holds a `fallback: Option<DecTree>` (absent when the switch
is known to be exhaustive); `DecLitSwitch` always has a `fallback: DecTree`
because literal matches are never structurally exhaustive. In the source code
the local variable `fb` consistently names this optional/required fallback
sub-tree.

**Primary files:**
- `src/ir/lir/match_tree.nx` — `DecCtorSwitch`, `DecLitSwitch`, and the
  algorithm that constructs the fallback from the default matrix

---

## BT — Backtrace (intrinsics and custom section)

**Expansion:** Backtrace

**Summary:** The `bt` prefix appears in two related contexts:

1. **Backtrace intrinsics** (`bt-depth`, `bt-frame`, `capture-backtrace`):
   compiler intrinsics under `nexus:intrinsic` that query the per-frame name
   ring buffer written by `__nx_main_wrap_bt_capture` at each `can_throw` call
   site. `bt-depth` returns the number of captured frames; `bt-frame(idx)`
   returns the function name at a given depth.

2. **`nx.bt.symbols` custom section**: a WASM custom section emitted by the
   code generator mapping every internal function's wasm index to its
   fully-qualified Nexus name. External disassemblers and source-map tooling
   use it to render frame IDs back to readable names.

Note: `bt_void` and `bt_i32` in `src/backend/wasm/defs.nx` are **WASM
blocktype** encoding constants (0x40 and 0x7F), not backtrace-related; the
`bt_` prefix there stands for "blocktype".

**Primary files:**
- `src/backend/codegen/intrinsics.nx` — `BtDepth`, `BtFrame`, `CaptureBacktrace`
  in the `Intrinsic` enum; `parse_intrinsic` dispatcher
- `src/backend/codegen/module_asm.nx` — `build_bt_symbols_section`
- `src/backend/codegen/context.nx` — `bt_capture_idx` field (per-function
  import slot for `capture-backtrace`)
- `src/backend/wasm/defs.nx` — `bt_void`, `bt_i32` (blocktype bytes; unrelated
  to backtrace)

---

## TAU / RHO — Type and Row Metavariables

**Expansion:**
- **tau (τ)**: type metavariable (Greek letter τ)
- **rho (ρ)**: row metavariable (Greek letter ρ)

**Summary:** In the formal type-system specification (`docs/spec/type-system-formal.md`)
and in comments throughout the typechecker, τ denotes a type-kinded
metavariable and ρ denotes a row-kinded metavariable (a sequence of labelled
entries used for `require { ... }` and `throws { ... }` rows). The distinction
matters for the `kindOf` predicate: a quantifier `X` has kind `Type` if it
occurs in a τ-position but not a ρ-position, and kind `Row` if it occurs in a
ρ-position (a row tail) but not a τ-position. Occurrences in both or neither
are static errors.

In code, `occurs_tau(x, typ)` and `occurs_rho(x, typ)` are the two
structural recursions that implement these predicates. The names `tau` and
`rho` also appear in comments explaining unification variables and in
`infer.nx` for fresh metavariable generation.

**Primary files:**
- `src/typecheck/check/occurs.nx` — `occurs_tau`, `occurs_rho`,
  `enforce_kindof_quantifiers`
- `src/typecheck/infer.nx` — unification and metavariable instantiation
- `src/typecheck/check.nx` — references `occurs_tau`/`occurs_rho` in module doc

**See also:** `docs/spec/type-system-formal.md` §"Polymorphism Introduction"
(spec lines L548–L571)
