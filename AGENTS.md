# Nexus Agents

Nexus is a self-hosting language and compiler designed for LLM-friendly development.

Language reference skill: `npx skills add nexus-llm-lang/Nexus --skill nexus-lang`

## Workflow

### Before every commit: lint and fmt

Run on the files you touched; fix everything before committing.

```bash
./nexus lint --check src/ nxlib/        # same scope as CI
./nexus fmt <file.nx>...                # rewrite in place
./nexus fmt --check <file.nx>...        # verify only
```

Do not relax a lint rule to get a commit through. If a relaxation is unavoidable,
explain it in the commit message.

CI formats `src/`, `nxlib/`, `tests/`, `examples/`, except
`tests/negative/parser/lex_line_comment_eof_column.nx` (intentionally has no
trailing newline) — never format that file.

### Before every push: test

```bash
./nexus test tests/                     # same as the CI test job
./bootstrap.sh --ci                     # also, when src/ changed (stage0 -> stage2 + seed match)
```

Push only when these pass on a clean tree.

Any change under `src/**` or `nxlib/**` must regenerate the seed with
`./bootstrap.sh`, and the new `nexus.wasm` (and `nexus` launcher) must land in
the same commit as that change (see README).

### Session completion

1. Create issues for remaining work (`bd create`).
2. Run lint/fmt/test as above if code changed.
3. Update issue status (`bd close`).
4. `git push` — work is not complete until it succeeds.
5. Hand off context for the next session.

## Conventions

- **Nix-managed repo**: when dev tooling or targets change, update `flake.nix` first.
- **Capability rows**: `require { ... }` and `throws { ... }` are sets; order is not
  load-bearing. List alphabetically (`require { Console, Fs }`,
  `require { PermClock, PermConsole, PermFs, PermProc }`). Drop `require` entirely
  when the body uses no capabilities.
- **bd issue IDs** (`nexus-XXXX`) do not go into source code or filenames; the bd
  database is local to the owner's machine. Put them in commit messages, PR
  descriptions, and the issue body.
- **Comments describe behavior, not design authority.** State what the code does
  (checkable against the code). Do not write "the sanctioned / canonical / only /
  intended way", "X counts as Y", or "must be done via Z" — an LLM has no design
  authority. You may transcribe intent the designer already stated; you may not
  invent it. If a rationale isn't established, ask instead of writing it as fact.

## Module placement (`src/`)

`src/tools/` holds CLI subcommand drivers: thin arg handling that calls an engine
(e.g. `tools/lint.nx`, `tools/fmt.nx`). Reusable engines live in their own
top-level dir (`lint/`, `fmt/`, ...) so other surfaces such as the LSP can use them.

Shared support is purpose-named: `syntax/` (ast/token/source_pos), `collection/`
(generic data structures), `diagnostics/` (errors), `ir/support/` (compiler-internal
helpers). Do not reintroduce a `common/` or `util/` grab-bag.
