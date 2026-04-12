# Nexus Agents

Nexus is a self-hosting language and compiler designed for LLM-friendly development.

## Nexus Language Skill

```bash
npx skills add Nymphium/Nexus --skill nexus-lang
```

## Nix-managed Repository
Development tooling/targets change → update `flake.nix` first.

## Session Completion
1. Create issues for remaining work (`bd create`)
2. Run quality gates if code changed
3. Update issue status (`bd close`)
4. Push to remote — work is NOT complete until `git push` succeeds
5. Hand off context for next session
