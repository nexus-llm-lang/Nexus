---
layout: default
title: Nexus
---

<p class="hero-tagline">
<strong>Nexus</strong> is a programming language built on one premise: <strong>LLMs are strong at literal program constructs but weak at contextual ones.</strong>
Garbage collection, implicit conversions, ambient I/O, continuation-based control flow — these contextual mechanisms are where LLM-generated code breaks and where human review fails. Nexus replaces them with syntactically explicit alternatives.
</p>

<div class="code-showcase code-showcase-top" markdown="1">
<div class="code-card" markdown="1">

### Coeffects -- Hello world

Capability requirements declared in function signatures. Dependency injection via ports and handlers.

```nexus
import { Console }, * as stdio from "stdlib/stdio.nx"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "Hello, Nexus!")
  end
end
```

</div>
</div>

<div class="code-showcase" markdown="1">
<div class="code-card" markdown="1">

### Linear Types

Resources consumed exactly once. No GC — the compiler tracks every allocation.

```nexus
let %h = Fs.open_read(path: "data.txt")
let %r = Fs.read(handle: %h)
match %r do
  case { content: text, handle: %h2 } ->
        Fs.close(handle: %h2)
end
```

</div>
<div class="code-card" markdown="1">

### Conc Blocks

Parallel tasks with captured variables. Compiles to WASM with WASI capabilities.

```nexus
let %arr = [| 0, 0 |]
conc do
  task t1 do
    let r = &%arr; r[0] <- compute_a()
  end
  task t2 do
    let r = &%arr; r[1] <- compute_b()
  end
end
```

</div>
</div>

## Quick Start

```bash
nexus                   # REPL
nexus run example.nx    # interpret
nexus build example.nx  # compile to main.wasm
nexus check example.nx  # typecheck only
```

---

<div class="doc-nav">
  <div class="doc-section">
    <h4>Design</h4>
    <ul>
      <li><a href="design">Design Thesis</a> — Why every construct is literal</li>
    </ul>
    <h4>Language Specification</h4>
    <ul>
      <li><a href="spec/syntax">Syntax</a> — Grammar and EBNF</li>
      <li><a href="spec/types">Types</a> — Type system, linear types, borrowing</li>
      <li><a href="spec/effects">Effects and Coeffects</a> — Ports, handlers, inject</li>
      <li><a href="spec/exception-groups">Exception Groups</a> — Structured exceptions, multi-arm catch</li>
      <li><a href="spec/lazy">Lazy Evaluation</a> — Deferred computation with <code>@</code></li>
      <li><a href="spec/concurrency">Concurrency</a> — Parallel tasks with <code>conc</code> blocks</li>
      <li><a href="spec/semantics">Semantics</a> — Evaluation model</li>
    </ul>
  </div>
  <div class="doc-section">
    <h4>Environment</h4>
    <ul>
      <li><a href="env/cli">CLI</a> — Command-line interface</li>
      <li><a href="env/wasm">WASM and WASI</a> — Capability mapping and ABI</li>
      <li><a href="env/ffi">FFI</a> — Wasm interop</li>
      <li><a href="env/stdlib">Standard Library</a> — Builtin modules</li>
      <li><a href="env/tools">Tools</a> — AI coding agent skill</li>
    </ul>
  </div>
</div>
