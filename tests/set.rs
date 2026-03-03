mod common;

use common::source::run;
use nexus::interpreter::Value;

#[test]
fn set_insert_contains_and_size() {
    let src = r#"
import as set from nxlib/stdlib/set.nx

let main = fn () -> i64 do
  let ops = set.i64_key_ops()
  let s0 = set.empty(key_ops: ops)
  let s1 = set.insert(set: s0, val: 10)
  let s2 = set.insert(set: s1, val: 20)
  let s3 = set.insert(set: s2, val: 10)
  let has_10 = set.contains(set: s3, val: 10)
  if has_10 then
    return set.size(set: s3)
  else
    return -1
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(2));
}

#[test]
fn set_union_intersection_difference() {
    let src = r#"
import as set from nxlib/stdlib/set.nx

let main = fn () -> i64 do
  let ax = [1, 2, 3]
  let bx = [3, 4]
  let ops = set.i64_key_ops()
  let a = set.from_list(key_ops: ops, xs: ax)
  let b = set.from_list(key_ops: ops, xs: bx)
  let u = set.union(left: a, right: b)
  let i = set.intersection(left: a, right: b)
  let d = set.difference(left: a, right: b)
  let u_part = set.size(set: u) * 100
  let i_part = set.size(set: i) * 10
  let d_part = set.size(set: d)
  return u_part + i_part + d_part
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(412));
}

#[test]
fn set_custom_key_ops_can_change_membership_rule() {
    let src = r#"
import as set from nxlib/stdlib/set.nx

let eq_half = fn (left: i64, right: i64) -> bool do
  return (left / 2) == (right / 2)
end

let hash_half = fn (key: i64) -> i64 do
  return key / 2
end

let main = fn () -> bool do
  let ops = set.make_key_ops(eq: eq_half, hash: hash_half)
  let s0 = set.empty(key_ops: ops)
  let s1 = set.insert(set: s0, val: 4)
  return set.contains(set: s1, val: 5)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
