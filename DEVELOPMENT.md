# Development

## Current Status

- [x] Basic parsing and AST
- [x] Tree-walking interpreter
- [x] Type checking (Hindley-Milner with effects/mutability)
- [x] REPL
- [x] Effect System (Row-based)
    - [x] Syntax (`effect { E1, E2 | r }`)
    - [x] Row Unification (order-independent)
    - [x] Effect Polymorphism
    - [x] Raise and Try/Catch (Runtime & Typecheck)
- [x] Linear Types
    - [x] Syntax (`%Type`)
    - [x] Linear tracking (consumption)
    - [x] Exactly-once enforcement
    - [x] Integration with `match` and `if`
    - [x] Prohibition of `Ref<Linear>`

## Todo

### Core & Semantics (High Priority)
- [ ] **Exhaustiveness Check**: Verify that `match` expressions cover all possible patterns.
- [ ] **Linear Borrowing**: Allow temporary access to linear values without consuming them (e.g., `borrow` keyword or analysis).
- [ ] **Refined Effect Checking**: Enhance unification to support flexible subset/superset relationships for effects beyond basic row polymorphism.

### Infrastructure & Tooling (Medium Priority)
- [ ] **Error Messages**: Implement rich diagnostics with source spans using Ariadne.
- [ ] **Standard Library**: Add basic data structures (List, Map) and utilities.
- [ ] **Concurrency Runtime**: Implement actual parallel execution for `conc` blocks (currently sequential).

### Future Goals (Low Priority)
- [ ] **Modules & Imports**: Implement file loading and namespace management.
- [ ] **Native Compilation**: LLVM IR / MLIR backend.
- [ ] **LSP Server**: Editor support.
- [ ] **Self-hosting**: Rewrite Nexus in Nexus.
