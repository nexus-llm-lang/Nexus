# Basics

Nexus is an LLM-native programming language designed for robust, verifiable, and efficient software development.

## File Extension

Nexus source files use the `.nx` extension.

## Execution

Nexus programs are executed using the `nexus` CLI.

```bash
# Run a file
nexus <file.nx>

# Start REPL mode
nexus
```

## Structure

A Nexus program consists of top-level definitions:
- Functions (`fn`)
- Type definitions (`type`)
- Imports (`import`)
- Ports (`port`)

## Entrypoint

Every executable program must define a `main` function.
See [Runtime](runtime.md) for details.
