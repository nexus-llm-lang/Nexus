# Nexus

<center>
  <img src="https://raw.githubusercontent.com/nexus-llm-lang/nexus-llm-lang.github.io/main/assets/img/logo/default.png" alt="Nexus Logo" width="80%"/>
</center>

Nexus is a language built around one bet. **LLMs are good at code you can read on the page, and bad at code that depends on what's off the page.**

Implicit control flow, hidden state, and ambient authority are where LLMs go wrong. Human review misses these spots too. Nexus puts each resource, each side effect, and each cap need in the syntax at the use site.

## Design thesis

Mainstream languages lean on *off-page* mechanisms. GC, implicit casts, ambient I/O, exceptions propagating through unmarked call stacks. These are the patterns where LLM-written code goes wrong and where human review fails to catch it.

Nexus flips this around:

| Off the page (cut) | On the page (Nexus form) |
|---|---|
| Implicit resource cleanup (GC) | `%` linear types — used once |
| Hidden aliasing | `&` borrow — a read-only view |
| Ambient I/O | `require { Net }` — stated up front |
| Implicit control transfer (continuations) | `try/catch` — old-school unwind |
| Positional arguments | `add(a: 1, b: 2)` — labels required |

Each resource state, each side effect, and each dep on the environment is visible in the source text. Context implies nothing.

## Why coeffects, and why not effects

Most of the effect-system literature centers on *algebraic effects*. A function performs an effect, and a handler picks it up through a delimited continuation. The handler decides whether to resume, restart, or abort the effectful work.

Nexus drops that model. A continuation is the quintessential off-page construct. Control flow turns on which handler happens to be installed at runtime, and the handler's resume-or-not behaviour is invisible at the call site.

Instead, Nexus uses **coeffects**. The `require { ... }` clause declares what caps a function *needs from its environment*, rather than what it *does to* the environment:

```nexus
cap Logger do
  fn info(msg: string) -> unit
end

let greet = fn (name: string) -> unit require { Logger, Console } do
  Logger.info(msg: "Greeting " ++ name)
  Console.println(val: "Hello, " ++ name ++ "!")
  return ()
end
```

`Logger.info(...)` is a **direct call** to a statically resolved handler method. It does not suspend into a continuation. The handler is a plain value that implements an interface:

```nexus
let console_logger = handler Logger require { Console } do
  fn info(msg: string) -> unit do
    Console.println(val: "[INFO] " ++ msg)
    return ()
  end
end
```

`inject` supplies handler values to a scope, discharging the corresponding `require`:

```nexus
let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    inject console_logger do
      greet(name: "Nexus User")
    end
  end
  return ()
end
```

Read this as dep injection rather than algebraic effect handling. There are no continuations, no implicit control transfer, and no hidden resume points. The only builtin effect is `Exn` (the exception type), handled via the usual `try/catch` with unwind semantics.

## Linear types and borrowing

Resources that have to be released — file handles, server sockets, db connections — are tracked as **linear types** (`%`). The compiler enforces once-and-only-once use:

```nexus
let %h = Fs.open_read(path: path)
let %r = Fs.read(handle: %h)      -- %h consumed here
match %r do
  case { content: c, handle: %h2 } ->  -- %h2 extracted
    Fs.close(handle: %h2)      -- %h2 consumed
end
```

When you need to read without consuming, **borrow** the value with `&`:

```nexus
let server = Net.listen(addr: addr)
let req = Net.accept(server: &server)  -- borrow: server not consumed
let method = request_method(req: &req)   -- borrow: req not consumed
Net.respond(req: req, ...)         -- req consumed
Net.stop(server: server)         -- server consumed
```

No garbage collector. No implicit drop. The resource lifecycle is on the page.

## Cap-based security

Runtime perms map straight to WASI caps:

```nexus
let main = fn () -> unit require { PermNet, PermConsole } do
  inject net_mod.system_handler, stdio.system_handler do
    try
      let body = Net.get(url: "https://example.com")
      Console.println(val: body)
    catch e ->
      Console.println(val: "Request failed")
    end
  end
  return ()
end
```

The `require { PermNet, PermConsole }` clause is checked at build time and enforced again at the WASI runtime level. A function may do network I/O only when it declares `PermNet` and a handler is injected.

## Building

The compiler ships as a self-extracting POSIX sh + wasm polyglot. Build it locally:

