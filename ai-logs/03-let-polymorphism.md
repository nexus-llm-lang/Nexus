# Step 3: Let-Polymorphism and Type Checking

## Achievements
- [x] Extended AST (`Type`, `Function`) to support generics and function types (`Arrow`).
- [x] Updated Parser to handle generic type parameters (`fn id<T>(...)`) and type variables.
- [x] Implemented a Hindley-Milner style Type Checker in `src/typecheck.rs`.
    - **Environment**: `TypeEnv` storing `Scheme`s.
    - **Inference**: `infer` function for expressions.
    - **Unification**: `unify` function with substitution composition.
    - **Polymorphism**: `generalize` (for `let` and `fn`) and `instantiate` (for usage).
- [x] Verified with `step6_poly.nx` which defines and uses a generic identity function `id<T>`.

## Key Features
- **Generic Functions**: `fn id<T>(x: T) -> T`.
- **Let Polymorphism**: `let i = id(x: 10)` infers `i` as `I64`.
- **Explicit Types + Inference**: Respects explicit annotations while inferring usage.

## Notes
- Nexus syntax requires full labeled arguments, so `id(10)` is invalid; `id(x: 10)` is correct.
- `Expr::Constructor` was added to parser but currently only used for `Result` variants (`Ok`, `Err`).
