# Development

## Current Status

- [x] Basic parsing and AST
- [x] Tree-walking interpreter
- [x] Type checking (Hindley-Milner with effects/mutability/linearity)
- [x] REPL (multiline input, incomplete-input accumulation, top-level definitions)
- [x] Effect System (row-based: syntax, row unification, polymorphism, raise/try-catch)
- [x] Linear Types (syntax, tracking, exactly-once, `match`/`if` integration, `Ref<Linear>` prohibition)
- [x] Modules & Imports (`import as`, selective import, external import)
- [x] Basic FFI with WASM (`external` functions, runtime calls, type-checked signatures)
- [x] Type annotations on bindings (`let x: T = ...`) and WASM scalar compatibility in typechecking
- [x] WASM-compatible scalar types (`i32`, `i64`, `f32`, `f64`) with numeric literal defaulting
- [x] Function values (named function as value) and inline lambda literals (`fn (...) -> ... do ... endfn`)
- [x] Closure safety semantics (no `Ref` capture, linear-capture makes closure linear, recursive local lambda with explicit annotation)
- [x] Linearity weakening at call sites (`T` can flow to `%T` parameters)
- [x] Initial stdlib modules (`nxlib/stdlib/stdio.nx`, `nxlib/stdlib/stdlib.nx`, `list.nx`, `array.nx`)
- [x] Rust sources for stdlib wasm at `src/lib/{stdio,stdlib}` (build outputs: `nxlib/stdlib/{stdio,stdlib}.wasm`)
- [x] Property-based tests for type/effect/reference/linear behaviors (proptest)

## Todo

### Core & Semantics (High Priority)
- [x] **Exhaustiveness Check**: Verify that `match` expressions cover all possible patterns.
- [x] User-defined ADTs and pattern matching on them.
- [x] **Linear Borrowing**: Allow temporary access to linear values without consuming them (`borrow` keyword).
- [x] **Refined Effect Checking**: Enhance unification to support flexible subset/superset relationships for effects beyond basic row polymorphism.
- [x] **Error Messages**: Implement rich diagnostics with source spans using Ariadne.
- [x] Stabilize higher-order semantics around closures (recursion/capture corner cases, effect inference edge cases).

### Infrastructure & Tooling (Medium Priority)
- [x] FFI: Interoperate with WASM libraries for performance-critical code and system access.
- [ ] **Standard Library**: Expand beyond List/Array/stdio (e.g. Map, richer collection/effect utilities).
- [ ] **Concurrency Runtime**: Implement actual parallel execution for `conc` blocks (currently sequential).
- [ ] Profiler, Testing and Benchmarking Tools: Measure performance and identify bottlenecks.
- [ ] Integrate `tree-sitter-nexus` parser into tooling/CI workflows.

### Future Goals (Low Priority)
- [x] **Modules & Imports**: File loading and namespace management (basic).
- [ ] **WASM Compilation**: LLVM IR / MLIR backend.
- [ ] **LSP Server**: Editor support.
- [ ] **Self-hosting**: Rewrite Nexus in Nexus.
