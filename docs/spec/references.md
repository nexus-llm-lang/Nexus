# References & Mutability

Nexus emphasizes explicit resource management and side effects.
Memory is safe, and mutable references are explicit (`~` sigil).

## Immutable by Default

Variables are immutable by default.

```nexus
let x = 10
// x = 11 // Error: Immutable
```

## Mutable References (`~`)

Mutable variables are marked with `~` at declaration and use.

```nexus
let ~counter = 0
~counter <- ~counter + 1
```

- `let ~x` creates a mutable reference.
- `~x` dereferences the value.
- `<-` assigns a new value to the reference.

## Borrowing Rules

- A function returning a reference (`Ref<T>`) is generally disallowed to prevent escaping stack references (Gravity Rules).
- References cannot be captured by concurrent tasks (`conc`).
- Immutable variables cannot hold mutable references.

## Linear Types (`%`) (Planned)

Linear types (`%`) will enforce exact-once usage for resources (e.g., file handles, network sockets).
Parsing is supported, but linear semantics are not yet enforced.
