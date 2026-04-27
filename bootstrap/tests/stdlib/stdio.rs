use crate::harness::{exec_with_stdlib, should_fail_typecheck, should_typecheck};

/// Negative: read_bytes is a Console capability, so calling it without
/// `require { PermConsole }` must fail typecheck (parallels read_line).
#[test]
fn console_read_bytes_requires_perm_console() {
    let err = should_fail_typecheck(
        r#"
import { Console }, * as stdio from "std:stdio"
import * as bb from "std:bytebuffer"

let main = fn () -> unit do
  inject stdio.system_handler do
    let %buf = Console.read_bytes(n: 4)
    bb.free(%buf)
    return ()
  end
end
"#,
    );
    assert!(
        err.contains("PermConsole") || err.contains("permission") || err.contains("require"),
        "expected permission-related error, got: {err}"
    );
}

/// Positive: read_bytes typechecks under `require { PermConsole }`, and
/// the linear ByteBuffer must be discharged (here via `bb.free`).
#[test]
fn console_read_bytes_typechecks_with_perm_console() {
    should_typecheck(
        r#"
import { Console }, * as stdio from "std:stdio"
import * as bb from "std:bytebuffer"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    let %buf = Console.read_bytes(n: 16)
    bb.free(%buf)
    return ()
  end
end
"#,
    );
}

/// Negative: read_bytes returns `%ByteBuffer` — dropping it without
/// `bb.free` must trip linearity. Guards the linear-return contract on
/// the cap surface (the FFI hands out a fresh pool entry per call).
#[test]
fn console_read_bytes_drop_is_rejected() {
    let err = should_fail_typecheck(
        r#"
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    let %leaked_buf = Console.read_bytes(n: 4)
    return ()
  end
end
"#,
    );
    assert!(
        err.contains("Unused linear") && err.contains("leaked_buf"),
        "expected linearity error naming `leaked_buf`, got: {err}"
    );
}

/// Runtime: a mock Console handler returns an `empty()` buffer for
/// `read_bytes`, and main asserts length == 0 then frees it. Exercises
/// the cap-dispatch path end-to-end without relying on real stdin.
/// (Real-FFI byte-for-byte preservation is covered by the Rust unit
/// tests in `nexus_collection_wasm::read_bytes_tests`.)
#[test]
fn console_read_bytes_with_mock_handler() {
    exec_with_stdlib(
        r#"
import { Console } from "std:stdio"
import * as bb from "std:bytebuffer"

let mock_console = handler Console do
  fn print(val: string) -> unit do return () end
  fn println(val: string) -> unit do return () end
  fn eprint(val: string) -> unit do return () end
  fn eprintln(val: string) -> unit do return () end
  fn read_line() -> string do return "" end
  fn getchar() -> string do return "" end
  fn read_bytes(n: i64) -> %ByteBuffer do
    return bb.empty()
  end
end

let main = fn () -> unit do
  inject mock_console do
    let %buf = Console.read_bytes(n: 8)
    let len = bb.length(buf: &buf)
    bb.free(%buf)
    if len != 0 then raise RuntimeError(val: "expected empty buf from mock") end
    return ()
  end
end
"#,
    );
}
