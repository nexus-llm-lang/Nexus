# ADR 0001 — `nexus.wasm` as Stage0 Seed

- Status: Accepted (2026-05-06)
- Scope: any PR that touches `src/**` or `nxlib/**` (including `stdlib`, which
  is a symlink into `nxlib/stdlib`).
- Owners: compiler maintainers.

## Context

Nexus is self-hosted: the compiler in `src/**` and the standard library in
`nxlib/**` are written in Nexus and compile themselves. The only piece of
the toolchain not written in Nexus is the small Rust shim under
`bootstrap/` that knows how to host a wasm component.

The committed binary `nexus.wasm` at the repo root is the **Stage0 seed** of
the bootstrap. It is *not* a build artifact in the usual sense: it is a
checked-in input that anyone (CI, contributors, downstream packagers) needs
in order to compile the current `src/**` without having a working Nexus
compiler installed first. Without a current seed, a fresh checkout cannot
build itself.

`bootstrap.sh` makes this concrete:

1. Builds the Rust host (`bootstrap/target/release/nexus`).
2. Stage 0: produces `stage0.wasm` from `src/driver.nx` (using either the
   cached `nexus.wasm` or `nexus build` from the Rust binary).
3. Stage 1: runs `stage0.wasm` to compile `src/driver.nx` → `stage1.wasm`.
4. Stage 2: runs `stage1.wasm` to compile `src/driver.nx` → `stage2.wasm`.
5. Verifies `stage1.wasm == stage2.wasm`. The success log line is
   `Fixed point reached! stage1.wasm and stage2.wasm are identical.`
   (`bootstrap.sh:143`).
6. Installs `stage1.wasm` as the new `nexus.wasm`.

CI runs `./bootstrap.sh --ci` on every push and pull request
(`.github/workflows/ci.yml`, job `bootstrap-verify`). A non-fixed point or
an ill-formed `stage2.wasm` fails the job.

The implication: if a PR changes `src/**` or `nxlib/**` without also
updating `nexus.wasm`, the committed seed no longer matches the committed
sources. The bootstrap may still pass *transiently* (because Stage 0 is
allowed to be older than Stage 1 — the fixed-point check tolerates one
seed/source skew), but as soon as the new sources rely on a feature the
old seed cannot parse or lower, the next person to run `bootstrap.sh` from
a clean checkout cannot rebuild the compiler. The repo becomes
unbuildable from its own seed.

## Decision

Every PR that modifies compiler or stdlib source — anything under `src/**`
or `nxlib/**` — must regenerate `nexus.wasm` so the committed seed remains
buildable from the committed sources.

Concrete rules:

1. **Author regenerates the seed.** After the source change is finalized,
   the author runs `./bootstrap.sh` locally and includes the resulting
   `nexus.wasm` in the same PR.
2. **Commit pattern.** The seed regeneration goes in a dedicated commit
   with subject:

   ```
   chore(wasm): regenerate nexus.wasm after <bd-id>
   ```

   `git log --oneline --grep "regenerate nexus.wasm"` shows the established
   pattern (e.g. `8230b7e chore(wasm): regenerate nexus.wasm after qr4c
   trailing-comma support`). The seed regen commit may also be the same
   commit as the source change when the change is small and self-contained;
   the dedicated commit is preferred because it isolates the binary diff
   from the source diff in review.
3. **Reviewer check.** Reviewers reject PRs that touch `src/**` or
   `nxlib/**` and do not update `nexus.wasm`, unless the PR explicitly
   relies on the forward-compat protocol below (in which case a follow-up
   PR with the seed regeneration is required, and must land before any
   further compiler-source change).
4. **CI enforcement.** `./bootstrap.sh --ci` fails when stage1 and stage2
   differ, when stage2 is ill-formed, or when stage0 cannot parse current
   sources. This catches stale seeds in the common case. CI does **not**
   independently check that the committed `nexus.wasm` matches what
   regeneration would produce — that responsibility stays with the author
   and the reviewer.

## Consequences

For authors:

- Local bootstrap (`./bootstrap.sh`, no `--ci`) is part of the normal edit
  cycle for compiler-source PRs.
- The diff in a compiler-source PR usually contains both the source delta
  and a `nexus.wasm` binary delta. `.gitattributes` already marks
  `nexus.wasm` as binary so it does not pollute textual diffs.

