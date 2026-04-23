---
layout: default
title: Refactor Baseline (epic ij0e)
---

# Refactor Baseline — Epic `ij0e`

Regression safety net for the `src/` cohesion/coupling/efficiency refactor.
Captured before any phase-1/2/3/4 work; every subsequent commit in the epic is
compared against this snapshot.

## Scope

| Field | Value |
|---|---|
| Commit | `9f255298971aedd455d2f3eaf16c778f91caf207` |
| Date | 2026-04-23 |
| Host | Intel Core i5-1038NG7 @ 2.00 GHz, 8 logical cores, macOS 26.4.1 (x86_64) |
| Toolchain | rustc 1.93.0, wasmtime 38.0.3, wasm-tools 1.239.0 |
| Build profile | `cargo build --release`, stage2 produced via `./bootstrap.sh --ci` |

## 1. Self-compile fixed point

Running `./bootstrap.sh --ci` on this commit verifies `stage1.wasm == stage2.wasm`
byte-for-byte. Recorded artifacts:

| Artifact | Value |
|---|---|
| `stage1` byte size | 807,567 |
| `stage2` byte size | 807,567 |
| `cmp -s stage1 stage2` | identical |
| `wasm-tools validate stage2` | passes |
| SHA-256 (stage1 = stage2 = `nexus.wasm`) | `4892522a92c826cd28e2ab42b6ae9210cb09b34f2e5f921f27452ae1274c01a1` |

**Regression rule**: every commit in this epic MUST either preserve this SHA
(pure refactor, no semantic change) or produce a new fixed point where
`stage1 == stage2` still holds. If stage2 diverges from stage1, revert.

## 2. Fixture / test pass

Command: `cargo test --release --manifest-path bootstrap/Cargo.toml -- --test-threads=4`

| Binary | passed | failed | ignored |
|---|---|---|---|
| lib unit tests (binary 1) | 48 | 0 | 0 |
| lib unit tests (binary 2) | 3 | 0 | 0 |
| integration (main test bin) | 605 | 0 | 1 |
| doc-tests | 0 | 0 | 0 |
| **total** | **656** | **0** | **1** |

Integration suite wall-clock: **58.45 s** (release).

The single ignored test is `nxc::codegen::lazy_thunk_syntax` — tracked by
epic `dtq5` (restore `@` lazy semantics) and issue `4y14`; not in this epic's
scope.

**Regression rule**: each commit in this epic must keep `passed == 656` and
`failed == 0`. New failures must be fixed or explicitly scoped; ignoring a
previously-green test without a new bd issue is forbidden.

## 3. Compile-time baseline (median of 3 runs)

All measurements use the self-hosted compiler: `wasmtime run <core-flags>
nexus.wasm <input> --verbose /tmp/out.wasm`. The Rust bootstrap is not the
target of this refactor, so its timing is not tracked here.

Format: `median (min–max)` in milliseconds.

### `examples/fib.nx` (small, 13 LOC)

| phase | median (ms) | min–max |
|---|--:|---:|
| read source | 1 | 1–2 |
| lex | 0 | 0–0 |
| parse | 0 | 0–0 |
| HIR | 8 | 7–9 |
| resolve | 0 | 0–0 |
| typecheck* | 2 | 1–2 |
| MIR | 1 | 0–1 |
| LIR | 0 | 0–0 |
| optimize | 1 | 1–1 |
| codegen | 2 | 1–2 |
| write | 4 | 4–5 |
| **total** | **19** | **18–19** |

### `examples/web_server.nx` (medium, 80 LOC)

| phase | median (ms) | min–max |
|---|--:|---:|
| read source | 0 | 0–1 |
| lex | 1 | 1–1 |
| parse | 0 | 0–0 |
| HIR | 21 | 19–25 |
| resolve | 6 | 6–7 |
| typecheck* | 14 | 13–15 |
| MIR | 2 | 2–2 |
| LIR | 4 | 4–5 |
| optimize | 2 | 1–2 |
| codegen | 5 | 5–6 |
| write | 5 | 5–7 |
| **total** | **56** | **54–58** |

