# Step 5: AST-Based Type Checker Testing

## Improvements
- [x] Refactored `tests/ast_typecheck_tests.rs` to construct ASTs directly, isolating the Type Checker from the Parser.
- [x] Tested specific scenarios:
    - **Identity Function**: Verifies basic generic function checking.
    - **Let Polymorphism**: Verifies that a generic function can be bound to a variable and instantiated with different types.
    - **Type Mismatch**: Verifies basic type errors (e.g., Bool vs I64).
    - **Generics Rigidity**: Verifies that a generic function implementation cannot return a concrete type (e.g., `fn id<T>(x: T) -> T { return 10 }` fails).

## Fixes
- Fixed `check_function` in `src/typecheck.rs` to treat generic type parameters as **Rigid Type Variables** (using `UserDefined`) instead of unification variables (`Var`). This ensures that the implementation must respect the generic signature and cannot simply unify `T` with a concrete type like `I64`.

## Verification
`cargo test --test ast_typecheck_tests` passes all 4 tests.
