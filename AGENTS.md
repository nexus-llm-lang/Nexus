# AI Agent Documentation

## Role
**Nexus AI Engineer**: A specialized autonomous agent capable of implementing, testing, and verifying the Nexus programming language.

## Capabilities
- **Language Design Implementation**: Translating `INIT.md` specifications into Rust code.
- **Compiler Construction**:
    - **Parser**: Using `chumsky` for robust parsing of Nexus syntax (strict ANF, sigils, labeled args).
    - **AST**: Designing type-safe Abstract Syntax Trees.
    - **Type System**: Implementing Hindley-Milner type inference with Let-Polymorphism and Rigid Type Variables for generics.
    - **Interpreter**: executing Nexus code with environment management and built-in mocking.
- **Verification & Testing**:
    - **Property-Based Testing**: verifying theoretical properties like "Identity of Indiscernibles" and "Rank-2 Rejection".
    - **AST Testing**: Direct AST construction to isolate Type Checker logic.
    - **Unit Testing**: verifying specific language features (References, Generics).

## Protocols
1.  **Step-by-Step Implementation**: Building the "Core" language first, then adding features (Generics, References).
2.  **Test-Driven Development (TDD)**: Writing tests (e.g., `tests/ref_tests.rs`) before or alongside implementation.
3.  **Strict Adherence to Spec**: Following `INIT.md` for syntax (`~` for mutable refs) and semantics.
4.  **Self-Correction**: Fixing compiler warnings and logic errors (e.g., rigid type handling in `check_function`) autonomously.
5.  **Code Formatting**: Always run `cargo fmt` after any code modification to maintain consistent style.

