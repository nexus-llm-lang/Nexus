# Nexus Agents

Nexus is a self-hosting language and compiler designed for LLM-friendly development.

## Nexus Language Skill

```bash
npx skills add nexus-llm-lang/Nexus --skill nexus-lang
```

## Nix-managed Repository
Development tooling/targets change → update `flake.nix` first.

## Capability Row Ordering
`require { ... }` and `throws { ... }` rows are sets — order is not load-bearing.
By convention, list capabilities/exceptions alphabetically (e.g. `require { Console, Fs }`, `require { PermClock, PermConsole, PermFs, PermProc }`).
Drop `require { ... }` entirely if the body uses no caps.

## bd Issue IDs in Source

**Do not put bd issue IDs (`nexus-XXXX`) into source code or filenames.**
The bd database is local to this repo; references are unresolvable for any reader
outside the owner's machine.
Capture bd context in **commit messages**, **PR descriptions**, and the bd issue body
itself — never in source comments or test filenames.

## Session Completion
1. Create issues for remaining work (`bd create`)
2. Run quality gates if code changed
3. Update issue status (`bd close`)
4. Push to remote — work is NOT complete until `git push` succeeds
5. Hand off context for next session
