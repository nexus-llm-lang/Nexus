# ADR 0009 — Memo Plan / Arena heap-mark/reset for Codegen

**Status**: Accepted  
**Date**: 2026-05-01  
**Issues**: nexus-e6uh, nexus-bavy, commits b6577878, ec5e61ed  

## Context

Nexus programs can contain pure functions: functions with no capability
requirements, no exception rows, and deterministic outputs.  Repeated calls
to pure functions with the same arguments produce the same value.

For long-running programs (e.g., the nexus compiler itself, which calls pure
type predicates millions of times during typecheck), redundant pure calls are
a significant source of latency.  The question is whether the compiler backend
can automatically insert call-site caches for eligible calls, and if so, what
memory strategy should govern the cache lifetime.

### Purity determination

A call site is memo-eligible if:
1. The callee has no `require` row entries (no capability side effects).
2. The callee has no `throws` row entries (cannot raise exceptions).
3. All arguments are scalar (i64 / i32) or pointer-stable string handles.
4. The callee has no mutable reference captures.

These conditions are checkable at LIR time without a full alias analysis.

### Cache lifetime and the arena question

A memoized call site needs storage for:
- A "populated" flag (i32, 0 = empty).
- The cached return value (i64 or smaller).

Two memory strategies were considered:

| Strategy | Description | Trade-off |
|----------|-------------|-----------|
| Static slot | A per-call-site i32 + i64 pair in the data segment | Zero allocation cost; cache lives forever; cannot be invalidated |
| Arena mark/reset | Cache in the bump arena; reset via `heap_mark`/`heap_reset` | Enables scoped invalidation; adds indirection |

The static-slot approach was chosen for the MVP (nexus-e6uh, commit
`b6577878`): each memo-eligible call site gets a **single-slot cache**
emitted as a fixed data-segment pair.

The name "memo plan" refers to the analysis pass (`MemoPlanner`) in
`src/backend/codegen/memo.nx` that identifies eligible sites and assigns
them slot indices.

### Arena heap-mark/reset as a future extension

The nexus runtime already provides `heap_mark` / `heap_reset` primitives
(carryovers from the Rust-era arena allocator, now the unified atomic-bump
arena).  A more sophisticated memo strategy could:

1. At the start of a major computation phase, record `heap_mark`.
2. Store memoized results in the arena.
3. At the end of the phase, `heap_reset` to the mark — invalidating all
   cached results in O(1) and reclaiming memory.

This was **not implemented** in the MVP.  The static single-slot approach is
sufficient for the hot paths identified (pure type predicates with scalar
arguments).  The arena strategy is noted here for when memo results are
larger-than-word or when invalidation semantics are needed.

## Decision

**Single-slot static memo cache emitted by `MemoPlanner` at eligible pure
call sites (nexus-bavy / nexus-e6uh).**

The `MemoPlanner` in `src/backend/codegen/memo.nx`:
- Identifies calls satisfying the purity conditions above.
- Assigns each a static data-segment slot (8 bytes: 4B flag + 4B value for
  i32 returns, or 4B flag + 8B value for i64 returns).
- Codegen wraps the call: check flag → hit: return cached; miss: call,
  store, return.

The arena heap-mark/reset mechanism is noted as the extension path for
larger cached values and for invalidation — deferred to a future issue.

## Consequences

- **Positive**: hot pure calls (type predicates, string hashing, length
  checks) are cached with ~3 wasm instructions of overhead on the hit path.
- **Positive**: the memo slots are in the data segment — no allocation, no
  GC interaction, no thread-safety concern (each wasm instance has its own
  linear memory).
- **Positive**: the `MemoPlanner` is modular and can be extended with
  additional eligibility conditions without changing codegen.
- **Negative**: single-slot caches are only effective for functions called
  with the same arguments on consecutive or nearby invocations.  For functions
  called with N distinct argument sets in a loop, the single slot thrashes.
- **Negative**: cache slots are never invalidated or reclaimed for the static
  strategy.  For programs with warm-up phases followed by different workloads,
  stale cached values may persist (though for pure functions this is correct —
  it only wastes the slot).
- **Negative**: the arena mark/reset extension requires a disciplined
  "phase boundary" abstraction that does not yet exist in the runtime.
- **Neutral**: the codegen split (nexus-2guw.6, commit `db6e01e6`) extracted
  memo into `src/backend/codegen/memo.nx`; future changes to memo eligibility
  are isolated to that file.
