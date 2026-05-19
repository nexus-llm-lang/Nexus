# ADR 0007 — port → cap Terminology Consolidation

**Status**: Accepted  
**Date**: 2026-05-19  
**Issues**: nexus-n5zz, commit 47dea8df; earlier commit 7d123136 (keyword rename)  

## Context

Nexus's effect system uses capability objects to mediate access to system
resources.  These objects were historically called *ports* in the codebase:
the keyword was `port`, the type system referred to `port types`, and the
require-row was written as a port-row.

The word "port" has competing meanings in systems programming:

- Network port (a 16-bit TCP/UDP number).
- A channel endpoint in message-passing systems (CSP, Go channels).
- An interface or adaptor in component architectures.

None of these meanings align with the nexus usage.  *Capability* is the term
of art for "a token that grants permission to perform an operation":

- Object-capability systems (POLA, Capsicum, CloudABI) use exactly this term.
- The W3C WebAssembly Component Model uses "capability import" for imports that
  carry permissions.
- The WASI specification calls its resource handles "capabilities."

Having two names for the same concept in the same codebase caused friction:
documentation referenced "ports," code comments referred to "caps," and
newcomers had to learn the mapping.  After the `port` keyword was renamed to
`cap` (commit `7d123136`, 2026-04-24) there was still widespread use of "port"
in variable names, function names, error messages, and comments throughout
`src/`, `nxlib/`, and `tests/`.

### Scope of the duality

Before the consolidation:

- `src/ir/hir/build/defs.nx`: `HirPortDef`, `build_port_def`, `port_names`.
- `src/common/error.nx`: error messages like "port method not found".
- `src/backend/`: `lower_port_call`, `port_type_of`.
- Test fixtures: `port SomeCap do ... end` (keyword already fixed; remainder
  was in string literals and comments).
- `nxlib/stdlib/`: `port` in docstrings and type aliases.

The keyword `cap` was already the surface syntax; the internal names lagged.

## Decision

**Full rename of all "port" references to "cap" throughout `src/`, `nxlib/`,
and `tests/`, committed as a single atomic refactor (nexus-n5zz, commit
`47dea8df`).**

The rename covered:
- HIR node names (`HirPortDef` → `HirCapDef`, etc.)
- Function and variable names (`lower_port_call` → `lower_cap_call`, etc.)
- Error message strings
- Inline comments and docstrings
- Test fixture prose

The keyword `cap` was already the surface syntax since commit `7d123136`
(2026-04-24).  The consolidation at nexus-n5zz closed the gap between keyword
and implementation naming.

A preceding intermediate commit (`4c3351c6`) established the "cap is the only
name" policy before the full sweep.

### Rationale

- A single canonical term reduces onboarding friction.  Developers searching
  for "port" will not find capability-related code and vice versa.
- The rename is mechanical: every occurrence of the old term in non-user-facing
  positions can be safely replaced.  There is no semantic change.
- Committing as a single large refactor (rather than incrementally) avoids a
  long period where both terms exist simultaneously.

## Consequences

- **Positive**: the codebase has one name for one concept; `grep port` will
  not return capability-related hits.
- **Positive**: alignment with the W3C Component Model and WASI vocabulary
  makes the relationship to WebAssembly plumbing clearer for contributors
  familiar with those specifications.
- **Negative**: the commit is large (touching `src/`, `nxlib/`, `tests/`,
  `nexus.wasm`, `nexus` polyglot in one shot).  Any branch that diverged
  before the commit will see a large merge conflict surface.
- **Negative**: any external documentation, blog posts, or tutorials written
  before 2026-05-19 that reference "port" types will be stale.
- **Neutral**: the formal type theory (ADR 0006) uses "capability" throughout;
  the rename makes the implementation match the theory rather than inventing a
  third vocabulary.
