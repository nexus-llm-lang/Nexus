# Nexus Agents

This project is a language spec and implementation for LLM-friendly language.

## Guidelines

- Follow TDD (Test Driven Development)
    - Prefer property-based testing where applicable
    - Don't have to write concrete syntax for every tests; use ASTs or type environments for whatever is sufficient
- Write clear commit messages
- Update documentation every after feature implementation or fixes
- Ensure `cargo test` and `cargo fmt` passes before committing
- This repository is Nix-managed; when development tooling/targets change, update `flake.nix` first.
