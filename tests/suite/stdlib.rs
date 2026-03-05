

use crate::common::source::{check, run};
use nexus::interpreter::Value;

#[test]
fn test_to_string_runtime_error() {
    let src = r#"
import { to_string } from nxlib/stdlib/exn.nx

let main = fn () -> string do
  let e: Exn = RuntimeError(val: "boom")
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
  raise RuntimeError(val: "oops")
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
    let src = &crate::common::fixtures::read_test_fixture("set_insert_contains_and_size.nx");
    assert_eq!(run(src).unwrap(), Value::Int(2));
}

#[test]
fn set_union_intersection_difference() {
    let src = &crate::common::fixtures::read_test_fixture("set_union_intersection_difference.nx");
    assert_eq!(run(src).unwrap(), Value::Int(412));
}

#[test]
fn set_custom_key_ops_can_change_membership_rule() {
    let src = &crate::common::fixtures::read_test_fixture("set_custom_key_ops_can_change_membership_rule.nx");
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}



#[test]
fn hashmap_put_get_or_and_contains_key() {
    let src = &crate::common::fixtures::read_test_fixture("hashmap_put_get_or_and_contains_key.nx");
    assert_eq!(run(src).unwrap(), Value::Int(126));
}

#[test]
fn hashmap_get_lookup_and_remove() {
    let src = &crate::common::fixtures::read_test_fixture("hashmap_get_lookup_and_remove.nx");
    assert_eq!(run(src).unwrap(), Value::Int(71));
}

#[test]
fn hashmap_custom_key_ops_can_change_key_equivalence() {
    let src = &crate::common::fixtures::read_test_fixture("hashmap_custom_key_ops_can_change_key_equivalence.nx");
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
  let exn = RuntimeError(val: "boom")
  let r = result.from_exn(exn: exn)
  return result.is_err(res: r)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn result_to_exn_raises_and_is_catchable() {
    let src = &crate::common::fixtures::read_test_fixture("result_to_exn_raises_and_is_catchable.nx");
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

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_math_max_symmetry(a in -1000i64..1000, b in -1000i64..1000) {
        let src1 = format!("
import {{ max }} from nxlib/stdlib/math.nx
let main = fn () -> i64 do return max(a: {}, b: {}) end
", a, b);
        let src2 = format!("
import {{ max }} from nxlib/stdlib/math.nx
let main = fn () -> i64 do return max(a: {}, b: {}) end
", b, a);
        let val1 = run(&src1).unwrap();
        let val2 = run(&src2).unwrap();
        assert_eq!(val1, val2);
    }

    #[test]
    fn prop_math_min_symmetry(a in -1000i64..1000, b in -1000i64..1000) {
        let src1 = format!("
import {{ min }} from nxlib/stdlib/math.nx
let main = fn () -> i64 do return min(a: {}, b: {}) end
", a, b);
        let src2 = format!("
import {{ min }} from nxlib/stdlib/math.nx
let main = fn () -> i64 do return min(a: {}, b: {}) end
", b, a);
        assert_eq!(run(&src1).unwrap(), run(&src2).unwrap());
    }

    #[test]
    fn prop_math_max_gte(a in -1000i64..1000, b in -1000i64..1000) {
        let src = format!("
import {{ max }} from nxlib/stdlib/math.nx
let main = fn () -> bool do
    let m = max(a: {}, b: {})
    if m >= {} then
        return m >= {}
    else
        return false
    end
end
", a, b, a, b);
        assert_eq!(run(&src).unwrap(), Value::Bool(true));
    }

    #[test]
    fn prop_string_length_concat(s1 in "[a-zA-Z0-9]{0,20}", s2 in "[a-zA-Z0-9]{0,20}") {
        let src = format!("
import {{ length }} from nxlib/stdlib/string.nx
let main = fn () -> bool do
    let s1 = [=[{}]=]
    let s2 = [=[{}]=]
    let concat = s1 ++ s2
    return length(s: concat) == (length(s: s1) + length(s: s2))
end
", s1, s2);
        assert_eq!(run(&src).unwrap(), Value::Bool(true));
    }
}