### `src/driver.nx` (self-host large, 309 LOC entry + ~18k LOC transitive)

| phase | median (ms) | min–max | % of total |
|---|--:|---:|--:|
| read source | 0 | 0–1 | ~0% |
| lex | 7 | 7–8 | ~0.1% |
| parse | 3 | 3–3 | ~0.0% |
| HIR | 878 | 869–880 | ~8% |
| resolve | 6124 | 5949–6626 | ~57% |
| typecheck* | 7902 | 7859–8466 | ~74% (contains `resolve`) |
| MIR | 94 | 92–94 | ~0.9% |
| LIR | 823 | 781–936 | ~7.7% |
| optimize | 318 | 316–344 | ~3% |
| codegen | 451 | 442–463 | ~4.2% |
| write | 132 | 129–137 | ~1.2% |
| **total** | **10714** | **10551–11173** | 100% |

\* Pre-existing measurement bug at `src/driver.nx:241`: the `typecheck` timer
reports `t5 - t4` where `t4` is the *pre-resolve* timestamp, not `t4b`
(post-resolve). So the printed `typecheck` duration INCLUDES the `resolve`
duration. Corrected typecheck-only time ≈ `typecheck_reported − resolve_reported`:
~1,778 ms for driver.nx. This bug is **out of scope for epic ij0e** (purely a
measurement issue); fixing it belongs in a separate small correctness patch.

### Regression rule

Per the epic acceptance criteria, compile time must not degrade by more than
±5% from these medians on this host. Improvements are welcome.

**Hotspot for Phase 4 efficiency audits**: `resolve` dominates the self-host
compile at ~57% of total wall-clock. `hir` (with reachability analysis) and
`lir` (with list-accumulator `rev_*` patterns) are the next-largest phases.

## 4. Source LOC baseline

Command: `wc -l $(fd -e nx . src)`. Current baseline: **18,378 LOC across 33 files**.
(Epic description said 18,392 LOC; the 14-line delta reflects small commits
landed between epic drafting and this baseline — not a measurement error.)

### Per-file breakdown

| File | LOC | Role |
|---|--:|---|
| `src/backend/codegen.nx` | 2,352 | **Monster** — WASM emission + TCMC + string ops |
| `src/backend/wasm_collect.nx` | 476 | |
| `src/backend/wasm_defs.nx` | 479 | WASM opcode/section constants |
| `src/backend/wasm_merge.nx` | 1,363 | **Monster** — stdlib merge, WASM binary decode (ULEB/SLEB) |
| `src/backend/wasm_section.nx` | 451 | |
| `src/common/ast.nx` | 244 | (god-module: 20 imports, by design) |
| `src/common/error.nx` | 171 | |
| `src/common/naming.nx` | 10 | |
| `src/common/token.nx` | 155 | |
| `src/driver.nx` | 309 | Pipeline entry + inline ULEB reader (L38–69) |
| `src/frontend/lexer.nx` | 615 | |
| `src/frontend/parse_core.nx` | 221 | |
| `src/frontend/parse_pattern.nx` | 205 | |
| `src/frontend/parse_type.nx` | 254 | |
| `src/frontend/parser.nx` | 1,025 | **Monster** — expression atoms (L80–~300) |
| `src/ir/demand.nx` | 177 | |
| `src/ir/hir.nx` | 1,560 | **Monster** — AST→HIR (L17–~868) + reachability (L1121+) |
| `src/ir/hir_types.nx` | 133 | |
| `src/ir/imports.nx` | 211 | |
| `src/ir/lir.nx` | 3,162 | **Monster** — ANF lowering + Maranget decision tree (L605–~1800) |
| `src/ir/lir_opt.nx` | 794 | |
| `src/ir/lir_types.nx` | 69 | |
| `src/ir/mir.nx` | 821 | **Monster** — lowering + port-call resolution (L192–~615) |
| `src/ir/mir_types.nx` | 75 | |
| `src/ir/rdrname.nx` | 198 | |
| `src/ir/resolve.nx` | 242 | |
| `src/ir/symtab.nx` | 224 | |
| `src/typecheck/check.nx` | 433 | |
| `src/typecheck/infer.nx` | 1,111 | **Monster** — type inference |
| `src/typecheck/subst.nx` | 313 | |
| `src/typecheck/types.nx` | 187 | |
| `src/typecheck/unify.nx` | 338 | |
| **total** | **18,378** | |

