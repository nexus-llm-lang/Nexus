use crate::common::source::run;
use nexus::interpreter::Value;

#[test]
fn option_some_is_some() {
    let src = r#"
import { Option, is_some } from stdlib/option.nx

let main = fn () -> bool do
  let opt = Some(val: 42)
  return is_some(opt: opt)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn option_unwrap_or_some() {
    let src = r#"
import { Option, unwrap_or } from stdlib/option.nx

let main = fn () -> i64 do
  let opt = Some(val: 10)
  return unwrap_or(opt: opt, default: 0)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn option_map_some() {
    let src = r#"
import { Option, map, unwrap_or } from stdlib/option.nx

let double = fn (val: i64) -> i64 do return val * 2 end

let main = fn () -> i64 do
  let opt = Some(val: 5)
  let mapped = map(opt: opt, f: double)
  return unwrap_or(opt: mapped, default: 0)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn option_and_then_chains() {
    let src = r#"
import { Option, and_then, unwrap_or } from stdlib/option.nx

let safe_div = fn (val: i64) -> Option<i64> do
  if val == 0 then return None()
  else return Some(val: 100 / val) end
end

let main = fn () -> i64 do
  let a = and_then(opt: Some(val: 5), f: safe_div)
  let b = and_then(opt: Some(val: 0), f: safe_div)
  return unwrap_or(opt: a, default: 0) + unwrap_or(opt: b, default: -1)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(19));
}

#[test]
fn option_or_else_prefers_some() {
    let src = r#"
import { Option, or_else, unwrap_or } from stdlib/option.nx

let main = fn () -> i64 do
  let a: Option<i64> = None()
  let b = Some(val: 42)
  return unwrap_or(opt: or_else(opt: a, other: b), default: 0)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn option_unwrap_none_raises() {
    let src = r#"
import { Option, unwrap } from stdlib/option.nx

let main = fn () -> i64 do
  let opt: Option<i64> = None()
  try return unwrap(opt: opt)
  catch e -> return -1
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(-1));
}

#[test]
fn option_expect_none_message() {
    let src = r#"
import { Option, expect } from stdlib/option.nx
import { to_string } from stdlib/exn.nx

let main = fn () -> string do
  let opt: Option<i64> = None()
  try
    let _ = expect(opt: opt, msg: "value required")
    return "unreachable"
  catch e -> return to_string(exn: e)
  end
end
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::String("RuntimeError: value required".to_string())
    );
}

#[test]
fn option_none_is_none() {
    let src = r#"
import { Option, is_none } from stdlib/option.nx

let main = fn () -> bool do
  let opt: Option<i64> = None()
  return is_none(opt: opt)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
