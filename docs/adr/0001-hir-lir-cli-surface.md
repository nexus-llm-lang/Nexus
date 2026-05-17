# ADR 0001 — `nexus hir` / `nexus lir` CLI surface

**Status**: Accepted  
**Date**: 2026-05-17  
**Issue**: nexus-jrco  

## Context

The IR-dump family (`nexus ast`, `nexus hir`, `nexus lir`, `nexus types`) was
introduced in nexus-ud6j to give LLM agents and compiler developers
machine-readable access to each compilation phase.  All four subcommands are
currently visible in the top-level `nexus help` output under "Context &
Introspection".

The question for nexus-jrco is whether **hir** and **lir** should remain at
the same level of visibility as **ast** and **types**, or whether their
intended audience justifies a narrower surface.

### Audience mapping

| Phase | Primary audience | Stability expectation |
|-------|----------------|-----------------------|
| `ast` | LLM agents, IDE tooling | Stable — parser shape is fixed |
| `types` | LLM agents, IDE tooling | Stable — per-binding type info |
| `hir` | Compiler developers | Unstable — schema evolves with resolver/HIR passes |
| `lir` | Compiler developers (backend) | Unstable — schema evolves with codegen |

Key observations from code archaeology:

- `nexus grep` depends on the **parser** (AST), not HIR.  No production tool
  or LLM agent integration in `src/tools/` calls the HIR or LIR dump paths.
- `src/tools/fix.nx` references `ast/types/hir/lir dump runners` as a list
  but only to note code-reuse rationale; it does not invoke hir/lir at
  runtime.
- Runtime tests for hir/lir (`ud6j_hir_dump_test.nx`, `stj8_hir_expr_tree_test.nx`,
  etc.) call internal APIs (`src/tools/ir_dump.nx`) directly — they are
  compiler regression tests, not user-workflow tests.
- HIR and LIR schemas are tightly coupled to in-progress refactors
  (nexus-95iz.30) and cannot be treated as stable API surfaces today.

### Options considered

| # | Option | Summary |
|---|--------|---------|
| 1 | Keep both (status quo) | hir + lir appear in top-level help, no gating |
| 2 | Gate behind `--dev` / `NEXUS_DEV` | Hidden from normal help; available to developers |
| 3 | Collapse to `nexus dump --phase=ast\|hir\|lir\|types` | One subcommand, phase as flag |
| 4 | Drop hir / lir entirely | Remove CLI surface; keep internal API only |

## Decision

**Option 2: gate `nexus hir` and `nexus lir` behind a `--dev` flag or
`NEXUS_DEV` environment variable.**

Rationale:

- **Keep**: hir and lir have genuine utility for compiler development
  (debugging resolve passes, backend codegen, diff-based regression).
  Dropping them (option 4) loses that without meaningful benefit — the
  commands already exist with working implementations.
- **Do not promote to stable**: the schemas change with every significant
  refactor.  Advertising them in top-level help implies stability the
  compiler does not provide.
- **Gate, not collapse**: collapsing to `nexus dump --phase=...` (option 3)
  adds a flag-parsing layer for minimal UX gain.  The current subcommand names
  (`nexus hir`, `nexus lir`) are self-documenting and already match
  developers' mental model.
- **Gate visibility only**: the commands remain functional and their internal
  APIs (`src/tools/ir_dump.nx`) are unchanged.  Existing runtime tests are
  unaffected.

The gating mechanism (driver-side check for `--dev` or `NEXUS_DEV`) is a
**separate implementation ticket** (follow-up to nexus-jrco); main.nx is
currently subject to large-scale refactor nexus-95iz.30 and must not be
modified concurrently.  This ADR records the decision so that the CLI
follow-up has clear acceptance criteria.

### Per-audience summary

| Audience | Recommended command | Notes |
|----------|-------------------|-------|
| Nexus user (writing programs) | `nexus build`, `nexus run`, `nexus typecheck` | No IR dump needed |
| LLM agent / IDE tooling | `nexus ast`, `nexus types`, `nexus explain`, `nexus context` | Stable schemas |
| Compiler developer | `nexus hir --dev`, `nexus lir --dev` | Post-gate; unstable schema |

## Consequences

- **Positive**: top-level help becomes less noisy for end users; stable vs
  unstable surfaces are clearly separated.
- **Positive**: no existing tests or tool integrations break (gating is
  visibility-only; the dump paths remain callable).
- **Negative / follow-up**: the `--dev` gate implementation is deferred to
  a separate ticket.  Until that ticket lands, hir/lir remain visible in
  top-level help (status quo is preserved in the interim).
- **Negative**: developers must know to pass `--dev` (or set `NEXUS_DEV`)
  after the gate lands.  This is acceptable given the compiler-developer
  audience.
- **Neutral**: `nexus ast` and `nexus types` are explicitly **not** gated;
  they are stable user-facing introspection commands.
