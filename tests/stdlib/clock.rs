use crate::common::check::should_fail_typecheck;
use crate::common::wasm::exec_with_stdlib;

#[test]
fn clock_now_returns_positive_value() {
    exec_with_stdlib(
        r#"
import { Clock }, * as clk from stdlib/clock.nx

let main = fn () -> unit require { PermClock } do
  inject clk.system_handler do
    let t = Clock.now()
    if t <= 0 then raise RuntimeError(val: "clock not positive") end
    return ()
  end
end
"#,
    );
}

#[test]
fn clock_sleep_does_not_crash() {
    exec_with_stdlib(
        r#"
import { Clock }, * as clk from stdlib/clock.nx

let main = fn () -> unit require { PermClock } do
  inject clk.system_handler do
    Clock.sleep(ms: 10)
    return ()
  end
end
"#,
    );
}

#[test]
fn clock_requires_perm_clock() {
    let err = should_fail_typecheck(
        r#"
import { Clock }, * as clk from stdlib/clock.nx

let main = fn () -> i64 do
  inject clk.system_handler do
    return Clock.now()
  end
end
"#,
    );
    assert!(
        !err.is_empty(),
        "Clock.now without PermClock should fail typechecking"
    );
}
