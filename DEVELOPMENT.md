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

- [ ] Improve error messages
- [ ] Add more standard library functions
- [ ] Implement modules/imports
- [ ] Native compilation (LLVM/MLIR)
