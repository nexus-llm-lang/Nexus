# Nexus Agents

This project uses AI agents to assist with development.

## Guidelines

- Follow TDD (Test Driven Development)
- Write clear commit messages
- Update documentation after feature implementation
- Ensure `cargo test` and `cargo fmt` passes before committing

## Context

- `src/ast.rs`: AST definitions
- `src/parser.rs`: Parser (Chumsky)
- `src/typecheck.rs`: Type system (HM + Linear + Effects)
- `src/interpreter/mod.rs`: Interpreter
- `tests/`: Integration tests

## Features

### Linear Types
Implemented in `src/typecheck.rs`.
- Sigil: `%`
- Type: `Type::Linear(Box<Type>)`
- Tracking: `linear_vars` in `TypeEnv`
- Enforcement: Consumption check in `Expr::Variable`, End-of-scope check in `check_function`/`infer_body`.
- Tests: `tests/linear_tests.rs`

### Effect System
Implemented in `src/typecheck.rs` and `src/interpreter/mod.rs`.
- Row-based: `Type::Row(Vec<Type>, Option<Box<Type>>)`
- Unification: Order-independent row unification in `src/typecheck.rs`.
- Polymorphism: Support for effect variables in function signatures.
- Exception: `Raise` expression and `Try-Catch` statement.
- Tests: `tests/effect_tests.rs`
