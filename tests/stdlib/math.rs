use crate::harness::exec_with_stdlib;

#[test]
fn negate_in_math_module() {
    exec_with_stdlib(
        r#"
import { negate } from stdlib/math.nx

let main = fn () -> unit do
  let result = negate(val: false)
  if result != true then raise RuntimeError(val: "expected true") end
  return ()
end
"#,
    );
}

#[test]
fn core_id_returns_argument() {
    exec_with_stdlib(
        r#"
import { id } from stdlib/core.nx

let main = fn () -> unit do
  let result = id(val: 42)
  if result != 42 then raise RuntimeError(val: "expected 42") end
  return ()
end
"#,
    );
}

#[test]
fn random_range_returns_in_bounds_value() {
    exec_with_stdlib(
        r#"
import { Random }, * as rng from stdlib/random.nx

let main = fn () -> unit require { PermRandom } do
  inject rng.system_handler do
    let n = Random.range(min: 10, max: 20)
    if n >= 10 then
      if n < 20 then
        return ()
      else
        raise RuntimeError(val: "n >= 20")
      end
    else
      raise RuntimeError(val: "n < 10")
    end
  end
end
"#,
    );
}

#[test]
fn random_range_requires_perform() {
    let err = crate::harness::should_fail_typecheck(
        r#"
import { Random }, * as rng from stdlib/random.nx

let main = fn () -> i64 do
  inject rng.system_handler do
    let n = Random.range(min: 0, max: 10)
    return n
  end
end
"#,
    );
    assert!(
        !err.is_empty(),
        "random.range without PermRandom should fail typechecking"
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
    fn prop_math_abs_non_negative(n in -1000i64..1000) {
        let src = format!("
import {{ abs }} from stdlib/math.nx
let main = fn () -> unit do
    let x = abs(val: {n})
    if x < 0 then raise RuntimeError(val: \"abs returned negative\") end
    return ()
end
");
        exec_with_stdlib(&src);
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
        let src = format!("
import {{ max }} from stdlib/math.nx
let main = fn () -> unit do
    let v1 = max(a: {a}, b: {b})
    let v2 = max(a: {b}, b: {a})
    if v1 != v2 then raise RuntimeError(val: \"max not symmetric\") end
    return ()
end
");
        exec_with_stdlib(&src);
    }

    #[test]
    fn prop_math_min_symmetry(a in -1000i64..1000, b in -1000i64..1000) {
        let src = format!("
import {{ min }} from stdlib/math.nx
let main = fn () -> unit do
    let v1 = min(a: {a}, b: {b})
    let v2 = min(a: {b}, b: {a})
    if v1 != v2 then raise RuntimeError(val: \"min not symmetric\") end
    return ()
end
");
        exec_with_stdlib(&src);
    }

    #[test]
    fn prop_math_max_gte(a in -1000i64..1000, b in -1000i64..1000) {
        let src = format!("
import {{ max }} from stdlib/math.nx
let main = fn () -> unit do
    let m = max(a: {a}, b: {b})
    if m >= {a} then
        if m >= {b} then
            return ()
        else
            raise RuntimeError(val: \"max < b\")
        end
    else
        raise RuntimeError(val: \"max < a\")
    end
end
");
        exec_with_stdlib(&src);
    }
}