### Monster-file extract targets (Phase 1)

Six files over 1,000 LOC — six candidates for the Phase 1 extracts listed
in the epic:

| File | LOC | Child issue | Target sub-extraction |
|---|--:|---|---|
| `src/ir/lir.nx` | 3,162 | `ij0e.2` | Maranget decision tree → `src/ir/lir/match_tree.nx` |
| `src/backend/codegen.nx` | 2,352 | `ij0e.4`, `ij0e.5` | string ops + TCMC → `src/backend/codegen/{string,tcmc}.nx` |
| `src/ir/hir.nx` | 1,560 | `ij0e.3` | reachability → `src/ir/hir/reachability.nx` |
| `src/backend/wasm_merge.nx` | 1,363 | `ij0e.9` | binary readers → `src/backend/wasm_read.nx` |
| `src/typecheck/infer.nx` | 1,111 | (not in Phase 1) | — |
| `src/frontend/parser.nx` | 1,025 | `ij0e.7` | parse_atom → `src/frontend/parse_atom.nx` |
| `src/ir/mir.nx` | 821 | `ij0e.6` | port-call → `src/ir/mir/port.nx` |

Acceptance-criterion ≥ −30% LOC applies to at least four of the five named
monsters (`lir`, `codegen`, `hir`, `wasm_merge`, `infer`, `parser`). Epic
criterion: post-extract totals per file must drop by ≥ 30% vs the LOC above.

### Copy-paste baseline (Phase 2 DRY targets)

`rg 'Monomorphic reverse'` — **8 occurrences** across:

1. `src/frontend/lexer.nx`
2. `src/ir/hir.nx`
3. `src/ir/lir.nx`
4. `src/ir/lir_opt.nx`
5. `src/ir/mir.nx`
6. `src/backend/codegen.nx`
7. `src/typecheck/check.nx`
8. `src/typecheck/infer.nx`

Target (`ij0e.8`): ≤ 1 occurrence (central module) **or** zero (fix nxc's
`list.reverse` codegen bug and drop the workaround entirely).

WASM binary decode (ULEB/SLEB/section readers) — **5 files**:

1. `src/driver.nx` (inline `rd_uleb_simple`, `uleb_size`, `scan_sections` at L38–69)
2. `src/backend/wasm_merge.nx` (`rd_uleb`, `rd_sleb`, `scan_sleb_end` at L50–84+)
3. `src/backend/wasm_section.nx`
4. `src/backend/wasm_defs.nx`
5. `src/backend/codegen.nx`

Target (`ij0e.9`): 1 shared module (`src/backend/wasm_read.nx`).

## 5. How to compare against this baseline

- **Fixed point**: after any commit, run `./bootstrap.sh --ci`. Exit 0 + the
  `Fixed point reached!` line is mandatory.
- **SHA comparison**: if the commit is pure refactor (no semantic output change),
  `shasum -a 256 nexus.wasm` should still equal the baseline SHA. If it differs,
  investigate — either the refactor is not behaviour-preserving, or stdlib /
  codegen changed in a way that intentionally shifts output bytes.
- **Fixture count**: `cargo test --release --manifest-path bootstrap/Cargo.toml`
  must report `656 passed; 0 failed; 1 ignored`.
- **Compile time**: re-run the 3-target × 3-run measurement harness and
  compare medians. `src/driver.nx` total median must stay within ±5%
  (≈ ±535 ms) of 10,714 ms on this host.
- **LOC**: `wc -l $(fd -e nx . src)` — total should trend down as DRY lands.

### Reproduction commands

