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

## ABI

This section documents how Nexus values are represented in WebAssembly. Understanding the ABI is necessary for writing FFI bindings and debugging compiled output.

### Type Mapping

Every Nexus type maps to a WASM value type:

| Nexus Type | WASM Type | Representation |
|---|---|---|
| `i32` | `i32` | Direct |
| `i64` | `i64` | Direct |
| `f32` | `f32` | Direct |
| `f64` / `float` | `f64` | Direct |
| `bool` | `i32` | 0 = false, 1 = true |
| `char` | `i32` | Unicode scalar value |
| `unit` | (none) | No runtime value |
| `string` | `i64` | Packed pointer + length |
| `[T]` (list) | `i64` | Heap pointer |
| `[| T |]` (array) | `i64` | Heap pointer |
| `{ ... }` (record) | `i64` | Heap pointer |
| ADT variant | `i64` | Heap pointer |
| closure / `fn(...)` | `i64` | Heap pointer |

All heap-allocated values (records, ADTs, lists, arrays, closures) are represented as `i64` pointers into linear memory. Primitives smaller than 64 bits (`bool`, `char`, `i32`, `f32`) use their native WASM types.

### String Encoding

Strings are packed into a single `i64`:

```
bits 63-32: offset (u32, pointer into linear memory)
bits 31-0:  length (u32, byte count)
```

Pack: `(offset << 32) | length`
Unpack: `offset = value >>> 32`, `length = value & 0xFFFFFFFF`

String literal bytes are written to the WASM data section starting at offset 16. Literals are deduplicated by value. The heap base is aligned to 8 bytes after all string data.

### Memory Layout

```
Offset 0-15:     Reserved (WASM header)
Offset 16-N:     String literals (data section)
Offset align8(N): Heap base — objects grow upward
```

**Allocation strategy:**

- **With stdlib.wasm**: Calls `allocate(bytes: i32) -> i32` from the stdlib module. This uses dlmalloc internally.
- **Without stdlib**: Bump allocator using WASM global 0 as the heap pointer. Grows memory on demand via `memory.grow`.

All heap objects are stored in 8-byte words.

### Heap Object Layout

#### Constructor (ADT variant)

```
Word 0:  i64  tag
Word 1:  i64  field[0]
Word 2:  i64  field[1]
...
```

The tag is computed via FNV-1a: `hash(name) ^ arity * FNV_PRIME`. Pattern matching compares tags with `i64.eq`.

**Field ordering**: Fields are stored in **lexicographic order by field name**. When a constructor is created with labeled arguments (e.g., `Cons(v: x, rest: xs)`), the arguments are sorted before storage. Field extraction via pattern matching uses the same sorted index. List literals `[a, b, c]` desugar to `Cons` with positional arguments in this sorted order.

#### Record

Same layout as constructors:

```
Word 0:  i64  tag
Word 1:  i64  field[0]  (sorted by field name)
Word 2:  i64  field[1]
...
```

The tag is `fnv1a("rec" + sorted_field_names) ^ field_count * FNV_PRIME`. Fields are stored in **lexicographic order** by field name.

#### Closure

```
Word 0:  i64  funcref table index
Word 1:  i64  captured value[0]
Word 2:  i64  captured value[1]
...
```

Closures are called via `call_indirect`. The closure pointer is passed as the first argument (`__env: i64`), and the callee loads captured values from `__env` at the appropriate offsets.

### Value Packing

When storing values in heap objects, all values are normalized to `i64`:

| Source Type | Pack to i64 | Unpack from i64 |
|---|---|---|
| `i64` | (no-op) | (no-op) |
| `i32` | `i64.extend_i32_s` | `i32.wrap_i64` |
| `f64` | `i64.reinterpret_f64` | `f64.reinterpret_i64` |
| `f32` | `i32.reinterpret_f32` then `i64.extend_i32_u` | `i32.wrap_i64` then `f32.reinterpret_i32` |
| `unit` | `i64.const 0` | (error) |

### Calling Convention

- **Internal functions**: All labeled parameters and arguments are sorted **lexicographically by label**. Both the function signature and call sites use the same sorted order.
- **External functions (FFI)**: Parameters stay in **source (definition) order** to match the stdlib WASM ABI. Call arguments are matched to external parameters **by label**, not by position.
- `unit` parameters generate no WASM parameter.
- `unit`-returning functions have an empty WASM result type.
- Tail calls use WASM `return_call` when not inside a `try` block.

### Indirect Calls (Closures)

Closure calls use `call_indirect` with an extended signature. The first parameter is always `__env: i64` (the closure heap pointer):

```
// Nexus: let f = fn (x: i64) -> i64 do ... end
// WASM type: (param i64 i64) (result i64)
//                   ^env ^x
```

The caller pushes `__env`, then the normal arguments, then the table index as `i32` for `call_indirect`.

### Exception Model

Exceptions use two WASM globals (not WASM exception handling):

```wasm
(global $exn_flag  (mut i32) (i32.const 0))   ;; 0 = no exception
(global $exn_value (mut i64) (i64.const 0))   ;; exception object pointer
```

**Raise**: stores the exception value, sets the flag to 1, returns a dummy value.
**Catch**: after each statement, checks `$exn_flag`. If set, jumps to the catch handler, clears the flag, and binds `$exn_value` to the catch parameter.

### FFI Boundary

External functions use a different parameter encoding for strings and arrays. The packed `i64` representation is split into separate pointer and length parameters:

| Nexus Type | WASM Params (FFI) | Notes |
|---|---|---|
| `i32`, `bool`, `char` | 1x `i32` | Direct |
| `i64` | 1x `i64` | Direct |
| `f32` | 1x `f32` | Direct |
| `f64` | 1x `f64` | Direct |
| `string` | 2x `i32` (ptr, len) | Unpacked from packed i64 at boundary |
| `[| T |]` (array) | 2x `i32` (ptr, len) | Same unpacking as string |
| `%ByteBuffer`, opaque | 1x `i64` | Handle passed directly |
| `unit` | (none) | No parameter generated |

Parameter order: external function parameters preserve **source (definition) order**, not lexicographic order. This is because the WASM function type signature must match the stdlib export exactly. Call-site arguments are reordered to match by looking up each external parameter's label.

Return values use the same types as internal functions (strings return as packed `i64`).

### Module Structure

#### Exports

| Name | Kind | Condition |
|---|---|---|
| `main` | function | Always (entry point) |
| `wasi:cli/run@0.2.6#run` | function | Always (WASI run command) |
| `memory` | memory | If memory is defined or imported |
| `__conc_<name>` | function | If program uses `conc` blocks |

#### Imports

| Module | Name | Condition |
|---|---|---|
| `<module>.wasm` | `<wasm_name>` | Each `external` binding |
| stdlib module | `allocate` | If objects + stdlib present |
| `nexus:runtime/conc` | `__nx_conc_spawn`, `__nx_conc_join` | If `conc` blocks present |
| `nexus:runtime/backtrace` | `__nx_bt_push`, `__nx_bt_pop`, `__nx_bt_freeze` | If `raise`/`try` present |

#### Custom Sections

| Section Name | Format | Purpose |
|---|---|---|
| `nexus:capabilities` | UTF-8 newline-separated names | Declared runtime permissions |

#### Funcref Table

If the program uses closures or function references, a funcref table is emitted:

- Element type: `funcref`
- Size: number of unique function references
- Active initialization at offset 0
- Used by `call_indirect` for closure dispatch

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
