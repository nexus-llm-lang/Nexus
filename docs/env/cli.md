---
layout: default
title: CLI
---

# CLI

## Global Flags

| Flag | Description |
|---|---|
| `--verbose` / `-v` | Enable structured timing output to stderr |

## Commands

### `nexus run [FILE|-]`

Interpret a Nexus source file:

```bash
nexus run program.nx
nexus run --allow-console --allow-fs program.nx
echo 'let main = fn () -> unit do return () end' | nexus run -
```

Supports stdin piping and shebang scripts.

### `nexus build [FILE|-]`

Compile to a WASM component:

```bash
nexus build program.nx                  # outputs main.wasm
nexus build program.nx -o output.wasm   # custom output path
```

Requires `wasm-merge` for dependency bundling. Configure via:
- `--wasm-merge PATH` flag
- `NEXUS_WASM_MERGE` environment variable

Resolution order: `--wasm-merge` > `NEXUS_WASM_MERGE` > `wasm-merge` from `PATH`.

Inspect declared capabilities:

```bash
nexus build program.nx --explain-capabilities           # list capability names (default)
nexus build program.nx --explain-capabilities=none      # suppress capability output
nexus build program.nx --explain-capabilities=wasmtime  # show wasmtime run command
```

Output format:

```bash
nexus build program.nx --explain-capabilities-format=text  # human-readable (default)
nexus build program.nx --explain-capabilities-format=json  # machine-readable JSON
```

Run the built component:

```bash
wasmtime run -Scli main.wasm
wasmtime run -Scli -Shttp -Sinherit-network -Sallow-ip-name-lookup -Stcp main.wasm
```

### `nexus check [FILE|-]`

Parse and typecheck only. No execution, no WASM output.

```bash
nexus check program.nx
```

### REPL

Run `nexus` with no arguments to start an interactive session:

```bash
nexus
```

- Persistent definitions across inputs
- Multi-line input support (prompt changes to `..`)
- `PermConsole` auto-enabled
- `:help` for commands
- `:exit` / `:quit` or Ctrl-D to quit
- `:reset` to clear accumulated state
- `:defs` to list accumulated definitions

## Capability Flags

| Flag | Permission | Description |
|---|---|---|
| `--allow-fs` | `PermFs` | Filesystem access |
| `--allow-net` | `PermNet` | Network access |
| `--allow-console` | `PermConsole` | Standard I/O |
| `--allow-random` | `PermRandom` | Random number generation |
| `--allow-clock` | `PermClock` | Clock and timers |
| `--allow-proc` | `PermProc` | Process control |
| `--allow-env` | `PermEnv` | Environment variables |
| `--preopen DIR` | (with `PermFs`) | Preopen a directory for filesystem access |

Capability flags apply to `nexus run`. The compiled WASM binary encodes required capabilities in the `nexus:capabilities` section -- the host runtime (e.g., wasmtime) enforces them at execution time. See [WASM and WASI](../wasm).

## Development

Build and run from source:

```bash
cargo run -- run program.nx --allow-console
cargo run -- build program.nx
cargo run -- check program.nx
```
