use chumsky::Parser;
use nexus::parser;
use nexus::typecheck::TypeChecker;
use proptest::prelude::*;

fn check(src: &str) -> Result<(), String> {
    let program = parser::parser()
        .parse(src)
        .map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&program).map_err(|e| e.message)
}

fn io_program(body: &str) -> String {
    format!(
        r#"
fn io(x: i64) -> unit effect {{ IO }} do
    return ()
endfn

fn main() -> unit effect {{ IO }} do
{body}
    return ()
endfn
"#
    )
}

fn pure_program(body: &str) -> String {
    format!(
        r#"
fn pure(x: i64) -> unit do
    return ()
endfn

fn main() -> unit do
{body}
    return ()
endfn
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
fn id<T>(x: T) -> T do
    return x
endfn

fn main() -> i64 do
    return id(x: {n})
endfn
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_polymorphic_id_rejects_return_mismatch(n in any::<i64>()) {
        let src = format!(
            r#"
fn id<T>(x: T) -> T do
    return x
endfn

fn main() -> bool do
    return id(x: {n})
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_effectful_call_with_perform_is_ok(calls in 0usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    perform io(x: 0)\n");
        }
        let src = io_program(&body);
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_effectful_call_without_perform_is_error(calls in 1usize..8, missing_index in 0usize..16) {
        let missing = missing_index % calls;
        let mut body = String::new();
        for i in 0..calls {
            if i == missing {
                body.push_str("    io(x: 0)\n");
            } else {
                body.push_str("    perform io(x: 0)\n");
            }
        }
        let src = io_program(&body);
        prop_assert!(check(&src).is_err());
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
    fn prop_pure_call_with_perform_is_error(calls in 1usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    perform pure(x: 0)\n");
        }
        let src = pure_program(&body);
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_first_combinator_keeps_first_type(n in any::<i64>(), b in any::<bool>()) {
        let src = format!(
            r#"
fn first<A, B>(a: A, b: B) -> A do
    return a
endfn

fn main() -> i64 do
    return first(a: {n}, b: {b})
endfn
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_named_argument_label_mismatch_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
fn f(x: i64) -> i64 do
    return x
endfn

fn main() -> i64 do
    return f(y: {n})
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_declared_pure_function_cannot_perform_io(calls in 1usize..8) {
        let mut body = String::new();
        for _ in 0..calls {
            body.push_str("    perform io(x: 0)\n");
        }
        let src = format!(
            r#"
fn io(x: i64) -> unit effect {{ IO }} do
    return ()
endfn

fn pure_wrap(x: i64) -> unit effect {{}} do
{body}
    return ()
endfn

fn main() -> unit do
    pure_wrap(x: 0)
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_raise_without_exn_effect_is_error(msg in "[a-zA-Z0-9_]{1,16}") {
        let src = format!(
            r#"
fn fail() -> unit do
    raise [=[{msg}]=]
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_try_catch_with_io_handler_typechecks(msg in "[a-zA-Z0-9_]{1,16}") {
        let src = format!(
            r#"
fn risky(msg: string) -> unit effect {{ Exn }} do
    raise msg
    return ()
endfn

fn main() -> unit effect {{ IO }} do
    try
        perform risky(msg: [=[{msg}]=])
    catch e ->
        perform print(val: e)
    endtry
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_linear_array_borrow_then_drop_is_ok(xs in proptest::collection::vec(-100i64..=100, 1usize..8)) {
        let elems = xs
            .iter()
            .map(|x| x.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let src = format!(
            r#"
import as array from "nxlib/stdlib/array.nx"

fn main() -> i64 do
    let %arr = [| {elems} |]
    let n = array.length(arr: borrow %arr)
    drop_array(arr: %arr)
    return n
endfn
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_ref_write_then_read_typechecks(a in any::<i64>(), b in any::<i64>()) {
        let src = format!(
            r#"
fn main() -> i64 do
    let ~c = {a}
    ~c <- {b}
    let v = ~c
    return v
endfn
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_ref_assignment_type_mismatch_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
fn main() -> unit do
    let ~c = {n}
    ~c <- true
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_immutable_assignment_is_error(a in any::<i64>(), b in any::<i64>()) {
        let src = format!(
            r#"
fn main() -> unit do
    let c = {a}
    c <- {b}
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_linear_value_must_be_consumed_once(n in any::<i64>()) {
        let src = format!(
            r#"
fn main() -> unit do
    let %x = {n}
    drop_i64(val: %x)
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_linear_unused_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
fn main() -> unit do
    let %x = {n}
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_linear_double_consume_is_error(n in any::<i64>()) {
        let src = format!(
            r#"
fn main() -> unit do
    let %x = {n}
    drop_i64(val: %x)
    drop_i64(val: %x)
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_linear_cannot_be_stored_in_ref(n in any::<i64>()) {
        let src = format!(
            r#"
fn main() -> unit do
    let %x = {n}
    let ~r = %x
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }

    #[test]
    fn prop_linear_borrow_then_consume_is_ok(n in any::<i64>()) {
        let src = format!(
            r#"
fn peek(x: &i64) -> i64 do
    return x
endfn

fn main() -> i64 do
    let %x = {n}
    let a = peek(x: borrow %x)
    let b = peek(x: borrow %x)
    drop_i64(val: %x)
    return a + b
endfn
"#
        );
        prop_assert!(check(&src).is_ok());
    }

    #[test]
    fn prop_linear_branch_mismatch_is_error(n in any::<i64>(), cond in any::<bool>()) {
        let src = format!(
            r#"
fn main() -> unit do
    let %x = {n}
    if {cond} then
        drop_i64(val: %x)
    else
        return ()
    endif
    return ()
endfn
"#
        );
        prop_assert!(check(&src).is_err());
    }
}
