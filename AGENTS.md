# Nexus Agents

This project is a language spec and implementation for LLM-friendly language.

## Nexus Language Skill

A Claude Code skill for writing Nexus code is available at `skills/nexus-lang/`. Install it to your environment:

```bash
npx skills add Nymphium/Nexus --skill nexus-lang
```

## Guidelines

- Follow TDD (Test Driven Development)
    - Prefer property-based testing where applicable
    - Don't have to write concrete syntax for every tests; use ASTs or type environments for whatever is sufficient
- Write clear commit messages
- Update documentation every after feature implementation or fixes
- Ensure `cargo test` and `cargo fmt` passes before committing
- This repository is Nix-managed; when development tooling/targets change, update `flake.nix` first.

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd sync
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
