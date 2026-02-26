# Nexus CLI

The `nexus` command-line interface is the primary tool for running and interacting with Nexus programs.

## Usage

```bash
nexus run [FILE|-]
nexus build [FILE|-] [-o OUTPUT] [--wasm] [--wasm-merge PATH]
nexus check [FILE|-]
```

### `run`

Runs Nexus source (`.nx`) via interpreter (`main` is executed after parse + typecheck).

```bash
nexus run example.nx
```

If no file is provided and stdin is piped, `run` reads from stdin:

```bash
cat example.nx | nexus run
nexus run < example.nx
```

`nexus run` does not execute wasm modules.
Build component wasm with `nexus build --wasm`, then run with `wasmtime`.

#### Shebang scripts

`run` supports scripts with shebang (the first `#!...` line is ignored by parser).

```nexus
#!/usr/bin/env -S nexus run
fn main() -> i64 do
  return 42
endfn
```

### `build`

Builds after parse + typecheck.
By default, `build` emits:
- a packed executable embedding component wasm (default `./main.out`)

```bash
nexus build program.nx
# writes ./main.out
```

Emit component-model wasm explicitly:

```bash
nexus build program.nx --wasm
# writes ./main.wasm
```
This path performs component embedding/encoding/composition in-process (Rust crates).

When `--wasm` needs file-backed wasm import bundling, `wasm-merge` is invoked externally.
You can override the command path explicitly:

```bash
nexus build program.nx --wasm --wasm-merge /opt/bin/wasm-merge
```

Or via env var:

```bash
NEXUS_WASM_MERGE=/opt/bin/wasm-merge nexus build program.nx --wasm
```

Resolution order is: `--wasm-merge` > `NEXUS_WASM_MERGE` > `wasm-merge` from `PATH`.

Run the emitted component directly with `wasmtime`:

```bash
wasmtime run -Scli -Shttp -Sinherit-network -Sallow-ip-name-lookup -Stcp main.wasm
```

Specify an explicit output path:

```bash
nexus build program.nx -o out/program
# writes out/program
nexus build program.nx --wasm -o out/program.wasm
```

Current codegen coverage is intentionally limited to the typed-ANF monomorphic subset.
external calls are emitted as wasm imports (including `print` from `nxlib/stdlib/stdio.wasm`).
`nexus build` (packed executable) bundles file-based wasm imports into the embedded pack format
and resolves them via the runtime linker.
`nexus build --wasm` also bundles file-based imports before component encoding using
external `wasm-merge`.
Preview2/component import modules (for example `wasi:http/...`) are preserved and not merged.
Network IO should use component builds (`--wasm`) and execute via `wasmtime run`
with WASI HTTP enabled.
`cargo build` automatically rebuilds `nxlib/stdlib/{stdio,stdlib}.wasm` via `build.rs`.
If needed, wasm rebuild can be skipped with `NEXUS_SKIP_WASM_BUILD=1`.
For example, `examples/fib.nx` can be built and run as wasm.

If no file is provided and stdin is piped:
- `nexus build` writes `main.out` by default.
- `nexus build --wasm` writes `main.wasm` by default.

### `check`

Runs static checks only (parse + typecheck), without interpretation or wasm generation.

```bash
nexus check program.nx
cat program.nx | nexus check
```

### REPL Mode

If no subcommand is provided and stdin is a TTY, the CLI starts REPL mode.

```bash
nexus
```

- **Commands**: Type `:help` for available REPL commands.
- **Exit**: Type `:exit` or `:quit`, or press `Ctrl-D`.
- **History**: Use the Up and Down arrow keys to navigate through previous inputs.
- **Evaluation**: Each line is parsed as a statement or expression, typechecked, and executed immediately. The result and its type are displayed. If multiple lines are needed (e.g., inside an `if` block), the prompt changes to `..`.
- **Top-level**: You can also define functions, types, and constants at the top level of the REPL; they will persist across evaluations.

If you are developing Nexus itself, you can use `cargo` to run the CLI:

```bash
# Run a source file (interpreter)
cargo run -- run example.nx

# Run from stdin
cat example.nx | cargo run -- run

# Start REPL
cargo run

# Emit wasm
cargo run -- build example.nx --wasm

# Emit packed executable
cargo run -- build example.nx -o out/example

# Check only
cargo run -- check example.nx
```

*Note: If you are using Nix, wrap these commands in `nix develop --command ...`.*
