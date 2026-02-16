# Step 6: Rank-2 Polymorphism Rejection Test

## Achievements
- [x] Implemented `test_rank2_usage_fails` in `tests/ast_typecheck_tests.rs`.
- [x] Verified that a generic function parameter cannot be used polymorphically within the function body (e.g., called with both `Int` and `Bool`).
- [x] Confirmed standard Hindley-Milner behavior where type variables are monotypes within their scope.

## Verification
`cargo test --test ast_typecheck_tests` passes 5 tests, including `test_rank2_usage_fails`.
The error message from the failed type check confirms the conflict: `Type mismatch: I64 vs Bool` (or similar expected unification failure).
