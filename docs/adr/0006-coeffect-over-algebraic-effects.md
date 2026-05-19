# ADR 0006 — Coeffect over Algebraic Effects: Design Philosophy

**Status**: Accepted  
**Date**: 2026-05-01 (formalized; design predates this date)  

## Context

Nexus is a language with an explicit effect system.  When designing the effect
system, there were two main architectural choices: **algebraic effects with
effect handlers** (as in Koka, Effekt, or OCaml 5) versus **coeffects /
capability-based effects** (as in the capability-passing literature and
object-capability systems).

### Algebraic effects (effect handlers)

In an algebraic effect system, every side-effecting operation is modeled as an
*effect* that a handler can intercept, resume, or abort.  Effects propagate
upward through the call stack until a handler is installed.  The type of a
function includes its *effect set* in the return row.  Handlers can perform
arbitrary resumptions — including multi-shot (resuming the continuation more
than once).

This provides maximum generality: IO, exceptions, state, async, and
non-determinism can all be expressed as effects with handlers.

### Coeffects / capability passing

In a coeffect-based system, side effects are mediated through *capabilities*:
objects passed as arguments (or implicitly threaded) that represent the right
to perform an operation.  A function that needs console output receives a
`Console` capability; a function that needs filesystem access receives `Fs`.
The capability *is* the permission — holding it is sufficient, and not holding
it means the operation is statically forbidden.

The type of a function includes its *require row* — the set of capabilities it
needs from the call site.

### Why the distinction matters

In Nexus's concrete setting:

- **Handlers** are present for structured effect injection (`inject ... with
  cap SomeCap do ... end`), but they are *capability handlers*, not
  algebraic-effect handlers.  They bind a new capability into the dynamic
  scope of a block.  There is no general multi-shot resume.
- **Exceptions** (`throw` / `catch`) are handled via WASM stack-switching
  (`cont.new` / `resume`) and typed separately via the `throws` row, not via
  the capability row.
- **The `require` row** on every function arrow type lists the capabilities
  required — the coeffect.  This is checked at call sites; a caller without
  `Fs` cannot call a function that requires `Fs` without explicitly injecting
  it.

### Options considered

| # | Design | Summary |
|---|--------|---------|
| 1 | Full algebraic effects | Every operation is an effect; handlers are multi-shot resumable |
| 2 | Capabilities only (no handlers) | Object-capability passing; no dynamic interception |
| 3 | Coeffect require-row + capability-scoped handlers (landed) | Require-row tracks needed caps; inject scopes a cap into a block |
| 4 | Monadic effects (Haskell style) | Explicit monad threading; incompatible with nexus's imperative style |

## Decision

**Option 3: coeffect require-row + capability-scoped handlers.**

Nexus tracks the capability *requirement* at the type level (the `require`
row on function arrows) rather than modeling every operation as a resumable
effect.  This is a coeffect discipline.

The `inject cap X with Y do ... end` construct provides scoped capability
binding: inside the block, operations requiring `X` are satisfied by `Y`.
This is a capability handler — it does not intercept operations mid-flight
or resume continuations; it only scopes the binding.

Exceptions remain separate: `throws { TypeError, ... }` is a distinct row in
the function arrow, tracked independently from capability requirements.
`try ... catch ... end` handles thrown exceptions; `inject` handles capability
requirements.  The two rows are structurally parallel but semantically distinct.

Multi-shot algebraic effect handlers (e.g., for non-determinism or
backtracking) are explicitly **out of scope** for the current design.  The
`cont.new` / `resume` primitives support single-shot continuations for
the async (`@`) use case; they are not exposed as a general effect-handler
mechanism to user programs.

### Rationale

- Capability-based effects are easier to reason about for systems programmers:
  "does this function need `Fs`?" is a question with a direct answer at the
  call site, not a question about which handlers are installed in the dynamic
  scope.
- Coeffect tracking composes cleanly with linear types (`%T`): a linear
  capability can be passed exactly once, enforcing single-use access patterns.
- Full algebraic effects require a runtime mechanism for intercepting and
  resuming operations (a trampoline or continuation-passing transform).  The
  WASM stack-switching proposal covers single-shot async; general multi-shot
  resumption adds runtime complexity disproportionate to use cases in nexus's
  target domains.
- The `require` row is already present in function arrow types; adding
  capability tracking required no new type-level mechanism.

## Consequences

- **Positive**: capability requirements are visible at every call site in
  the type signature.  There is no hidden effect propagation; the `require`
  row is explicit.
- **Positive**: composing two functions with different capability requirements
  is straightforward: the combined require-row is the union.
- **Positive**: injecting a mock capability in tests is a first-class
  operation (`inject cap Console with mock_console do ... end`); no mocking
  framework needed.
- **Negative**: programs that need dynamic interception (e.g., capturing all
  `Fs` calls in a test to check they happened) must thread a capability
  wrapper manually; there is no general algebraic-effect-style interceptor.
- **Negative**: non-determinism, backtracking, and coroutine-style effects
  cannot be expressed as user-defined effects.  They require language-level
  additions.
- **Neutral**: exceptions (`throws`) and capabilities (`require`) are parallel
  rows but governed by different rules.  Developers must keep the two
  mechanisms distinct in their mental model.
