---
layout: landing
title: Nexus Language
---

<p class="hero-tagline">
Nexus is a programming language built on one premise: <strong>LLMs are strong at literal program constructs but weak at contextual ones.</strong>
Garbage collection, implicit conversions, ambient I/O, continuation-based control flow — these contextual mechanisms are where LLM-generated code breaks and where human review fails. Nexus replaces them with syntactically explicit alternatives.
</p>

<div class="feature-grid">
<div class="feature-card">
<h3>Coeffects</h3>
<p>Capability requirements declared in function signatures. Dependency injection via ports and handlers — no continuations, no implicit control flow.</p>
</div>
<div class="feature-card">
<h3>Linear Types</h3>
<p>Resources consumed exactly once. No garbage collector, no finalizers. The compiler tracks every allocation.</p>
</div>
<div class="feature-card">
<h3>Explicit Syntax</h3>
<p>Keyword-terminated blocks, mandatory borrow annotations, labeled arguments. Every call site means exactly what it says.</p>
</div>
<div class="feature-card">
<h3>WebAssembly</h3>
<p>Compiles to WASM component model. Permissions map to WASI capabilities, enforced at both compile time and runtime.</p>
</div>
</div>

## Quick Start

```bash
nexus                    # REPL
nexus run example.nx     # interpret
nexus build example.nx   # compile to main.wasm
nexus check example.nx   # typecheck only
```

## Hello World

```nexus
import { Console }, * as stdio from stdlib/stdio.nx

let main = fn () -> unit require { PermConsole } do
    inject stdio.system_handler do
        Console.println(val: "Hello, Nexus!")
    end
    return ()
end
```

Everything is explicit: the function requires `PermConsole`, a handler is injected to satisfy it, and `main` returns `unit`.

## All Versions

<ul id="version-list" class="version-list">
<li><a href="latest/">latest</a></li>
</ul>
