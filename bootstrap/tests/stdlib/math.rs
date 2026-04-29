use crate::harness::{exec_nxc_core_capture_stdout, exec_with_stdlib, exec_with_stdlib_caps_should_trap};
use nexus::runtime::ExecutionCapabilities;

#[test]
fn negate_in_math_module() {
    exec_with_stdlib(
        r#"
import { negate } from "std:math"

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
import { id } from "std:core"

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
    // Routed through the self-hosted (nxc) compile path because the
    // pure-Nexus PRNG (nexus-dvr6.9.6) reads/writes a 16-byte state cell
    // via `nexus:runtime/memory` ops. Those ops are inlined as wasm
    // memory instructions by the nxc codegen but trap as
    // "not provisionable" when sourced through the Rust-built component
    // path (`exec_with_stdlib`).
    let out = exec_nxc_core_capture_stdout(
        "bootstrap/tests/fixtures/nxc/test_rand_range_in_bounds.nx",
    );
    let lines: Vec<&str> = out.lines().map(str::trim).collect();
    assert_eq!(
        lines,
        ["ok-lazy-seed", "ok-explicit-seed"],
        "unexpected stdout: {:?}",
        out
    );
}

#[test]
fn random_range_requires_perform() {
    let err = crate::harness::should_fail_typecheck(
        r#"
import { Random }, * as rng from "std:rand"

let main = fn () -> i64 do
  inject rng.system_handler do
    let n = Random.range(min: 0, max: 10)
    return n
  end
end
"#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn random_traps_on_rust_compiler_path_without_memory_intrinsics() {
    // The pure-Nexus PRNG installed by nexus-dvr6.9.6 reads/writes a
    // 16-byte state cell at offset 983040 via the
    // `nexus:runtime/memory` intrinsics. Those ops are inlined by the
    // self-hosted (nxc) codegen but trap as "not provisionable" when
    // sourced through the Rust-built component path, regardless of
    // capability flags.
    //
    // This test pins the architectural fact that `exec_with_stdlib`
    // (Rust compiler + component-model linker) cannot exercise the new
    // Random handler. Functional Random tests must route through the
    // nxc fixture path (see `random_range_returns_in_bounds_value` and
    // `stdlib::rand::rand_determinism_pcg_step_byte_equal_across_runs`).
    //
    // The previous incarnation of this test asserted the WASI-level
    // `random_get` deny path. After dvr6.9.6 the PRNG no longer touches
    // `random_get`; the equivalent runtime-side gate is now Clock denial
    // (the seed source is `__nx_now`/`clock_time_get`), which is covered
    // by the analogous test in `stdlib/clock.rs`.
    let caps = ExecutionCapabilities {
        allow_console: true,
        allow_fs: true,
        allow_clock: true,
        allow_random: true,
        ..ExecutionCapabilities::deny_all()
    };
    let err = exec_with_stdlib_caps_should_trap(
        r#"
import { Random }, * as rng from "std:rand"

let main = fn () -> unit require { PermRandom } do
  inject rng.system_handler do
    let _ = Random.range(min: 0, max: 10)
    return ()
  end
end
"#,
        caps,
    );
    assert!(
        err.contains("not provisionable")
            || err.contains("error while executing"),
        "expected runtime/memory provisioning trap on Rust compiler path, got: {}",
        err
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
import {{ abs }} from \"stdlib/math.nx\"
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
import {{ max }} from \"stdlib/math.nx\"
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
import {{ min }} from \"stdlib/math.nx\"
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
import {{ max }} from \"stdlib/math.nx\"
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
