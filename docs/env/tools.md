---
layout: default
title: Tools
---

# AI Coding Agent Tools

Nexus is designed to be written by LLMs. To help coding agents produce correct Nexus code, this repository provides a [Claude Code skill](https://docs.anthropic.com/en/docs/claude-code/skills) containing the full language reference.

## Claude Code Skill

### Installation

```bash
npx skills add Nymphium/Nexus --skill nexus-lang
```

The skill activates automatically when Claude Code works with `.nx` files.

### Contents

| File | Description |
|---|---|
| `SKILL.md` | Quick reference: syntax rules, effect system, type summary, anti-patterns |
| `references/syntax.md` | Complete EBNF grammar and operator precedence |
| `references/types.md` | Type system: primitives, records, ADTs, linear types, borrowing, mutability |
| `references/effects.md` | Ports, handlers, inject, runtime permissions, checked exceptions |
| `references/stdlib.md` | Full standard library API (all modules and function signatures) |
| `references/patterns.md` | Idiomatic code patterns: list recursion, error handling, concurrency, web servers |
| `templates/*.nx` | Starter templates for hello-world, web server, CLI app, port/handler DI |

### What the skill teaches agents

- **Labeled arguments** -- all call sites use `f(param: value)`, never positional
- **Block delimiters** -- `do ... end`, `then ... else ... end`, not braces
- **Linear types** (`%`) -- resources consumed exactly once, compiler-enforced
- **Borrowing** (`&`) -- immutable views without consumption
- **Coeffect system** -- `port` → `handler` → `inject` → `require` flow
- **Runtime permissions** -- `PermConsole`, `PermFs`, `PermNet`, etc.
- **Standard library** -- correct import forms and function signatures

### Other agents

The skill is a set of Markdown files. Agents that don't support Claude Code skills can read the files directly from `skills/nexus-lang/`.
