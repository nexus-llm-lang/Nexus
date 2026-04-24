---
layout: default
title: Tools
---

# Tools

## Language Server (LSP)

Nexus ships a built-in Language Server Protocol server for editor integration and programmatic analysis.

### Usage

```bash
nexus lsp          # start LSP server (stdio)
```

Configure your editor to run `nexus lsp` as the language server for `.nx` files.

### Capabilities

| Feature | Description |
|---|---|
| Diagnostics | Parse errors, type errors, and warnings published on open/change |
| Hover | Type information for variables, functions, type definitions, enums |
| Go to Definition | Jump to the definition site of a symbol (same file) |
| Document Symbols | Outline of functions, types, enums, ports, exceptions |
| References | Find all occurrences of an identifier |
| Rename | Rename an identifier across the file |
| Completion | Keywords, symbols from the type environment, module members |

### CLI Diagnostics (LLM-friendly)

For non-interactive use (CI pipelines, LLM tool calls, scripts), `nexus check --format json` outputs structured diagnostics to stdout:

```bash
nexus check --format json program.nx
```

The output contains `file`, `ok` (bool), `diagnostics` (with range, severity, message), and `symbols` (with name, kind, range). Exit code is `0` on success, `1` on errors.

### Editor Setup Examples

**Neovim (nvim-lspconfig)**

```lua
vim.api.nvim_create_autocmd('FileType', {
  pattern = 'nexus',
  callback = function()
    vim.lsp.start({
      name = 'nexus',
      cmd = { 'nexus', 'lsp' },
      root_dir = vim.fs.root(0, '.git'),
    })
  end,
})
```

**VS Code (settings.json)**

Use a generic LSP client extension (e.g., [vscode-languageclient](https://github.com/AstroNvim/astrolsp)) and configure `nexus lsp` as the server command for `.nx` files.

**Helix (languages.toml)**

```toml
[[language]]
name = "nexus"
scope = "source.nexus"
file-types = ["nx"]
language-servers = ["nexus-lsp"]

[language-server.nexus-lsp]
command = "nexus"
args = ["lsp"]
```

---

## AI Coding Agent Skill

Nexus is designed to be written by LLMs. To help coding agents produce correct Nexus code, this repository provides a [Claude Code skill](https://docs.anthropic.com/en/docs/claude-code/skills) containing the full language reference.

### Installation

```bash
npx skills add nexus-llm-lang/Nexus --skill nexus-lang
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
- **Capability system** -- `cap` → `handler` → `inject` → `require` flow
- **Runtime permissions** -- `PermConsole`, `PermFs`, `PermNet`, etc.
- **Standard library** -- correct import forms and function signatures

### Other agents

The skill is a set of Markdown files. Agents that don't support Claude Code skills can read the files directly from `skills/nexus-lang/`.

LLM agents can also use `nexus check --format json` as a tool to get structured diagnostics without installing the skill.
