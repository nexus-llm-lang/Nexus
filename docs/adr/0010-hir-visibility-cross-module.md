# ADR 0010 — HIR Visibility: Cross-Module Variant Strip via visible_enum_names

**Status**: Accepted  
**Date**: 2026-05-18  
**Issue**: nexus-8xke, commit 2af429f2  

## Context

Nexus supports cross-module imports.  When module B imports module A and A
defines a non-public enum `type Priv = Foo | Bar end`, module B should not be
able to pattern-match against `Foo` or `Bar` — they are encapsulation
boundaries.  This is the "D-Export non-public constructor leak" rule in the
type theory.

### The problem

Before nexus-8xke, the typecheck pass merged all imported HIR into a single
unified `HirProgram` and ran `build_enum_entries` over the combined definition
set.  At that point, every enum's constructors were visible to every module in
the compilation unit, regardless of the `pub` annotation on the original
definition.

Three attempts to fix this were made before the architecture landed.

### What does NOT work

**Attempt 1 & 2 — HIR-level variant-strip**: set `EnumDef.variants = []`
when `is_defining == 0 && !pub_` during HIR construction.  This made the
typecheck pass see empty variant sets for non-public enums from imported
modules — correct for typechecking.  But it also stripped the variant
information from the `HirProgram` that codegen consumes.  The LIR backend
uses `enum_defs.variants` for tag-width and memory layout.  Stripping variants
caused wasm validation errors ("expected i32, found i64") in
`walk_call_target` of `effect_analysis.nx`.

**Attempt 3 — typecheck-time `local_names` filter**: gate `build_enum_entries`
on `!is_local(ename, local_names) && !pub_`.  `local_names` is populated by
`extract_function_names` in `src/pipeline.nx`, which captures only TLLet
names — never enum names.  The gate therefore matched every non-public enum
including locally-defined ones, causing ~59 test regressions.

### Root cause

By the time `check_program` runs, the unified `HirProgram` has merged
definitions from every module.  The information about which module *defined*
an enum (`is_defining: i64`) is present during HIR construction but not
threaded through to the typecheck phase.  Reconstructing it at typecheck time
via `local_names` is architecturally incorrect because `local_names` has
different semantics.

## Decision

**Add `visible_enum_names: [string]` to `HirAccum` and `HirProgram`.
Populate at HIR construction time (where `is_defining` is known).
Consume in typecheck only — codegen sees the unmodified `HirProgram`.**

Implementation (nexus-8xke, commit `2af429f2`):

- In `hir.nx::process_top_level` at the `TLEnum` branch, include the enum
  name in `visible_enum_names` only when `is_defining == 1 || is_public`.
- Thread `visible_enum_names` through all `HirAccum` and `HirProgram`
  construction sites (~17 src files; destructures with trailing `_` absorb
  the new positional field without breaking — see HIR ctor positional
  underscore pattern).
- In `check.nx`: `build_enum_entries(defs, visible_names)` strips variants
  to `[]` for non-visible enums.  `register_enum_constructors` respects the
  same list.  These are **typecheck-only** operations on a typecheck-local
  copy of the env; the `HirProgram` passed to codegen is untouched.

### Generalisation

The `visible_enum_names` pattern generalises: any cross-module visibility
check (non-public record types, non-public caps, non-public exports) should
follow the same structure — a `visible_*: [string]` field on
`HirAccum`/`HirProgram`, populated at HIR build where `is_defining` is in
scope, consumed by typecheck.

## Consequences

- **Positive**: the typecheck pass correctly rejects pattern-matches against
  non-public constructors from imported modules.
- **Positive**: codegen is unaffected; no layout or tag-width bugs.
- **Positive**: the pattern is general; adding visibility for other HIR node
  kinds follows the same template.
- **Negative**: `HirAccum` and `HirProgram` have a new field that must be
  threaded through every construction site.  The positional trailing-`_`
  pattern absorbs the new field in destructures, but construction sites
  (`HirAccum(...)`) require explicit updates.  Grep for `HirAccum(` and
  `HirProgram(` to find all sites.
- **Negative**: four iterations were required to land a working fix.  The
  root cause (defining-module context lost by unification time) was not
  obvious from the first symptom (wasm validation error in codegen).
- **Neutral**: the `is_defining` field on HIR nodes exists to support exactly
  this use case; the fix is the intended use of that field, not a workaround.
