use crate::common::source::run;
use nexus::interpreter::Value;

#[test]
fn tuple_fst_returns_left() {
    let src = r#"
import { Pair, fst } from stdlib/tuple.nx

let main = fn () -> i64 do
  let p = Pair(left: 10, right: 20)
  return fst(p: p)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn tuple_snd_returns_right() {
    let src = r#"
import { Pair, snd } from stdlib/tuple.nx

let main = fn () -> i64 do
  let p = Pair(left: 10, right: 20)
  return snd(p: p)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(20));
}
