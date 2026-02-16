# Step 7: Property-Based Testing for Type System Theorems

## Achievements
- [x] Implemented `tests/ast_property_tests.rs` verifying fundamental theorems of Let-Polymorphism.
    - [x] **Identity of Indiscernibles**: `id<T>` works universally.
    - [x] **K Combinator**: Correctly generalizes and instantiates multiple type variables (`A`, `B`).
    - [x] **Higher-Order Functions**: Verified `apply<A, B>(f: A->B, x: A) -> B` works with concrete functions (`to_int`).
    - [x] **Soundness (Occurs Check / Rigid Types)**: Verified `self_apply<T>(f: T) { f(f) }` is rejected. In Nexus, this is rejected due to `T` being rigid (UserDefined) vs `Arrow`, which correctly prevents unsafe recursion/infinite types.
    - [x] **Parametricity**: Verified that `fn bad<T>(x: T) -> T { return 42 }` is rejected, proving implementations cannot assume concrete types for generics.

## Verification
`cargo test --test ast_property_tests` passes all 5 tests.
