use crate::harness::{exec_with_stdlib, exec_with_stdlib_caps_should_trap, should_fail_typecheck};
use nexus::runtime::ExecutionCapabilities;

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
fn clock_denied_at_wasi_level_without_allow_clock() {
    let caps = ExecutionCapabilities {
        allow_console: true,
        allow_fs: true,
        ..ExecutionCapabilities::deny_all()
    };
    let err = exec_with_stdlib_caps_should_trap(
        r#"
import { Clock }, * as clk from stdlib/clock.nx

let main = fn () -> unit require { PermClock } do
  inject clk.system_handler do
    let _ = Clock.now()
    return ()
  end
end
"#,
        caps,
    );
    // The stub returns ENOSYS (errno 76), which the stdlib propagates as a wasm trap.
    // The eprintln message goes to stderr, not the trap string.
    assert!(
        err.contains("error while executing"),
        "expected wasm trap from denied clock access, got: {}",
        err
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
    insta::assert_snapshot!(err);
}
