# Runtime & Entrypoint

The Nexus language requires a specific entrypoint for execution.

## Entrypoint: `main`

Every executable program must define a `main` function.

### Signature

The `main` function must have the following signature:

```nexus
fn main() -> unit
```

- **Parameters**: It must take no arguments.
- **Return Type**: It must return `unit`.
- **Visibility**: It must be private (no `pub` modifier).
- **Effects**: It must not declare the `Exn` effect.

### Execution

When the program starts, the runtime executes the `main` function.
Any side effects (such as printing to stdout via `print` or logging) must be performed within `main` or functions called by it.

### Exit Code

- If `main` executes successfully, the process exits with code 0.
- If an unhandled error or panic occurs, the process exits with a non-zero code.
