# Step 1: Core Language Implementation Plan

## Goal
Implement the "core" of the Nexus language.

## Scope
*   **Base Types**: `i64`, `bool`, `str`, `unit`.
*   **Expressions**: `let`, function calls (labeled), raw strings, literals.
*   **Control Flow**: `if ... then ... endif`, `match ... do ... endmatch`.
*   **Functions**: `fn ... do ... endfn` with labeled arguments.
*   **Type Declarations**: `type Name = ...`.
*   **Exclusions**: `~` (mutable refs), `%` (linear types), user-defined effects, module system.

## Action Items
1.  Define AST in `src/ast.rs`.
2.  Implement Lexer in `src/lexer.rs`.
3.  Implement Parser in `src/parser.rs`.
4.  Create a simple REPL or file runner in `src/main.rs`.
