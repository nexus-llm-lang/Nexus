# Nexus

Nexus is an **LLM-friendly** programming language designed for maximum readability, safety, and seamless integration with modern AI-assisted development workflows. It combines a clean, keyword-oriented syntax with a powerful effect system and strict memory management.

## Key Philosophy: LLM-Native

Nexus is designed from the ground up to be "AI-friendly":
- **Explicit Context:** Keyword-terminated blocks (`endfn`, `endmatch`) provide clear boundaries for LLM context windows.
- **Labeled Clarity:** Labeled arguments are mandatory at call sites, reducing ambiguity for both humans and AI.
- **Predictable Structure:** A strict internal A-Normal Form (ANF) ensures the language remains easy to reason about and transform.
- **Human-Reviewable:** The same properties that help LLMs — explicit block endings, mandatory labels, no implicit behavior — make LLM-generated code straightforward for humans to review and verify.

## Key Features

### Core Characteristics
- **Call-by-Value:** Predictable and standard evaluation semantics.
- **Null-Free Environment:** No `null` or `nil`. Optionality is strictly handled via algebraic data types (e.g., `Result`, `Option`).
- **Mandatory Labeled Arguments:** All function call sites must use labels (`add(a: 1, b: 2)`), ensuring clarity and preventing positional parameter bugs.

### Resource & Memory Management (Sigils)
- **Syntactic Visibility:** Mutability (`~`) and Linearity (`%`) are encoded directly into the identifier's name. The state of a resource is visible everywhere it is used.
- **Scope-Bound Mutability:** Mutable bindings (`~`) are strictly restricted to the local stack and cannot escape their defining function.
- **Linear Types:** First-class support for linear resources (`%`) that must be consumed exactly once, providing compile-time guarantees for file handles, sockets, and other critical resources.

### Effects & Concurrency
- **Algebraic Effects:** A robust effect system using `port` and `handler` to decouple logic from side-effect implementation.
- **Structured Concurrency:** Native `conc` and `task` blocks for coordinated and safe parallel execution.
- **Wasm-First Design:** Built to target the WebAssembly component model for portable and secure deployment.

## Usage

Run the REPL:
```bash
nexus
>> print(val: [=[hello]=])
hello
() : unit
```

Run a script:
```bash
nexus run example.nx
```

Build artifacts:
```bash
nexus build example.nx         # writes packed ./main.out
nexus build example.nx --wasm  # writes ./main.wasm (component wasm)
```

Run `--wasm` output with wasmtime:
```bash
# basic component execution
wasmtime run -Scli main.wasm

# when using stdlib/net (HTTP)
wasmtime run -Scli -Shttp -Sinherit-network -Sallow-ip-name-lookup -Stcp main.wasm
```

Build packed executable with explicit output:
```bash
nexus build example.nx -o example
# writes ./example
```

See [docs/cli.md](docs/cli.md) for the full CLI reference.

## Example

```nexus
let fib = fn (n: i64) -> i64 do
  if n == 0 then
    return 0
  endif
  if n == 1 then
    return 1
  endif
  let n1 = n - 1
  let n2 = n - 2
  let a = fib(n: n1)
  let b = fib(n: n2)
  return a + b
endfn

let main = fn () -> unit effect { Console } do
  let res = fib(n: 10)
  let s = i64_to_string(val: res)
  let msg = [=[ Fibonacci(10) = ]=] ++ s
  print(val: msg)
  return ()
endfn
```

## Documentation

| Document | Description |
|---|---|
| [Syntax](docs/spec/syntax.md) | Grammar and EBNF |
| [Types](docs/spec/types.md) | Type system and inference |
| [Semantics](docs/spec/semantics.md) | Evaluation model |
| [Effects](docs/spec/effect_system.md) | Effect/coeffect system |
| [FFI](docs/env/ffi.md) | Wasm interop |
| [Runtime](docs/env/runtime.md) | Entrypoint and execution |
| [Stdlib](docs/env/stdlib.md) | Standard library |
| [CLI](docs/cli.md) | Command-line interface |

## License
[MIT](LICENSE)
