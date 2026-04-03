use crate::harness::{compile, exec, exec_with_stdlib};
use std::fs;

#[test]
fn codegen_module_alias_call_compiles() {
    exec(
        r#"
import * as math from "examples/math.nx"

let main = fn () -> unit do
    let result = math.add(a: 19, b: 23)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_fixture_fib_works_in_wasm() {
    let src = fs::read_to_string("examples/fib.nx").expect("fixture should exist");
    exec_with_stdlib(&src);
}

#[test]
fn codegen_fixture_di_port_compiles() {
    let src = fs::read_to_string("examples/di_port.nx").expect("fixture should exist");
    exec_with_stdlib(&src);
}

#[test]
fn codegen_fixture_module_test_compiles() {
    let src = fs::read_to_string("examples/module_test.nx").expect("fixture should exist");
    exec_with_stdlib(&src);
}

#[test]
fn codegen_fixture_network_access_compiles() {
    let src = fs::read_to_string("examples/network_access.nx").expect("fixture should exist");
    let wasm = compile(&src);
    assert!(!wasm.is_empty(), "compiled wasm should not be empty");
}

#[test]
fn codegen_print_works_via_external_stdio_module() {
    exec_with_stdlib(
        r#"
import { Console }, * as stdio from "stdlib/stdio.nx"

let main = fn () -> unit require { PermConsole } do
    inject stdio.system_handler do
        Console.println(val: "hello wasm")
    end
    return ()
end
"#,
    );
}

#[test]
fn codegen_print_after_from_i64_works_via_single_string_abi_module() {
    exec_with_stdlib(
        r#"
import { Console }, * as stdio from "stdlib/stdio.nx"
import { from_i64 } from "stdlib/string.nx"

let main = fn () -> unit require { PermConsole } do
    let s = from_i64(val: 42)
    inject stdio.system_handler do
        Console.println(val: s)
    end
    return ()
end
"#,
    );
}

#[test]
fn codegen_handler_reachability_resolves_port_call() {
    exec_with_stdlib(
        r#"
import { Console }, * as stdio from "stdlib/stdio.nx"

let main = fn () -> unit require { PermConsole } do
    inject stdio.system_handler do
        Console.print(val: "hello")
    end
    return ()
end
"#,
    );
}

#[test]
fn codegen_tail_recursive_function_executes_correctly() {
    exec(
        r#"
let sum_tail = fn (n: i64, acc: i64) -> i64 do
    if n <= 0 then return acc end
    return sum_tail(n: n - 1, acc: acc + n)
end

let main = fn () -> unit do
    let result = sum_tail(n: 100, acc: 0)
    if result != 5050 then raise RuntimeError(val: "expected 5050") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_deep_tail_recursion_does_not_overflow() {
    exec(
        r#"
let loop_n = fn (n: i64) -> i64 do
    if n <= 0 then return 0 end
    return loop_n(n: n - 1)
end

let main = fn () -> unit do
    let result = loop_n(n: 1000000)
    if result != 0 then raise RuntimeError(val: "expected 0") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_deep_mutual_tail_recursion_does_not_overflow() {
    exec(
        r#"
let is_even = fn (n: i64) -> bool do
    if n == 0 then return true end
    return is_odd(n: n - 1)
end

let is_odd = fn (n: i64) -> bool do
    if n == 0 then return false end
    return is_even(n: n - 1)
end

let main = fn () -> unit do
    let result = is_even(n: 1000000)
    if result != true then raise RuntimeError(val: "expected true") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_main_with_args_runs_with_stdlib() {
    exec_with_stdlib(
        r#"
let main = fn (args: [string]) -> unit do
    return ()
end
"#,
    );
}

#[test]
fn codegen_labeled_args_reordered_function_call() {
    exec(
        r#"
let add = fn (a: i64, b: i64) -> i64 do
    return a - b
end

let main = fn () -> unit do
    let result = add(b: 10, a: 52)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn codegen_labeled_args_reordered_constructor() {
    exec(
        r#"
type Pair = MkPair(fst: i64, snd: i64)

let main = fn () -> unit do
    let p = MkPair(snd: 10, fst: 32)
    match p do
      case MkPair(fst: a, snd: b) ->
        if a + b != 42 then raise RuntimeError(val: "expected 42") end
    end
    return ()
end
"#,
    );
}

#[test]
fn codegen_labeled_args_constructor_pattern_match_same_result() {
    exec(
        r#"
type Pair = MkPair(fst: i64, snd: i64)

let main = fn () -> unit do
    let p1 = MkPair(fst: 1, snd: 2)
    let p2 = MkPair(snd: 2, fst: 1)
    match p1 do
      case MkPair(fst: a1, snd: b1) ->
        match p2 do
          case MkPair(fst: a2, snd: b2) ->
            if a1 != a2 then raise RuntimeError(val: "fst mismatch") end
            if b1 != b2 then raise RuntimeError(val: "snd mismatch") end
        end
    end
    return ()
end
"#,
    );
}

#[test]
fn codegen_labeled_args_tail_call_reordered() {
    exec(
        r#"
let sub_tail = fn (a: i64, b: i64) -> i64 do
    if b == 0 then return a end
    return sub_tail(b: b - 1, a: a)
end

let main = fn () -> unit do
    let result = sub_tail(a: 42, b: 0)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn higher_order_function_passed_as_argument() {
    exec(
        r#"
let double = fn (x: i64) -> i64 do
    return x * 2
end

let apply = fn (f: (x: i64) -> i64, val: i64) -> i64 do
    return f(x: val)
end

let main = fn () -> unit do
    let result = apply(f: double, val: 21)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn higher_order_lambda_expression_passed_as_argument() {
    exec(
        r#"
let apply = fn (f: (x: i64) -> i64, val: i64) -> i64 do
    return f(x: val)
end

let main = fn () -> unit do
    let result = apply(f: fn (x: i64) -> i64 do return x + 1 end, val: 41)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn higher_order_function_stored_in_variable() {
    exec(
        r#"
let add_one = fn (x: i64) -> i64 do
    return x + 1
end

let main = fn () -> unit do
    let f = add_one
    let result = f(x: 41)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn higher_order_function_multiple_params() {
    exec(
        r#"
let add = fn (a: i64, b: i64) -> i64 do
    return a + b
end

let apply2 = fn (f: (a: i64, b: i64) -> i64, x: i64, y: i64) -> i64 do
    return f(a: x, b: y)
end

let main = fn () -> unit do
    let result = apply2(f: add, x: 19, y: 23)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn closure_captures_single_variable() {
    exec(
        r#"
let make_adder = fn (n: i64) -> (x: i64) -> i64 do
    return fn (x: i64) -> i64 do
        return x + n
    end
end

let main = fn () -> unit do
    let add5 = make_adder(n: 5)
    let result = add5(x: 37)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn closure_captures_multiple_variables() {
    exec(
        r#"
let make_linear = fn (a: i64, b: i64) -> (x: i64) -> i64 do
    return fn (x: i64) -> i64 do
        return a * x + b
    end
end

let main = fn () -> unit do
    let f = make_linear(a: 2, b: 10)
    let result = f(x: 16)
    if result != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

#[test]
fn closure_and_non_closure_same_type() {
    exec(
        r#"
let double = fn (x: i64) -> i64 do
    return x * 2
end

let make_adder = fn (n: i64) -> (x: i64) -> i64 do
    return fn (x: i64) -> i64 do
        return x + n
    end
end

let apply = fn (f: (x: i64) -> i64, val: i64) -> i64 do
    return f(x: val)
end

let main = fn () -> unit do
    let r1 = apply(f: double, val: 21)
    if r1 != 42 then raise RuntimeError(val: "expected 42 from double") end
    let add10 = make_adder(n: 10)
    let r2 = apply(f: add10, val: 32)
    if r2 != 42 then raise RuntimeError(val: "expected 42 from adder") end
    return ()
end
"#,
    );
}

// ---- TCMC (Tail Call Modulo Constructor) ----

#[test]
fn tcmc_list_map_produces_correct_result() {
    exec(
        r#"
type IntList = Nil | Cons(v: i64, rest: IntList)

let map_double = fn (xs: IntList) -> IntList do
    match xs do
        case Nil -> return Nil
        case Cons(v: v, rest: rest) ->
            return Cons(v: v * 2, rest: map_double(xs: rest))
    end
end

let sum = fn (xs: IntList, acc: i64) -> i64 do
    match xs do
        case Nil -> return acc
        case Cons(v: v, rest: rest) -> return sum(xs: rest, acc: acc + v)
    end
end

let main = fn () -> unit do
    let xs = Cons(v: 1, rest: Cons(v: 2, rest: Cons(v: 3, rest: Nil)))
    let doubled = map_double(xs: xs)
    let result = sum(xs: doubled, acc: 0)
    if result != 12 then raise RuntimeError(val: "expected 12") end
    return ()
end
"#,
    );
}

#[test]
fn tcmc_deep_list_build_does_not_overflow() {
    // iota uses TCMC pattern (Cons(n, iota(n-1))): should build long lists without stack overflow
    // head extracts the first element to verify correctness
    exec(
        r#"
type IntList = Nil | Cons(v: i64, rest: IntList)

let iota = fn (n: i64) -> IntList do
    if n <= 0 then return Nil end
    return Cons(v: n, rest: iota(n: n - 1))
end

let head = fn (xs: IntList) -> i64 do
    match xs do
        case Nil -> return 0
        case Cons(v: v, rest: _) -> return v
    end
end

let length_acc = fn (xs: IntList, acc: i64) -> i64 do
    match xs do
        case Nil -> return acc
        case Cons(v: _, rest: rest) -> return length_acc(xs: rest, acc: acc + 1)
    end
end

let main = fn () -> unit do
    let xs = iota(n: 5000)
    let h = head(xs: xs)
    if h != 5000 then raise RuntimeError(val: "expected head 5000") end
    let len = length_acc(xs: xs, acc: 0)
    if len != 5000 then raise RuntimeError(val: "expected length 5000") end
    return ()
end
"#,
    );
}