```bash
# Fixed point
./bootstrap.sh --ci
shasum -a 256 nexus.wasm

# Fixture pass
cargo test --release --manifest-path bootstrap/Cargo.toml -- --test-threads=4

# Phase timing (repeat 3× per target and take median)
WASMTIME_FLAGS='-W tail-call=y,exceptions=y,max-memory-size=8589934592 --dir=. --dir=/tmp'
wasmtime run $WASMTIME_FLAGS nexus.wasm examples/fib.nx        --verbose /tmp/out.wasm
wasmtime run $WASMTIME_FLAGS nexus.wasm examples/web_server.nx --verbose /tmp/out.wasm
wasmtime run $WASMTIME_FLAGS nexus.wasm src/driver.nx          --verbose /tmp/out.wasm

# LOC
wc -l $(fd -e nx . src)
```

## 6. Post-refactor snapshot (epic ij0e close, 2026-04-23)

After the epic finished, the same three targets were re-measured on the same
host (median of 3). `src/driver.nx` went from 10,714 ms → **10,909 ms
(+1.8%)** — well within the ±5% regression tolerance. Per-phase:

| phase | baseline (ms) | post (ms) | Δ |
|---|--:|--:|--:|
| HIR | 878 | 830 | **−5.4%** |
| resolve | 6,124 | 6,584 | **+7.5%** (more import modules) |
| typecheck\* | 7,902 | 8,306 | +5.1% (inherits resolve) |
| MIR | 94 | 106 | +12.8% (~12 ms, likely noise) |
| LIR | 823 | 795 | **−3.4%** |
| optimize | 318 | 314 | −1.3% |
| codegen | 451 | 419 | **−7.1%** |
| write | 132 | 127 | −3.8% |
| **total** | **10,714** | **10,909** | **+1.8%** |

Phases targeted by Phase 4 audits (ij0e.11 codegen, ij0e.12 HIR/LIR) all
improved naturally from Phase 1-3 decomposition — no hotspot cleared the
10% rewrite-justification bar, so both audits closed YAGNI.

Per-file LOC (monster files):

| file | baseline | post | Δ |
|---|--:|--:|--:|
| `src/ir/lir.nx` | 3,162 | 2,307 | **−27.0%** (ij0e.2) |
| `src/ir/mir.nx` | 821 | 601 | **−26.8%** (ij0e.6) |
| `src/ir/hir.nx` | 1,560 | 1,277 | **−18.1%** (ij0e.3) |
| `src/backend/wasm_merge.nx` | 1,363 | 1,308 | −4.0% (ij0e.9) |
| `src/backend/codegen.nx` | 2,352 | 2,369 | +0.7% (blocked — nexus-12w6) |
| `src/frontend/parser.nx` | 1,025 | 1,036 | +1.1% (blocked — nexus-12w6) |
| `src/typecheck/infer.nx` | 1,111 | 1,121 | +0.9% (not in Phase 1 scope) |
| **src/ total** | **18,378** | **18,712** | **+1.8%** |

New subdirectory-facade internals (did not exist at baseline):

| new file | LOC |
|---|--:|
| `src/ir/lir/match_tree.nx` | 958 |
| `src/ir/hir/reachability.nx` | 323 |
| `src/ir/mir/port.nx` | 241 |
| `src/backend/wasm_read.nx` | 123 |
| `src/common/mono_reverse.nx` | 22 |

The +1.8% total-LOC delta is explained by duplicated helpers in the new
submodules (`rev_*`, `lir_str_less_than`, FNV hash in match_tree.nx; `pack/pval/ppos`
in wasm_read.nx) plus added module-boundary docstrings across every file.
The original acceptance criterion wanted **−5%** total; it wasn't met because
three of six monster files (`codegen.nx`, `parser.nx`, `infer.nx`) couldn't be
decomposed due to structural mutual recursion between a subconcern and the
general expression-lowering of its pipeline stage. See follow-up issue
`nexus-12w6` for the planned shared-helper modules (`codegen/atom.nx`,
`lir/context.nx`) that would unblock the remaining extractions.

Final fixed-point SHA (2026-04-23, post-epic):

| artifact | value |
|---|---|
| stage1 = stage2 byte size | 811,155 |
| SHA-256 | *(updated on each bootstrap — run `shasum -a 256 nexus.wasm` to capture)* |

Fixture pass rate stayed at **656/0/1** throughout every commit.
