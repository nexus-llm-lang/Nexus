---
layout: default
title: Exception Groups
---

# Exception Groups

Exception groups define a named set of exception types. A `catch` arm matching a group name expands to match all members.

## Declaration

```nexus
exception NotFound(path: string)
exception PermDenied(path: string, code: i64)

exception group IOError = NotFound | PermDenied
```

Groups are declared with `exception group Name = Member1 | Member2 | ...`. Each member must be a previously declared exception. The `export` modifier makes the group visible to importers.

## Structured Exceptions

Individual exceptions carry typed fields -- not pre-formatted strings. This enables both fine-grained and coarse-grained error handling:

```nexus
exception UnexpectedToken(expected: string, got: string, span: Span)
exception MissingMain

exception group ParseError = UnexpectedToken | MissingMain
```

Zero-field exceptions (like `MissingMain`) omit parentheses in both declaration and pattern matching.

## Catching Groups

Catching a group name expands to match every member:

```nexus
try
  parse(tokens: tokens)
catch
  case ParseError ->
    // catches both UnexpectedToken and MissingMain
    Console.eprintln(val: "parse failed")
end
```

This is syntactic sugar -- the compiler expands `case ParseError ->` into one case per member.

## Catching Specific Exceptions

For precise handling, match individual exception types and destructure their fields:

```nexus
try
  compile(src: src)
catch
  case UnexpectedToken(expected: e, got: g, span: sp) ->
    Console.eprintln(val: "expected " ++ e ++ ", got " ++ g)
  case TypeMismatch(func_name: f, detail: d, span: sp) ->
    Console.eprintln(val: "in " ++ f ++ ": " ++ d)
  case MissingMain ->
    Console.eprintln(val: "no main function")
end
```

Specific arms and group arms can be mixed. The compiler checks each arm independently.

## Multi-Arm Catch

The `catch` clause supports two forms:

```nexus
// Legacy: single variable binding
try body catch e -> handle(e: e) end

// Multi-arm: pattern matching on exception constructors
try
  body
catch
  case NotFound(msg: m) -> handle_missing(m: m)
  case PermDenied(msg: m) -> handle_perm(m: m)
  case IOError -> handle_generic()
end
```

Multi-arm catch desugars to `catch __exn -> match __exn do ... end` during compilation.

## Group Composition

Groups reference individual exceptions, not other groups. To create a "super-group" spanning multiple phases, list all members directly:

```nexus
exception group CompileError =
  UnexpectedToken | MissingMain
  | TypeMismatch
  | SymbolNotFound
```

Groups are expanded at catch sites -- there is no hierarchical nesting at runtime.

---

See also: [Checked Exceptions and Capabilities](../effects), [Syntax](../syntax)
