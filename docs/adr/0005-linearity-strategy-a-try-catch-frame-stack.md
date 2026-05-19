# ADR 0005 — Linearity Strategy A for try/catch: frame_stack Snapshot

**Status**: Accepted  
**Date**: 2026-04-28  
**Issues**: nexus-7eex.1, nexus-7eex.2, nexus-uf8v  

## Context

Nexus has linear types: a value annotated `%T` must be consumed exactly once.
The typecheck linearity pass (`src/ir/linearity.nx`) tracks which linear
bindings are "live" at each program point and rejects programs that consume
a binding twice or let it escape a scope unconsumed.

The `try ... catch ... end` statement introduces a control-flow split: the
try-body may exit either normally (fall-through) or exceptionally (jump to the
catch arm).  This creates a linearity invariant that is subtle and was
initially wrong:

### The incorrect naive approach

If the catch arm is typechecked using the linear environment at the *end* of
the try-body, it sees the post-try state — all linear bindings consumed during
the try body are marked as used.  This is **wrong**: when an exception is
thrown mid-try, some bindings that appear consumed in the fall-through path
may not have been consumed at all (the store may not have been reached).

### The correct invariant

The catch arm must be typechecked starting from the linear environment at the
**entry** of the try-body — the *pre-try snapshot*.  This is "Strategy A":
save the linear_vars state before entering the try-body, and restore it as the
starting environment for the catch arm.

An additional constraint applies to the try-body: a linear binding live at
try-entry must not cross a throwable statement call.  If it does, an exception
at that call leaves the linear binding unconsumed with no handler to clean it
up (nexus-7eex.2, commit `f2ae5dde`).

### Alternative strategies considered

| # | Strategy | Description |
|---|----------|-------------|
| A | Pre-try snapshot | Catch arm starts from the pre-try linear env (landed) |
| B | Intersection | Catch arm uses the intersection of all possible throw-point envs |
| C | Pessimistic: treat all try-body linears as possibly live | Accept that catch arm must consume all pre-try linears unconditionally |

Strategy B requires tracking throw-point environments throughout the try body,
which significantly complicates the linearity walk.  Strategy C is overly
restrictive (forces the catch arm to re-consume bindings even if the try body
consumed them on all paths before the throw).  Strategy A is the simplest
correct rule.

## Decision

**Strategy A: catch arm starts from the pre-try linear_vars snapshot.**

Implementation (nexus-7eex.1, commit `316cca05`):

- Before entering the try-body, snapshot `linear_vars` as `pre_try_linear_vars`.
- Typecheck the try-body normally, consuming bindings.
- Typecheck the catch arm starting from `pre_try_linear_vars` (not the
  post-try state).
- After both arms, join the live sets: a binding is live post-try/catch only
  if it is live on both the fall-through and catch paths.

The additional throwable-call constraint (nexus-7eex.2): during the try-body,
if a linear binding is live when a statement call is encountered and that call
is in a throwable position, emit E2007 (linear value may escape via exception).

Error code discipline: an unconsumed linear in the try-body emits E2007 (not
E2009, which is for closure-capture leaks); this was corrected in nexus-uf8v
(commit `fadb879e`).

## Consequences

- **Positive**: the invariant is correct and decidable without dataflow beyond
  what the linearity pass already computes.
- **Positive**: the pre-try snapshot is a single extra copy of the
  `linear_vars` map; overhead is proportional to the number of live linear
  bindings, which is typically small.
- **Positive**: Strategy A's rules are easy to state and verify against the
  formal typing rules (T-Try-Catch requires the catch arm to typecheck under
  Γ_pre, not Γ_post).
- **Negative**: programs that consume a linear inside a try-body and then
  reference it in the catch arm get a false-positive linearity error.  This is
  actually correct behavior — the catch arm genuinely cannot rely on the
  consumption having happened.
- **Negative**: linear bindings live when a throwable call occurs inside a
  try-body are conservatively rejected, even if the specific exception handler
  would never be reached for that call.  Programs may need to consume linears
  before the throwable call.
- **Neutral**: the `try ... catch` form is a statement, not an expression
  (separate parser constraint).  Functions that need to return a value from a
  try/catch must factor the body into a helper function.
