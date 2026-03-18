use crate::harness::exec_with_stdlib;

#[test]
fn option_some_is_some() {
    exec_with_stdlib(
        r#"
import { Option, is_some } from stdlib/option.nx

let main = fn () -> unit do
  let opt = Some(val: 42)
  let result = is_some(opt: opt)
  if result != true then raise RuntimeError(val: "expected is_some true") end
  return ()
end
"#,
    );
}

#[test]
fn option_unwrap_or_some() {
    exec_with_stdlib(
        r#"
import { Option, unwrap_or } from stdlib/option.nx

let main = fn () -> unit do
  let opt = Some(val: 10)
  let result = unwrap_or(opt: opt, default: 0)
  if result != 10 then raise RuntimeError(val: "expected 10") end
  return ()
end
"#,
    );
}

#[test]
fn option_or_else_prefers_some() {
    exec_with_stdlib(
        r#"
import { Option, or_else, unwrap_or } from stdlib/option.nx

let main = fn () -> unit do
  let a: Option<i64> = None
  let b = Some(val: 42)
  let result = unwrap_or(opt: or_else(opt: a, other: b), default: 0)
  if result != 42 then raise RuntimeError(val: "expected 42") end
  return ()
end
"#,
    );
}

#[test]
fn option_none_is_none() {
    exec_with_stdlib(
        r#"
import { Option, is_none } from stdlib/option.nx

let main = fn () -> unit do
  let opt: Option<i64> = None
  let result = is_none(opt: opt)
  if result != true then raise RuntimeError(val: "expected is_none true") end
  return ()
end
"#,
    );
}

#[test]
fn option_unwrap_or_none() {
    exec_with_stdlib(
        r#"
import { Option, unwrap_or } from stdlib/option.nx

let main = fn () -> unit do
  let opt: Option<i64> = None
  let result = unwrap_or(opt: opt, default: 99)
  if result != 99 then raise RuntimeError(val: "expected 99") end
  return ()
end
"#,
    );
}

#[test]
fn tuple_fst_returns_left() {
    exec_with_stdlib(
        r#"
import { Pair, fst } from stdlib/tuple.nx

let main = fn () -> unit do
  let p = Pair(left: 10, right: 20)
  let result = fst(p: p)
  if result != 10 then raise RuntimeError(val: "expected 10") end
  return ()
end
"#,
    );
}