```bash
./bootstrap.sh        # produces ./nexus (polyglot launcher) and ./nexus.wasm
cp nexus ~/.local/bin/   # or anywhere on PATH
```

To bootstrap and install in one step, use `--install [PREFIX]` (defaults to `/usr/local`):

```bash
./bootstrap.sh --install             # installs to /usr/local/bin/nexus
./bootstrap.sh --install ~/.local    # installs to ~/.local/bin/nexus
```

[wasmtime](https://wasmtime.dev/) must be installed and on `PATH`. Override the wasm runtime via env vars:

```bash
NEXUS_MAX_WASM_STACK=134217728 nexus build foo.nx -o out.wasm   # 128 MiB stack
NEXUS_WASMTIME_ARGS="-S http,inherit-network" nexus build foo.nx -o out.wasm
```

The committed `nexus.wasm` at the repo root is the Stage0 seed of the self-host. Any change under `src/**` or `nxlib/**` must regenerate it via `./bootstrap.sh`, and the new `nexus.wasm` must land in the same commit. CI enforces `nexus.wasm == stage1.wasm`.

## Usage

```bash
nexus repl                          # interactive REPL
nexus build example.nx              # compile to out.wasm
nexus build example.nx -o main.wasm
nexus typecheck example.nx          # typecheck only
nexus test [path]                   # run the test runner
nexus lsp                           # start Language Server (stdio)
nexus help                          # full subcommand listing
```

Compiled programs run under wasmtime with the caps they declare:

```bash
wasmtime run -Scli main.wasm
wasmtime run -Scli -Shttp -Sinherit-network -Sallow-ip-name-lookup -Stcp main.wasm
```

## Example

```nexus
import { Console }, * as stdio from "std:stdio"
import { from_i64 } from "std:str"

let fib = fn (n: i64) -> i64 do
  if n <= 1 then return n end
  return fib(n: n - 1) + fib(n: n - 2)
end

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    let v = fib(n: 30)
    Console.println(val: "fib(30) = " ++ from_i64(val: v))
  end
  return ()
end
```

## Examples

The [examples](./examples) folder ships minimal, runnable demos of every language surface. The demos cover linear types and borrowing, mutable references, generics, pattern matching, exceptions, caps and handlers, lazy evaluation, and FFI.

Each file builds and runs on its own through the `nexus build` and `nexus run` commands. The README inside that folder gives an indexed listing.

## AI coding agent support

Nexus ships a [Claude Code skill](https://docs.anthropic.com/en/docs/claude-code/skills) that teaches coding agents the language syntax, type system, effect system, and stdlib.

```bash
npx skills add nexus-llm-lang/Nexus --skill nexus-lang
```

Once installed, Claude Code activates the skill as soon as it writes or reviews a `.nx` file.

## Documentation

The rendered site lives at **<https://nexus-llm-lang.github.io/>**. Its source lives in the [`nexus-llm-lang/nexus-llm-lang.github.io`](https://github.com/nexus-llm-lang/nexus-llm-lang.github.io) repo.

| Document | Description |
|---|---|
| [Design](https://nexus-llm-lang.github.io/latest/design/) | Design thesis: literal vs contextual |
| [Syntax](https://nexus-llm-lang.github.io/latest/spec/syntax/) | Grammar and EBNF |
| [Types](https://nexus-llm-lang.github.io/latest/spec/types/) | Type system, linear types, borrowing |
| [Effects](https://nexus-llm-lang.github.io/latest/spec/effects/) | Coeffect system, ports, handlers |
| [Semantics](https://nexus-llm-lang.github.io/latest/spec/semantics/) | Evaluation model, entrypoint |
| [CLI](https://nexus-llm-lang.github.io/latest/env/cli/) | Command-line interface |
| [WASM](https://nexus-llm-lang.github.io/latest/env/wasm/) | WASM compilation and WASI capabilities |
| [FFI](https://nexus-llm-lang.github.io/latest/env/ffi/) | Wasm interop |
| [Stdlib](https://nexus-llm-lang.github.io/latest/env/stdlib/) | Standard library |
| [Tools](https://nexus-llm-lang.github.io/latest/env/tools/) | LSP server, CLI diagnostics, AI coding agent skill |
| [Glossary](docs/glossary.md) | Codebase acronyms: HIR/MIR/LIR, TCMC, TCWF, RdrName, and more |

## License

[MIT](LICENSE)

