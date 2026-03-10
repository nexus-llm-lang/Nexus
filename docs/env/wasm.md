---
layout: default
title: WASM and WASI
---

# WASM and WASI

Nexus compiles to the WebAssembly Component Model with WASI for system interfaces. The coeffect system maps directly to WASI capabilities -- every `require { PermX }` in your program corresponds to a concrete WASI interface grant.

## Permission-to-Capability Mapping

| Nexus Permission | WASI Capability | CLI Flag | Enforcement |
|---|---|---|---|
| `PermConsole` | `stdin`, `stdout`, `stderr` | `--allow-console` | Enforced |
| `PermFs` | `wasi:filesystem/preopens` | `--allow-fs` | Enforced |
| `PermNet` | `wasi:http/outgoing-handler`, `wasi:sockets/*` | `--allow-net` | Enforced |
| `PermRandom` | `wasi:random/random` | `--allow-random` | Statically checked |
| `PermClock` | `wasi:clocks/wall-clock`, `monotonic-clock` | `--allow-clock` | Statically checked |
| `PermProc` | `wasi:cli/exit`, `environment` | `--allow-proc` | Statically checked |
| `PermEnv` | `wasi:cli/environment` | `--allow-env` | Statically checked |

## Capability Enforcement

### Static Verification

The type checker ensures:
1. Any function calling a capability-requiring port must itself `require` that capability or have it satisfied via `inject`
2. `main`'s `require` clause is the source of truth for the entire program's capability surface

### Binary Encoding

Required permissions are stored in a custom WASM section named `nexus:capabilities`:

```
Section name: "nexus:capabilities"
Data format:  UTF-8 newline-separated capability names
Example:      "Fs\nNet\nConsole\n"
```

This allows tools to inspect required permissions without executing the binary.

### Runtime Enforcement

The Nexus runtime (via wasmtime) configures the WASI context based on declared capabilities:

- **Filesystem isolation**: if `PermFs` is not required, no directories are preopened
- **Network isolation**: if `PermNet` is not required, network interfaces are not inherited
- **Console isolation**: if `PermConsole` is not required, stdio is not inherited

## Nexus Host Bridge

For HTTP networking, Nexus includes a host bridge component (`nexus_host_bridge`) that translates Nexus FFI calls (prefixed with `__nx_http`) into WASI HTTP component calls.

When a program requires `PermNet`, the compiler automatically composes this bridge into the final WASM component. This ensures the `Net` port works on any WASI-compliant host.

## Building and Running

### Compile to WASM

```bash
nexus build program.nx                  # outputs main.wasm
nexus build program.nx -o output.wasm   # custom output path
```

The build step requires `wasm-merge` for dependency bundling. Configure via `--wasm-merge PATH` or the `NEXUS_WASM_MERGE` environment variable.

### Run with wasmtime

```bash
# Minimal (no capabilities)
wasmtime run -Scli main.wasm

# With network
wasmtime run -Scli -Shttp -Sinherit-network -Sallow-ip-name-lookup -Stcp main.wasm

# With filesystem preopens
wasmtime run -Scli --dir ./data main.wasm
```

### Inspect Capabilities

```bash
nexus build program.nx --explain-capabilities              # list capability names (default)
nexus build program.nx --explain-capabilities=wasmtime     # show wasmtime run command with flags
nexus build program.nx --explain-capabilities=none         # suppress output
nexus build program.nx --explain-capabilities-format=json  # machine-readable JSON
```
