use crate::harness::{should_fail_typecheck, should_typecheck};

/// Positive: spawn followed by join typechecks. The %Task<T> handle is
/// consumed by host_join exactly once.
#[test]
fn lazy_host_spawn_then_join_typechecks() {
    should_typecheck(
        r#"
import * as lazy from "std:lazy"

let main = fn () -> unit do
  let @x = 42
  let t = lazy.host_spawn(a: x)
  let _v = lazy.host_join(handle: t)
  return ()
end
"#,
    );
}

/// Positive: host_force still composes spawn + join correctly.
#[test]
fn lazy_host_force_typechecks() {
    should_typecheck(
        r#"
import * as lazy from "std:lazy"

let main = fn () -> unit do
  let @x = 42
  let _v = lazy.host_force(a: x)
  return ()
end
"#,
    );
}

/// Negative (Acceptance #3 for nexus-lrg3): spawn-without-join must be a
/// linearity error. Prior to wrapping the runtime task id in `%Task<T>`,
/// the raw i64 was non-linear, so dropping the handle silently leaked the
/// runtime's tasks-map entry and (for threaded thunks) the Rust JoinHandle.
///
/// Uses a uniquely named local (`leaked_handle`) and asserts the specific
/// name appears in the diagnostic — guards against false positives from
/// unrelated linearity errors elsewhere in the imported lazy module.
#[test]
fn lazy_spawn_without_join_is_rejected() {
    let err = should_fail_typecheck(
        r#"
import * as lazy from "std:lazy"

let leak = fn () -> unit do
  let @x = 42
  let leaked_handle = lazy.host_spawn(a: x)
  return ()
end

let main = fn () -> unit do
  leak()
  return ()
end
"#,
    );
    assert!(
        err.contains("Unused linear") && err.contains("leaked_handle"),
        "expected linearity error naming `leaked_handle`, got: {err}"
    );
}

/// Negative: dropping a Task explicitly via statement-level `let _` must
/// also fail — `let _ = expr` rebinds the linear obligation to the name
/// `_`, it doesn't discharge it. Only `host_join` (or `host_force`)
/// discharges the linearity by destructuring the Task constructor.
#[test]
fn lazy_spawn_underscore_drop_is_rejected() {
    let err = should_fail_typecheck(
        r#"
import * as lazy from "std:lazy"

let main = fn () -> unit do
  let @x = 42
  let _ = lazy.host_spawn(a: x)
  return ()
end
"#,
    );
    assert!(
        err.contains("Unused linear"),
        "expected linearity error, got: {err}"
    );
}
