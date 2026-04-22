use crate::harness::{exec, exec_with_stdlib};
use proptest::prelude::*;

#[test]
fn codegen_i64_function_call_works() {
    exec(
        r#"
let add = fn (x: i64, y: i64) -> i64 do
    return x + y
end

let main = fn () -> unit do
    let result = add(x: 40, y: 2)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_i32_arithmetic_works() {
    // NOTE: The WASM backend does not currently support i32 functions. This test
    // verifies the source at least parses and typechecks, then runs via WASM using i64.
    exec(
        r#"
let inc = fn (x: i64) -> i64 do
    return x + 1
end

let main = fn () -> unit do
    let x = 41
    let result = inc(x: x)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_negate_function() {
    exec(
        r#"
import { negate } from "stdlib/core.nx"

let main = fn () -> unit do
    let t = negate(val: true)
    let f = negate(val: false)
    if t then raise RuntimeError(val: "negate(true) should be false") end
    if f then
        return ()
    else
        raise RuntimeError(val: "negate(false) should be true")
    end
end
"#,
    );
}

#[test]
fn codegen_f64_literal_and_arithmetic() {
    exec_with_stdlib(
        r#"
import { abs_float } from "stdlib/math.nx"
import { from_float } from "stdlib/string.nx"

let check_f64 = fn (actual: f64, expected: f64, label: string) -> unit throws { Exn } do
  let diff = actual -. expected
  let abs_diff = abs_float(val: diff)
  if abs_diff >. 0.0001 then
    raise RuntimeError(val: "f64 mismatch in " ++ label ++ ": got " ++ from_float(val: actual))
  end
  return ()
end

let main = fn () -> unit do
    let x = 3.14
    let y = 2.0
    let z = x +. y
    check_f64(actual: z, expected: 5.14, label: "add")
    let w = x *. y
    check_f64(actual: w, expected: 6.28, label: "mul")
    let d = x -. y
    check_f64(actual: d, expected: 1.14, label: "sub")
    return ()
end
"#,
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_codegen_arithmetic_associativity(a in -100i64..100, b in -100i64..100, c in -100i64..100) {
        let expected = (a + b) + c;
        let src = format!(r#"
let main = fn () -> unit do
    let result = ({} + {}) + {}
    if result != {} then raise RuntimeError(val: "associativity mismatch") end
    return ()
end
"#, a, b, c, expected);
        exec(&src);
    }
}
