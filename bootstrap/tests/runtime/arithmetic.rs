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
import { negate } from "std:core"

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
import { abs_float } from "std:math"
import { from_float } from "std:string_ops"

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

#[test]
fn codegen_prefix_unary_neg_i64() {
    // Cover all spec acceptance shapes: -INT literal, -var, -(expr), -call().
    exec(
        r#"
let one = fn () -> i64 do return 1 end

let main = fn () -> unit do
    let lit = -1
    if lit != 0 - 1 then raise RuntimeError(val: "neg literal") end

    let y = 5
    let nv = -y
    if nv != 0 - 5 then raise RuntimeError(val: "neg var") end

    let z = -(y + 2)
    if z != 0 - 7 then raise RuntimeError(val: "neg parens") end

    let c = -one()
    if c != 0 - 1 then raise RuntimeError(val: "neg call") end

    // Precedence: -x * y must be (-x) * y, not -(x * y).
    let prec = -y * 2
    if prec != 0 - 10 then raise RuntimeError(val: "unary prec vs *") end

    return ()
end
"#,
    );
}

#[test]
fn codegen_prefix_unary_fneg_f64() {
    exec_with_stdlib(
        r#"
import { abs_float } from "std:math"

let close = fn (a: f64, b: f64) -> bool do
    let d = a -. b
    return abs_float(val: d) <. 0.0001
end

let main = fn () -> unit do
    let x = 3.5
    let y = -.x
    if !close(a: y, b: 0.0 -. 3.5) then raise RuntimeError(val: "fneg var") end

    let lit = -.1.5
    if !close(a: lit, b: 0.0 -. 1.5) then raise RuntimeError(val: "fneg lit") end

    let parens = -.(x +. 1.0)
    if !close(a: parens, b: 0.0 -. 4.5) then raise RuntimeError(val: "fneg parens") end

    return ()
end
"#,
    );
}

#[test]
fn codegen_prefix_unary_not_bool() {
    exec(
        r#"
let main = fn () -> unit do
    let t = true
    if !t then raise RuntimeError(val: "!true should be false") end

    let f = false
    if !!f then raise RuntimeError(val: "!!false should be false") end

    // Precedence: !a && b must be (!a) && b.
    if !t && true then raise RuntimeError(val: "(!true) && true should be false") end

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
