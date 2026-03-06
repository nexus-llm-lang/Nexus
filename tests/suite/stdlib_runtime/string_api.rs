use crate::common::source::run;
use nexus::interpreter::Value;

#[test]
fn string_repeat_basic() {
    let src = r#"
import { repeat } from stdlib/string.nx

let main = fn () -> string do
  return repeat(s: "ab", n: 3)
end
"#;
    assert_eq!(run(src).unwrap(), Value::String("ababab".to_string()));
}

#[test]
fn string_pad_left_basic() {
    let src = r#"
import { pad_left } from stdlib/string.nx

let main = fn () -> string do
  return pad_left(s: "42", width: 5, fill: "0")
end
"#;
    assert_eq!(run(src).unwrap(), Value::String("00042".to_string()));
}

#[test]
fn string_pad_right_basic() {
    let src = r#"
import { pad_right } from stdlib/string.nx

let main = fn () -> string do
  return pad_right(s: "hi", width: 5, fill: ".")
end
"#;
    assert_eq!(run(src).unwrap(), Value::String("hi...".to_string()));
}

#[test]
fn string_concat_basic() {
    let src = r#"
import { concat } from stdlib/string.nx

let main = fn () -> string do
  return concat(a: "hello", b: " world")
end
"#;
    assert_eq!(run(src).unwrap(), Value::String("hello world".to_string()));
}

#[test]
fn string_join_basic() {
    let src = r#"
import { join } from stdlib/string.nx

let main = fn () -> string do
  let xs = Cons(v: "a", rest: Cons(v: "b", rest: Cons(v: "c", rest: Nil())))
  return join(xs: xs, sep: ", ")
end
"#;
    assert_eq!(run(src).unwrap(), Value::String("a, b, c".to_string()));
}

#[test]
fn string_parse_i64_valid() {
    let src = r#"
import { parse_i64 } from stdlib/string.nx
import { Option, unwrap_or } from stdlib/option.nx

let main = fn () -> i64 do
  return unwrap_or(opt: parse_i64(s: "42"), default: 0)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(42));
}

#[test]
fn string_parse_i64_invalid() {
    let src = r#"
import { parse_i64 } from stdlib/string.nx
import { Option, is_none } from stdlib/option.nx

let main = fn () -> bool do
  return is_none(opt: parse_i64(s: "not_a_number"))
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
