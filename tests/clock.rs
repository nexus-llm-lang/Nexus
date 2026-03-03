mod common;

use common::source::{check, run};
use nexus::interpreter::Value;

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