For reviewers:

- A `src/**`/`nxlib/**` PR without a paired `nexus.wasm` update is a red
  flag. Either the seed is missing or the change is on the forward-compat
  path (see below) and a follow-up regen is required.
- Inspect the commit history of the PR for the `chore(wasm): regenerate
  nexus.wasm after <bd-id>` commit.

For CI:

- `bootstrap-verify` is the load-bearing gate. If it goes flaky, treat the
  flake as a P0: silent seed/source drift is the failure mode.

## Forward-compat protocol (2-step) for syntax / IR additions

When a change introduces *new syntax or a new IR construct* that the
**current** seed cannot parse or lower, a single PR cannot both regenerate
the seed and use the new construct in `src/**`/`nxlib/**` — the old seed
would refuse to compile the new sources, so Stage 1 would never produce a
valid Stage 1 binary, so the seed cannot be regenerated in the same step.

The fix is to split the change across two PRs:

1. **PR-1: parser/IR-tolerant seed.** Teach the compiler to *accept* the
   new syntax (or new IR shape) without requiring it. The new construct
   parses, type-checks, and lowers, but no source under `src/**` or
   `nxlib/**` actually uses it yet. Regenerate `nexus.wasm` in the same
   PR. After PR-1 lands, the committed seed understands the new construct.
2. **PR-2: convert sources.** Change `src/**`/`nxlib/**` to use the new
   construct. The seed from PR-1 already accepts it, so Stage 1 succeeds,
   the fixed point holds, and `nexus.wasm` regenerates normally.

Both PRs follow the standard rule: each updates `nexus.wasm` with the
`chore(wasm): regenerate nexus.wasm after <bd-id>` commit.

### Worked example: adding a new operator token `<+>`

Suppose we want to add a new infix operator `<+>` and use it in the stdlib.
The current seed's lexer rejects the `<+>` token outright.

A *single* PR that adds the lexer/parser rule, the typing rule, and uses
`<+>` in `nxlib/stdlib/list.nx` would fail at Stage 1:

```
Stage 1: wasmtime run stage0.wasm src/driver.nx ...
  parse error: unexpected token '<+>' at nxlib/stdlib/list.nx:42
```

because `stage0.wasm` (the committed seed) was built before `<+>` existed.

The 2-step landing:

**PR-1 — `feat(parser): recognize <+> token (nexus-XXXX)`**

- Lexer recognizes `<+>`.
- Parser produces an AST node, type-checker assigns a type, lowering emits
  the right MIR/LIR/WASM.
- No `.nx` file under `src/**` or `nxlib/**` uses `<+>` yet — the
  committed sources still parse with the *old* seed because they contain
  no `<+>` occurrences.
- `./bootstrap.sh` succeeds: old seed compiles new sources (no `<+>`
  appears in them), Stage 1 understands `<+>` going forward, fixed point
  holds.
- Commit pair:
  ```
  feat(parser): recognize <+> token (nexus-XXXX)
  chore(wasm): regenerate nexus.wasm after XXXX
  ```

**PR-2 — `feat(stdlib): use <+> in list combinators (nexus-YYYY)`**

- `nxlib/stdlib/list.nx` (and friends) now contain `<+>`.
- Stage 0 is the seed from PR-1, which already understands `<+>`. Stage 1
  succeeds, fixed point holds.
- Commit pair:
  ```
  feat(stdlib): use <+> in list combinators (nexus-YYYY)
  chore(wasm): regenerate nexus.wasm after YYYY
  ```

The same shape applies to: new keywords, new pattern forms, new top-level
declarations, new MIR/LIR opcodes consumed during deserialization of any
on-disk format the compiler reads, and any change to the wire shape of an
intrinsic the seed must call. If in doubt — i.e. you cannot be sure the
old seed accepts the new sources — split the PR.

## Non-goals

- This ADR does not specify how `nexus.wasm` is built (that is
  `bootstrap.sh`).
- It does not specify a process for *replacing* the seed wholesale (e.g.
  in response to a security issue in the seed). That is a separate ADR
  if it ever becomes necessary.
- It does not require deterministic byte-for-byte reproducibility of
  `nexus.wasm` from sources — only the Stage 1 / Stage 2 fixed point.
  Reproducibility is a stronger property worth tracking separately.
