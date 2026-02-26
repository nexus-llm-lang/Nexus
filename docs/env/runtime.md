# Runtime & Entrypoint

The Nexus language requires a specific entrypoint for execution.

## Entrypoint: `main`

Every executable program must define a `main` function.

### Signature

The `main` function must have the following signature:

```nexus
let main = fn () -> unit require { ... } effect { ... } do
  ...
endfn
```

- **Parameters**: It must take no arguments.
- **Return Type**: It must return `unit`.
- **Visibility**: It must be private (`pub let main` is rejected).
- **Effects**: `main` may declare only `Console` in `effect` (or be empty).
- **Coeffects**: `main` may declare only `Net` and/or `Fs` in `require` (or be empty).
- **Exn**: `main` cannot expose `Exn` in its effect row.

### Execution

When the program starts, the runtime executes the `main` function.
Any side effects (such as printing to stdout via `print` or logging) must be performed within `main` or functions called by it.

### Exit Code

- If `main` executes successfully, the process exits with code 0.
- If an unhandled error or panic occurs, the process exits with a non-zero code.

## WASM-Oriented Runtime Notes

### Typed ANF (Compiler IR)

Nexus now has an initial typed ANF lowering stage for the future WASM backend.

- Implemented in `src/compiler/anf.rs` and `src/compiler/lower.rs`.
- Lowering starts from `main` and only includes reachable functions.
- Current scope is intentionally conservative: monomorphic reachable code path first.
- Reachable generic functions are rejected in this MVP path.
- Initial codegen from typed ANF to wasm binary is implemented in `src/compiler/codegen.rs`.

### String Representation for WASM

For non-GC WASM integration, strings are modeled as runtime-managed handles.

- Implemented in `src/runtime/string_heap.rs`.
- `StringHandle` is an integer handle suitable for WASM value passing.
- `StringHeap` keeps bytes plus explicit reference counts.
- Runtime operations include allocate/retain/release/concat.

This design avoids immediate dependence on WASM GC while keeping ownership/lifetime explicit.
