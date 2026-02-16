# Nexus Development Status

## Implementation Checklist

### Core Calculus (Internal Representation)
- [ ] **Strict ANF Transformation**: Explicitly transforming the Surface AST into a restricted Core Calculus form where all intermediate results are named.
- [x] **Polymorphism**: Core support for quantified types (System F style internal representation).
- [ ] **Linear/Affine Logic Core**: Internal representation for resource tracking.
- [ ] **Effect Algebraic Representation**: Internal representation for ports and handlers (Effect handlers core).
- [x] **Tree-Walking Interpreter**: Direct execution of the Current AST (Proto-Core).

### Surface Language
- [x] **Functions**: Definition, labeled arguments, explicit return types.
- [x] **Control Flow**: `if/else`, `match` (basic pattern matching).
- [x] **Records & Field Access**: Syntax for structured data.

### Type System
- [x] **Generics**: Parametric polymorphism (`fn id<T>(...)`).
- [x] **Let-Polymorphism**: Hindley-Milner inference for `let` bindings.
- [x] **Type Inference**: Inference for expressions, calls, and literals.
- [x] **Rigid Type Variables**: Enforcing parametricity within generic function bodies.
- [x] **Records**: Basic record creation and field access checking.

### Memory & Resources
- [x] **Mutable References (`~`)**: declaration (`let ~x = ...`), mutation (`~x <- v`) and reference (`~x`).
- [x] **Namespace Separation**: Distinct namespaces for immutable (`x`) and mutable (`~x`) variables.
- [x] **Gravity Rules**: Enforcing stack-locality (no `Ref` in return types, no `~` capture in tasks, immutable variables restricted from holding `Ref`).
- [ ] **Linear Types (`%`)**: Parsing exists, but affine/linear semantics (consumption check) not yet implemented.

### Effects & System
- [ ] **Effect System**: set-based effect system. `{IO, Net}` tagging and checking.
- [ ] **Ports & Handlers**: Interface definition (parsed) but no dependency injection logic yet.
- [ ] **Module System**: `import` parsing exists, but file loading/resolution is mocked.

## Future Goals

### Native Compilation
Currently, Nexus runs on a tree-walking interpreter (`src/interpreter.rs`). The ultimate goal is to compile to native code for performance and portability.

- [ ] **MLIR / LLVM Backend**: Target MLIR (e.g., Linalg or a custom dialect) or LLVM IR to generate optimized native binaries.
- [ ] **Optimization Passes**: Leverage LLVM/MLIR for standard optimizations (DCE, inlining, etc.).

### Tooling
- [ ] tree-sitter grammar for syntax highlighting and editor support.
- [ ] **LSP Server**: For editor integration (autocompletion, go-to-definition).
- [ ] **Formatter**: Standard code formatter (`nexus fmt`).
