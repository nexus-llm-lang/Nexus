mod common;

use common::source::{check, run};
use nexus::interpreter::Value;

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
