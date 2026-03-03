mod common;


use common::source::{check, run};
use nexus::interpreter::Value;

#[test]
fn test_to_string_runtime_error() {
    let src = r#"
import { to_string } from nxlib/stdlib/exn.nx

let main = fn () -> string do
  let e: Exn = RuntimeError(val: [=[boom]=])
  return to_string(exn: e)
end
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::String("RuntimeError: boom".to_string())
    );
}

#[test]
fn test_to_string_invalid_index() {
    let src = r#"
import { to_string } from nxlib/stdlib/exn.nx

let main = fn () -> string do
  let e: Exn = InvalidIndex(val: 42)
  return to_string(exn: e)
end
"#;
    assert_eq!(
        run(src).unwrap(),
        Value::String("InvalidIndex: 42".to_string())
    );
}

#[test]
fn test_backtrace_returns_frames() {
    let src = r#"
import { backtrace } from nxlib/stdlib/exn.nx

let inner = fn () -> unit effect { Exn } do
  raise RuntimeError(val: [=[oops]=])
  return ()
end

let outer = fn () -> [string] do
  try
    inner()
  catch e ->
    return backtrace(exn: e)
  end
  return []
end

let main = fn () -> [string] do
  return outer()
end
"#;
    let result = run(src).unwrap();
    // The backtrace should be a non-empty Cons list containing "inner"
    let mut frames = Vec::new();
    let mut current = result;
    loop {
        match &current {
            Value::Variant(name, args) if name == "Cons" && args.len() == 2 => {
                if let Value::String(s) = &args[0] {
                    frames.push(s.clone());
                }
                current = args[1].clone();
            }
            Value::Variant(name, _) if name == "Nil" => break,
            _ => break,
        }
    }
    assert!(
        frames.contains(&"inner".to_string()),
        "backtrace should contain 'inner', got: {:?}",
        frames
    );
}



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



#[test]
fn hashmap_put_get_or_and_contains_key() {
    let src = r#"
import as hashmap from nxlib/stdlib/hashmap.nx

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
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(126));
}

#[test]
fn hashmap_get_lookup_and_remove() {
    let src = r#"
import as hashmap from nxlib/stdlib/hashmap.nx

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
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(71));
}

#[test]
fn hashmap_custom_key_ops_can_change_key_equivalence() {
    let src = r#"
import as hashmap from nxlib/stdlib/hashmap.nx

let eq_half = fn (left: i64, right: i64) -> bool do
  return (left / 2) == (right / 2)
end

let hash_half = fn (key: i64) -> i64 do
  return key / 2
end

let main = fn () -> i64 do
  let ops = hashmap.make_key_ops(eq: eq_half, hash: hash_half)
  let m0 = hashmap.empty(key_ops: ops)
  let m1 = hashmap.put(map: m0, key: 4, value: 40)
  return hashmap.get_or(map: m1, key: 5, default: -1)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Int(40));
}



#[test]
fn clock_now_returns_positive_value() {
    let src = r#"
import { Clock }, * as clk from nxlib/stdlib/clock.nx

let main = fn () -> bool require { PermClock } do
  inject clk.system_handler do
    let t = Clock.now()
    return t > 0
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn clock_sleep_does_not_crash() {
    let src = r#"
import { Clock }, * as clk from nxlib/stdlib/clock.nx

let main = fn () -> bool require { PermClock } do
  inject clk.system_handler do
    Clock.sleep(ms: 10)
    return true
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn clock_requires_perm_clock() {
    let src = r#"
import { Clock }, * as clk from nxlib/stdlib/clock.nx

let main = fn () -> i64 do
  inject clk.system_handler do
    return Clock.now()
  end
end
"#;
    assert!(
        check(src).is_err(),
        "Clock.now without PermClock should fail typechecking"
    );
}



#[test]
fn random_range_returns_in_bounds_value() {
    let src = r#"
import { Random }, * as rng from nxlib/stdlib/random.nx

let main = fn () -> bool require { PermRandom } do
  inject rng.system_handler do
    let n = Random.range(min: 10, max: 20)
    if n >= 10 then
      return n < 20
    else
      return false
    end
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn random_range_requires_perform() {
    let src = r#"
import { Random }, * as rng from nxlib/stdlib/random.nx

let main = fn () -> i64 do
  inject rng.system_handler do
    let n = Random.range(min: 0, max: 10)
    return n
  end
end
"#;
    assert!(
        check(src).is_err(),
        "random.range without PermRandom should fail typechecking"
    );
}



#[test]
fn proc_exit_typechecks_with_perm_proc() {
    let src = r#"
import { Proc }, * as proc_mod from nxlib/stdlib/proc.nx

let main = fn () -> unit require { PermProc } do
  inject proc_mod.system_handler do
    Proc.exit(status: 0)
  end
end
"#;
    assert!(check(src).is_ok(), "Proc.exit with PermProc should typecheck");
}

#[test]
fn proc_exit_requires_perm_proc() {
    let src = r#"
import { Proc }, * as proc_mod from nxlib/stdlib/proc.nx

let main = fn () -> unit do
  inject proc_mod.system_handler do
    Proc.exit(status: 0)
  end
end
"#;
    assert!(
        check(src).is_err(),
        "Proc.exit without PermProc should fail typechecking"
    );
}

#[test]
fn proc_port_with_mock_handler() {
    // Test that Proc port can be implemented with a mock handler
    // (doesn't actually exit the process)
    let src = r#"
import { Proc } from nxlib/stdlib/proc.nx

let mock_proc = handler Proc do
  fn exit(status: i64) -> unit do
    return ()
  end
end

let main = fn () -> unit do
  inject mock_proc do
    Proc.exit(status: 0)
  end
end
"#;
    assert!(check(src).is_ok(), "Mock Proc handler should typecheck");
}



#[test]
fn result_from_exn_builds_err() {
    let src = r#"
import as result from nxlib/stdlib/result.nx

let main = fn () -> bool do
  let exn = RuntimeError(val: [=[boom]=])
  let r = result.from_exn(exn: exn)
  return result.is_err(res: r)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn result_to_exn_raises_and_is_catchable() {
    let src = r#"
import as result from nxlib/stdlib/result.nx

let main = fn () -> bool do
  let err: Result<i64, Exn> = Err(err: RuntimeError(val: [=[boom]=]))
  try
    let _ = result.to_exn(res: err)
    return false
  catch e ->
    return true
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_math_abs_non_negative(n in -1000i64..1000) {
        let src = format!("
import {{ abs }} from nxlib/stdlib/math.nx
let main = fn () -> bool do
    let x = abs(val: {n})
    return x >= 0
end
");
        assert_eq!(run(&src).unwrap(), Value::Bool(true));
    }
}
