use crate::harness::exec_with_stdlib;

#[test]
fn string_repeat_basic() {
    exec_with_stdlib(
        r#"
import { repeat, length } from "std:str"

let main = fn () -> unit do
  let result = repeat(s: "ab", n: 3)
  if length(s: result) != 6 then raise RuntimeError(val: "expected length 6") end
  return ()
end
"#,
    );
}

#[test]
fn string_pad_left_basic() {
    exec_with_stdlib(
        r#"
import { pad_left, length } from "std:str"

let main = fn () -> unit do
  let result = pad_left(s: "42", width: 5, fill: "0")
  if length(s: result) != 5 then raise RuntimeError(val: "expected length 5") end
  return ()
end
"#,
    );
}

#[test]
fn string_pad_right_basic() {
    exec_with_stdlib(
        r#"
import { pad_right, length } from "std:str"

let main = fn () -> unit do
  let result = pad_right(s: "hi", width: 5, fill: ".")
  if length(s: result) != 5 then raise RuntimeError(val: "expected length 5") end
  return ()
end
"#,
    );
}

#[test]
fn string_concat_basic() {
    exec_with_stdlib(
        r#"
import { concat, length } from "std:str"

let main = fn () -> unit do
  let result = concat(a: "hello", b: " world")
  if length(s: result) != 11 then raise RuntimeError(val: "expected length 11") end
  return ()
end
"#,
    );
}

#[test]
fn string_parse_i64_valid() {
    exec_with_stdlib(
        r#"
import { parse_i64 } from "std:str"
import { Option, unwrap_or } from "std:option"

let main = fn () -> unit do
  let result = unwrap_or(opt: parse_i64(s: "42"), default: 0)
  if result != 42 then raise RuntimeError(val: "expected 42") end
  return ()
end
"#,
    );
}

#[test]
fn string_parse_i64_invalid() {
    exec_with_stdlib(
        r#"
import { parse_i64 } from "std:str"
import { Option, is_none } from "std:option"

let main = fn () -> unit do
  let result = is_none(opt: parse_i64(s: "not_a_number"))
  if result != true then raise RuntimeError(val: "expected is_none to be true") end
  return ()
end
"#,
    );
}

#[test]
fn test_to_string_runtime_error() {
    exec_with_stdlib(
        r#"
import { to_string } from "std:exn"

let main = fn () -> unit do
  let e: Exn = RuntimeError(val: "boom")
  let result = to_string(exn: e)
  if result != "RuntimeError: boom" then raise RuntimeError(val: "unexpected to_string result") end
  return ()
end
"#,
    );
}

#[test]
fn test_to_string_invalid_index() {
    exec_with_stdlib(
        r#"
import { to_string } from "std:exn"

let main = fn () -> unit do
  let e: Exn = InvalidIndex(val: 42)
  let result = to_string(exn: e)
  if result != "InvalidIndex: 42" then raise RuntimeError(val: "unexpected to_string result") end
  return ()
end
"#,
    );
}

#[test]
fn test_backtrace_captures_call_stack() {
    exec_with_stdlib(
        r#"
import { backtrace } from "std:exn"
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
  try
    raise RuntimeError(val: "boom")
  catch e ->
    let bt = backtrace(exn: e)
    match bt do
      | Cons(v: first, rest: _) ->
        if first != "main" then
          Console.println(val: "expected frame 'main', got '" ++ first ++ "'")
          raise RuntimeError(val: "wrong frame")
        end
      | Nil ->
        raise RuntimeError(val: "expected non-empty backtrace")
    end
  end
  end
  return ()
end
"#,
    );
}

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_string_length_concat(s1 in "[a-zA-Z0-9]{0,20}", s2 in "[a-zA-Z0-9]{0,20}") {
        let src = format!("
import {{ length }} from \"stdlib/str.nx\"
let main = fn () -> unit do
    let s1 = [=[{}]=]
    let s2 = [=[{}]=]
    let concat = s1 ++ s2
    if length(s: concat) != (length(s: s1) + length(s: s2)) then raise RuntimeError(val: \"length mismatch\") end
    return ()
end
", s1, s2);
        exec_with_stdlib(&src);
    }
}

// Console-related tests (from stdlib.rs)

#[test]
fn console_read_line_requires_perm_console() {
    let err = crate::harness::should_fail_typecheck(
        r#"
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit do
  inject stdio.system_handler do
    let _ = Console.read_line()
    return ()
  end
end
"#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn console_read_line_typechecks_with_perm_console() {
    crate::harness::should_typecheck(
        r#"
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    let _ = Console.read_line()
    return ()
  end
end
"#,
    );
}

#[test]
fn console_getchar_with_mock_handler() {
    exec_with_stdlib(
        r#"
import { Console } from "std:stdio"

let mock_console = handler Console do
  fn print(val: string) -> unit do
    return ()
  end
  fn println(val: string) -> unit do
    return ()
  end
  fn read_line() -> string do
    return ""
  end
  fn getchar() -> string do
    return "A"
  end
end

let main = fn () -> unit do
  inject mock_console do
    let result = Console.getchar()
    if result != "A" then raise RuntimeError(val: "expected A") end
    return ()
  end
end
"#,
    );
}

#[test]
fn join_tail_recursive_deep() {
    exec_with_stdlib(
        r#"
import { join, length } from "std:str"

let make_strs = fn (n: i64, acc: [ string ]) -> [ string ] do
  if n == 0 then return acc end
  return make_strs(n: n - 1, acc: "x" :: acc)
end

let main = fn () -> unit do
  // 5k strings joined with ',' must not overflow. Smaller N than split (50k)
  // because join is O(N²) on string length — 50k would be too slow.
  let xs = make_strs(n: 5000, acc: [])
  let result = join(xs: xs, sep: ",")
  // expected byte length: 5000 * 1 + 4999 separators = 9999.
  if length(s: result) != 9999 then raise RuntimeError(val: "join produced wrong length") end
  return ()
end
"#,
    );
}

#[test]
fn split_tail_recursive_deep() {
    exec_with_stdlib(
        r#"
import { split, repeat, length } from "std:str"
import * as list from "std:list"

let main = fn () -> unit do
  // 50k repeats of "a," → split on "," → 50001 segments. Must not overflow.
  let s = repeat(s: "a,", n: 50000)
  let segs = split(s: s, sep: ",")
  let n = list.length(xs: segs)
  if n != 50001 then raise RuntimeError(val: "expected 50001 segments from split") end
  return ()
end
"#,
    );
}

#[test]
fn console_read_line_with_mock_handler() {
    exec_with_stdlib(
        r#"
import { Console } from "std:stdio"

let mock_console = handler Console do
  fn print(val: string) -> unit do
    return ()
  end
  fn println(val: string) -> unit do
    return ()
  end
  fn read_line() -> string do
    return "mock input"
  end
  fn getchar() -> string do
    return ""
  end
end

let main = fn () -> unit do
  inject mock_console do
    let result = Console.read_line()
    if result != "mock input" then raise RuntimeError(val: "expected mock input") end
    return ()
  end
end
"#,
    );
}
