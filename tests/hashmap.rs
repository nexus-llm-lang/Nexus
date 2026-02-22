use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

fn run(src: &str) -> Result<Value, String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)?;
    let mut interpreter = Interpreter::new(p);
    interpreter.run_function("main", vec![])
}

#[test]
fn hashmap_put_get_or_and_contains_key() {
    let src = r#"
import as hashmap from [=[nxlib/stdlib/hashmap.nx]=]

let main = fn () -> i64 do
  let ops = hashmap.i64_key_ops()
  let m0 = hashmap.empty(key_ops: ops)
  let m1 = hashmap.put(map: m0, key: 1, value: 10)
  let m2 = hashmap.put(map: m1, key: 2, value: 20)
  let m3 = hashmap.put(map: m2, key: 1, value: 99)
  let v1 = hashmap.get_or(map: m3, key: 1, default: 0)
  let v2 = hashmap.get_or(map: m3, key: 2, default: 0)
  let v3 = hashmap.get_or(map: m3, key: 3, default: 7)
  let has2 = hashmap.contains_key(map: m3, key: 2)
  if has2 then
    return v1 + v2 + v3
  else
    return -1
  endif
endfn
"#;
    assert_eq!(run(src).unwrap(), Value::Int(126));
}

#[test]
fn hashmap_get_lookup_and_remove() {
    let src = r#"
import as hashmap from [=[nxlib/stdlib/hashmap.nx]=]

let main = fn () -> i64 do
  let ops = hashmap.i64_key_ops()
  let m0 = hashmap.empty(key_ops: ops)
  let m1 = hashmap.put(map: m0, key: 7, value: 70)
  let m2 = hashmap.put(map: m1, key: 8, value: 80)
  let got = hashmap.get(map: m2, key: 7)
  let m3 = hashmap.remove(map: m2, key: 8)
  let sz = hashmap.size(map: m3)
  match got do
    case Found(value: v) -> return v + sz
    case Missing() -> return -1
  endmatch
endfn
"#;
    assert_eq!(run(src).unwrap(), Value::Int(71));
}

#[test]
fn hashmap_custom_key_ops_can_change_key_equivalence() {
    let src = r#"
import as hashmap from [=[nxlib/stdlib/hashmap.nx]=]

let eq_half = fn (left: i64, right: i64) -> bool do
  return (left / 2) == (right / 2)
endfn

let hash_half = fn (key: i64) -> i64 do
  return key / 2
endfn

let main = fn () -> i64 do
  let ops = hashmap.make_key_ops(eq: eq_half, hash: hash_half)
  let m0 = hashmap.empty(key_ops: ops)
  let m1 = hashmap.put(map: m0, key: 4, value: 40)
  return hashmap.get_or(map: m1, key: 5, default: -1)
endfn
"#;
    assert_eq!(run(src).unwrap(), Value::Int(40));
}
