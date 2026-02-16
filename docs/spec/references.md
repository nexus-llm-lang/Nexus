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

## Linear Types (`%`)

Linear types (`%`) enforce exact-once usage semantics, ideal for resource management.

```nexus
fn consume(r: %Resource) -> unit do
    // ... use r ...
endfn

let %res = create_resource()
perform consume(r: %res)
// perform consume(r: %res) // Error: Already consumed
```

### Properties

- **Exactly Once**: A linear variable must be used exactly once.
- **No Discard**: The wildcard pattern `_` cannot be used to discard a linear value. This ensures that every resource is explicitly consumed or handled.
- **No Ref**: Mutable references to linear types (`Ref<Linear<T>>`) are strictly forbidden to prevent aliasing violations.
