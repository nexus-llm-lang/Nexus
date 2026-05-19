# ADR 0008 — WASI Preview3 Migration Plan

**Status**: Accepted (Phase 1 shipped; Phase 2 in progress)  
**Date**: 2026-05-15  
**Issues**: nexus-vqmf (P2 OPEN), nexus-j8f2, nexus-hc62, commit ac0a9fb7  

## Context

Nexus currently targets **WASI preview1** under the threads proposal
(`-S threads`).  The threads requirement was introduced by nexus-f90i (ADR
0004) to support parallel `@` via WASI thread-spawn.  Preview1 + threads is
a supported configuration under wasmtime 44, but it locks out preview2 (which
disables the threads SIMD flag combination) and does not align with the
direction of the WebAssembly ecosystem.

**WASI preview3 (0.3.0-draft)** introduces native async / streaming semantics
via `future<T>` and `stream<T>` in the canonical ABI.  These primitives map
naturally onto nexus's `@`-thunk + `force_all` semantics:

- `@expr` produces a deferred computation (a thunk).
- `force_all` awaits a collection of thunks in parallel.
- `future<T>` is an owned handle to an asynchronous computation in the
  canonical ABI.

Migrating to p3 would allow nexus programs to be packaged as WebAssembly
components with native async — interoperable with the broader Wasm component
ecosystem (wasmtime serve, jco, etc.) — rather than relying on a
non-standard shared-memory threads hack.

### Status of the spec

As of 2026-05-15, WASI 0.3.0 is still **0.3.0-draft**.  Wasmtime 44 accepts
`-S p3=y` and `wasmtime serve` distinguishes WASIp2 vs WASIp3 components.  A
stable 0.3.0 release slipped past the February 2026 roadmap target.

### Concerns that were weighed and dismissed

- **Spec instability**: the user explicitly removed this from the decision
  matrix — accepted as cost-of-doing-business.  A breaking 0.3 spec change
  that actively breaks nexus emit would be grounds to revisit.
- **Codegen cost**: `src/backend/wasm/section.nx` and
  `src/backend/codegen/module_asm.nx` require changes for p3 component wrapping.
  Accepted as part of the migration.
- **wasmtime lock-in**: nexus already targets wasmtime (locked in `flake.nix`
  and build scripts).  Preview3 does not add a new lock-in.

### Options considered

| # | Option | Summary |
|---|--------|---------|
| 1 | Stay on preview1 + threads indefinitely | No migration cost; misses ecosystem alignment |
| 2 | Migrate to preview2 | Drops threads → loses parallel `@`; not acceptable |
| 3 | Migrate to preview3 with branch-by-abstraction | p1 path behind flag during 0.3-draft churn |
| 4 | Migrate to preview3 unconditionally (big bang) | High risk while spec is draft |

## Decision

**Option 3: staged migration to WASI preview3, keeping the preview1 + threads
path behind a build flag during the draft period.**

Phase 1 shipped (commit `ac0a9fb7`, nexus-vqmf): `--p3` flag on
`nexus build` emits a WASI preview3 component model wrapper alongside the
standard preview1 output.  The `force_all` future<T> wiring was added in
nexus-hc62 (commit `68797944`).

Phase 2 (nexus-vqmf, OPEN): replace the wasi-threads `lazy_spawn` /
`lazy_join` / `atomic.wait32/notify` implementation in
`nxlib/stdlib/runtime/lazy.nx` with canonical-ABI `future<T>` — the native
async path.  Bootstrap.sh / header.sh gain `-S p3=y` once the phase-2 path
is stable.

### Rationale

- p3 `future<T>` is a semantically cleaner model for nexus's `@`-thunk than
  a hand-rolled wasi-threads task struct with 24B shared-memory records.
- Component-model packaging opens nexus programs to the wasm component
  ecosystem (composition, wasm-pkg, etc.).
- Branch-by-abstraction (feature flag) keeps the preview1 path alive during
  spec churn; a rollback is possible if 0.3 makes a breaking change.

## Consequences

- **Positive**: nexus programs become first-class WebAssembly components;
  async interoperability with other component-model languages is possible.
- **Positive**: the `--p3-component` output is already available for
  evaluation (nexus-hc62); migration can proceed incrementally.
- **Positive**: if p3 stabilizes, the wasi-threads workaround (shared memory,
  atomic bumps, `wasi_thread_start` trampoline) can be deleted — net
  simplification of the runtime layer.
- **Negative**: while 0.3 is still draft, nexus.wasm targeting p3 may break
  on wasmtime version bumps.  The `--p3` flag explicitly marks this as an
  opt-in path.
- **Negative**: Phase 2 requires significant changes to
  `nxlib/stdlib/runtime/lazy.nx`, `src/backend/wasm/section.nx`, and
  `src/backend/codegen/module_asm.nx`.  The bootstrap fixpoint must be
  re-verified after each change.
- **Neutral**: the preview1 + threads path remains the default until Phase 2
  is complete.  Both paths carry the `-W threads=y,shared-memory=y` flags
  during the overlap period.
