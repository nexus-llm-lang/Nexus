mod common;

use common::source::{check_raw as check, run_raw};
use nexus::interpreter::Value;
use nexus::lang::parser;
use proptest::prelude::*;

fn run(src: &str) -> Result<Value, String> {
    run_raw(src, "__test_main")
}

fn io_program(body: &str) -> String {
    format!(
        r#"
type IO = {{}}

let io = fn (x: i64) -> unit effect {{ IO }} do
    return ()
end

let helper = fn () -> unit effect {{ IO }} do
{body}
    return ()
end

let main = fn () -> unit do
    return ()
end
"#
    )
}

fn pure_program(body: &str) -> String {
    format!(
        r#"
let pure = fn (x: i64) -> unit do
    return ()
end

let main = fn () -> unit do
{body}
    return ()
end
"#
    )
}

fn permutation_from_seed(n: usize, mut seed: u64) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let j = (seed as usize) % (i + 1);
        idx.swap(i, j);
    }
    idx
}

fn labeled_call_order_program(digits: &[i64], perm: &[usize], via_function_value: bool) -> String {
    let n = digits.len();
    let labels = (0..n).map(|i| format!("a{}", i)).collect::<Vec<_>>();
    let params = labels
        .iter()
        .map(|label| format!("{label}: i64"))
        .collect::<Vec<_>>()
        .join(", ");
    let expr = labels
        .iter()
        .enumerate()
        .map(|(i, label)| {
            let c = 11i64.pow(i as u32);
            if c == 1 {
                label.clone()
            } else {
                format!("{label} * {c}")
            }
        })
        .collect::<Vec<_>>()
        .join(" + ");
    let canonical_args = labels
        .iter()
        .zip(digits.iter())
        .map(|(label, value)| format!("{label}: {value}"))
        .collect::<Vec<_>>()
        .join(", ");
    let perm_args = perm
        .iter()
        .map(|&i| format!("{}: {}", labels[i], digits[i]))
        .collect::<Vec<_>>()
        .join(", ");

    let call_site = if via_function_value {
        format!(
            r#"
    let f = encode
    let permuted = f({perm_args})
"#
        )
    } else {
        format!("    let permuted = encode({perm_args})\n")
    };

    format!(
        r#"
let encode = fn ({params}) -> i64 do
    return {expr}
end

let __test_main = fn () -> bool do
    let canonical = encode({canonical_args})
{call_site}    return canonical == permuted
end
"#
    )
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_polymorphic_id_accepts_i64(n in any::<i64>()) {
        let src = format!(
            r#"
let id = fn <T>(x: T) -> T do
    return x
end

let __test_main = fn () -> i64 do
    return id(x: {n})
end
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_polymorphic_id_rejects_return_mismatch(n in any::<i64>()) {
        let src = format!(
            r#"
let id = fn <T>(x: T) -> T do
    return x
end

let __test_main = fn () -> bool do
    return id(x: {n})
end
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_effectful_call_with_perform_is_ok(calls in 0usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    io(x: 0)\n");
        }
        let src = io_program(&body);
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_pure_call_without_perform_is_ok(calls in 0usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    pure(x: 0)\n");
        }
        let src = pure_program(&body);
        prop_assert!(check(&src).is_ok());
    }


    #[test]
    fn prop_first_combinator_keeps_first_type(n in any::<i64>(), b in any::<bool>()) {
        let src = format!(
            r#"
let first = fn <A, B>(a: A, b: B) -> A do
    return a
end

let __test_main = fn () -> i64 do
    return first(a: {n}, b: {b})
end
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_named_argument_label_mismatch_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
let f = fn (x: i64) -> i64 do
    return x
end

let __test_main = fn () -> i64 do
    return f(y: {n})
end
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_declared_pure_function_cannot_perform_io(calls in 1usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    io(x: 0)\n");
        }
        let src = format!(
            r#"
type IO = {{}}

let io = fn (x: i64) -> unit effect {{ IO }} do
    return ()
end

let pure_wrap = fn (x: i64) -> unit effect {{}} do
{body}
    return ()
end

let main = fn () -> unit do
    pure_wrap(x: 0)
    return ()
end
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_raise_without_exn_effect_is_error(msg in "[a-zA-Z0-9_]{1,16}") {
        let src = format!(
            r#"
exception Fail(val: string)

let fail = fn () -> unit do
    raise Fail(val: [=[{msg}]=])
    return ()
end
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_try_catch_with_io_handler_typechecks(msg in "[a-zA-Z0-9_]{1,16}") {
        let src = format!(
            r#"
import {{ Console }}, * as stdio from nxlib/stdlib/stdio.nx
import {{ from_i64 }} from nxlib/stdlib/string.nx
exception MsgError(val: string)

let risky = fn (msg: string) -> unit effect {{ Exn }} do
    raise MsgError(val: msg)
    return ()
end

let main = fn () -> unit require {{ PermConsole }} do
    inject stdio.system_handler do
        try
            risky(msg: [=[{msg}]=])
        catch e ->
            match e do
                case MsgError(val: m) -> Console.print(val: m)
                case RuntimeError(val: m) -> Console.print(val: m)
                case InvalidIndex(val: i) ->
                    let m = from_i64(val: i)
                    Console.print(val: m)
            end
        end
    end
    return ()
end
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_linear_array_borrow_then_drop_is_ok(xs in proptest::collection::vec(-100i64..=100, 1usize..4)) {
        let elems = xs
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let src = format!(
            r#"
import as array from nxlib/stdlib/array.nx

let __test_main = fn () -> i64 do
    let %arr = [| {elems} |]
    let arr_ref = &%arr
    let n = array.length(arr: arr_ref)
    match %arr do case _ -> () end
    return n
end
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_ref_write_then_read_typechecks(a in any::<i64>(), b in any::<i64>()) {
        let src = format!(
            r#"
let __test_main = fn () -> i64 do
    let ~c = {a}
    ~c <- {b}
    let v = ~c
    return v
end
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_ref_assignment_type_mismatch_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let ~c = {n}
    ~c <- true
    return ()
end
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_immutable_assignment_is_error(a in any::<i64>(), b in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let c = {a}
    c <- {b}
    return ()
end
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_linear_value_must_be_consumed_once(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    match %x do case _ -> () end
    return ()
end
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_linear_primitive_auto_drop_is_ok(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    return ()
end
"#
        );
        // Primitive linear values are auto-dropped at scope end
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_linear_double_consume_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    match %x do case _ -> () end
    match %x do case _ -> () end
    return ()
end
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_linear_cannot_be_stored_in_ref(n in any::<i64>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    let ~r = %x
    return ()
end
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_linear_borrow_then_consume_is_ok(n in any::<i64>()) {
        let src = format!(
            r#"
let peek = fn (x: &i64) -> i64 do
    return x
end

let __test_main = fn () -> i64 do
    let %x = {n}
    let x_ref1 = &%x
    let a = peek(x: x_ref1)
    let x_ref2 = &%x
    let b = peek(x: x_ref2)
    match %x do case _ -> () end
    return a + b
end
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_linear_param_accepts_plain_value_via_weakening(n in any::<i64>()) {
        let src = format!(
            r#"
let consume = fn (x: %i64) -> i64 do
    match x do case _ -> () end
    return 1
end

let main = fn () -> unit do
    let y = consume(x: {n})
    match y do case _ -> () end
    return ()
end
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_linear_branch_mismatch_is_error(n in any::<i64>(), cond in any::<bool>()) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let %x = {n}
    if {cond} then
        match %x do case _ -> () end
    else
        return ()
    end
    return ()
end
"#
        );
        prop_assert!(check(&src).is_err());
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_n_ary_labeled_call_order_invariant_runtime(
        digits in proptest::collection::vec(0i64..=9, 2usize..4),
        seed in any::<u64>()
    ) {
        let perm = permutation_from_seed(digits.len(), seed);
        let src = labeled_call_order_program(&digits, &perm, false);
        let res = run(&src);
        prop_assert!(res.is_ok(), "runtime failed: {:?}\nsource:\n{}", res, src);
        prop_assert_eq!(res.unwrap(), Value::Bool(true));
    }

    #[test]
    fn prop_n_ary_labeled_call_order_invariant_via_function_value_runtime(
        digits in proptest::collection::vec(0i64..=9, 2usize..4),
        seed in any::<u64>()
    ) {
        let perm = permutation_from_seed(digits.len(), seed);
        let src = labeled_call_order_program(&digits, &perm, true);
        let res = run(&src);
        prop_assert!(res.is_ok(), "runtime failed: {:?}\nsource:\n{}", res, src);
        prop_assert_eq!(res.unwrap(), Value::Bool(true));
    }

    #[test]
    fn prop_named_function_value_preserves_identity(n in any::<i64>()) {
        let src = format!(
            r#"
let id = fn (x: i64) -> i64 do
    return x
end

let __test_main = fn () -> i64 do
    let f = id
    return f(x: {n})
end
"#
        );
        let res = run(&src);
        prop_assert!(res.is_ok(), "runtime failed: {:?}", res);
        prop_assert_eq!(res.unwrap(), Value::Int(n));
    }

    #[test]
    fn prop_inline_lambda_increments(n in -1_000_000_000_000i64..1_000_000_000_000i64) {
        let src = format!(
            r#"
let __test_main = fn () -> i64 do
    let inc = fn (x: i64) -> i64 do
        return x + 1
    end
    return inc(x: {n})
end
"#
        );
        let res = run(&src);
        prop_assert!(res.is_ok(), "runtime failed: {:?}", res);
        prop_assert_eq!(res.unwrap(), Value::Int(n + 1));
    }

    #[test]
    fn prop_lambda_captures_outer_variable(
        base in -1_000_000_000i64..1_000_000_000i64,
        delta in -1_000_000_000i64..1_000_000_000i64,
    ) {
        let src = format!(
            r#"
let __test_main = fn () -> i64 do
    let base = {base}
    let add_base = fn (x: i64) -> i64 do
        return x + base
    end
    return add_base(x: {delta})
end
"#
        );
        let res = run(&src);
        prop_assert!(res.is_ok(), "runtime failed: {:?}", res);
        prop_assert_eq!(res.unwrap(), Value::Int(base + delta));
    }

    #[test]
    fn prop_recursive_lambda_factorial(n in 0u32..=20) {
        let expected: i64 = (1..=n as i64).product();
        let src = format!(
            r#"
let __test_main = fn () -> i64 do
    let fact: (n: i64) -> i64 = fn (n: i64) -> i64 do
        if n == 0 then
            return 1
        else
            let n1 = n - 1
            let rec = fact(n: n1)
            return n * rec
        end
    end
    return fact(n: {n})
end
"#
        );
        let res = run(&src);
        prop_assert!(res.is_ok(), "runtime failed: {:?}", res);
        prop_assert_eq!(res.unwrap(), Value::Int(expected));
    }

    #[test]
    fn prop_ref_write_then_read_returns_last_written(a in any::<i64>(), b in any::<i64>()) {
        let src = format!(
            r#"
let __test_main = fn () -> i64 do
    let ~c = {a}
    ~c <- {b}
    let v = ~c
    return v
end
"#
        );
        let res = run(&src);
        prop_assert!(res.is_ok(), "runtime failed: {:?}", res);
        prop_assert_eq!(res.unwrap(), Value::Int(b));
    }

    #[test]
    fn prop_label_punning_syntax_is_rejected(n in any::<i64>()) {
        let src = format!(
            r#"
let id = fn (x: i64) -> i64 do
    return x
end

let __test_main = fn () -> bool do
    let x = {n}
    let _ = id(x)
    return true
end
"#
        );
        let parser = parser::parser();
        prop_assert!(parser.parse(&src).is_err());
    }

    #[test]
    fn prop_positional_call_syntax_rejected(n in any::<i64>()) {
        let src = format!(
            r#"
let f = fn (n: i64) -> unit do
    return ()
end

let main = fn () -> unit do
    f({n})
    return ()
end
"#
        );
        let parser = parser::parser();
        prop_assert!(parser.parse(&src).is_err());
    }
}
