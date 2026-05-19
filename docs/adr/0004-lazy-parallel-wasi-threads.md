# ADR 0004 — Lazy Parallel `@` via WASI Threads: Atomic-Bump Allocator, ~7x Bench

**Status**: Accepted  
**Date**: 2026-05-13  
**Issue**: nexus-f90i (Phase 8), commit 7efcc668  

## Context

Nexus's `@expr` (lazy thunk) operator was originally synchronous: `@` wrapped
an expression in a `call_indirect` that ran eagerly in the same thread.  The
old Rust embedder (`bootstrap/`) provided `__nx_lazy_spawn` / `__nx_lazy_join`
host functions for true parallelism, but those were deleted when `bootstrap/`
was removed (commit `753d5c2e`).

With the Rust embedder gone, `@` on a multi-core machine offered no
parallelism.  The `force_all` primitive blocked on a synchronous loop.
Benchmarks (`examples/bench_parallel.nx`) showed no speedup over sequential
execution.

### Technical constraints

A pure-wasm parallel implementation must:
1. Spawn real OS threads from within wasm (no host-side embedder).
2. Share linear memory between the main module and workers.
3. Not race on the allocator — nexus's bump allocator is a per-instance
   mutable global; two threads bumping the same pointer corrupt the heap.
4. Coexist with stack-switching continuations (algebraic effects /
   handlers use `cont.new` / `resume`).

### Options considered

| # | Option | Summary |
|---|--------|---------|
| 1 | WASI threads (`wasi_thread_start` export + shared memory) | Pure wasm, no embedder, standard proposal |
| 2 | Custom embedder via wasmtime Rust crate | Portable but reintroduces Rust dependency |
| 3 | JavaScript Web Workers (browser target) | Not relevant; nexus targets server-side WASI |
| 4 | Cooperative green threads (no true parallelism) | Simpler but does not use multiple cores |

## Decision

**Option 1: WASI threads with shared linear memory and an atomic-bump allocator.**

The implementation shipped across nexus-f90i phases 1-8:

- **Phase 1** (`6a532ed7`): wasm atomic memory ops (`i32.atomic.rmw.add`,
  `memory.atomic.wait32` / `notify`) added to codegen.
- **Phase 7** (`da2d2e96`): stdlib ported to Nexus source; `stdlib.wasm`
  pre-built blob eliminated.  All stdlib functions compiled into each program.
- **Phase 8** (`7efcc668`): parallel `@` via WASI threads.  Shared memory
  imported as `(import "env" "memory" ... shared)` and re-exported (a
  *defined* shared memory is per-instance under wasmtime 44 `-S threads`).
  Codegen emits `nexus:runtime/lazy` intrinsics and a synthesized
  `wasi_thread_start` trampoline.  `force_all` now real-parallel.

Post-Phase-8 cleanup (`37d2aad`): the allocator was unified.  All
codegen-emitted allocations and the inlined stdlib bump a single atomic cell
at linear offset 0, initialized to `heap_base` by a data segment.  The
`i32.atomic.rmw.add` bump eliminates cross-thread allocator races by
construction.

The stdlib blob was then fully inlined into every program (`06f46e8`):
`nxlib/stdlib_src/` moved into `nxlib/stdlib/`, deleting `stdlib.wasm` and
the `merge_with_stdlib` step.

### Gotcha: `proc_exit` + stack-switching SIGSEGV

Calling WASI `proc_exit` after using `cont.new` / `resume` (stack-switching)
crashes with exit code 139 under wasmtime 44.  The nexus runtime avoids this
by returning from `_start` normally.  Programs that manually call `proc_exit`
while a continuation stack is live are affected; this is a wasmtime bug, not a
nexus bug.

### Result

Benchmark `examples/bench_parallel.nx`: **~7x speedup** over sequential on an
8-core host.  `nexus test`: 64 passed / 2 failed / 66 total (the 2 failures
are pre-existing TyRef cliff bugs unrelated to threads).

## Consequences

- **Positive**: `force_all` on multi-core hardware delivers real parallelism
  with no embedder dependency.
- **Positive**: stdlib is fully Nexus source; no pre-built binary blobs in
  the repository.
- **Positive**: the atomic-bump allocator is safe for concurrent allocation;
  the per-thread race is eliminated by construction.
- **Negative**: `-S threads` (preview1) is required; this precludes WASI
  preview2 until nexus-vqmf (WASI p3 migration) ships.
- **Negative**: `proc_exit` + stack-switching crashes under wasmtime 44.
  Programs must return from `_start` rather than calling `proc_exit` while
  continuations are live.
- **Negative**: wasm alphabetical parameter ordering affects ABI for functions
  called positionally from outside the module (e.g., `wasi_thread_start`
  trampoline params must be ordered accordingly).
- **Neutral**: single `@expr` (non-`force_all`) remains a synchronous
  `call_indirect`; opt-in parallel spawn is deferred to a later issue.
- **Neutral**: `race` / `cancel` (first-of / drop semantics) need a runtime
  primitive not yet implemented; deferred.
