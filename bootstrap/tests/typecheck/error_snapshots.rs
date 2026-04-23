/// Systematic error message snapshot tests.
///
/// Every major error category the typechecker can produce should have a
/// snapshot so that regressions in diagnostic quality are caught automatically.
/// The snapshot captures the exact error message text — any change forces
/// explicit review via `cargo insta review`.
use crate::harness::should_fail_typecheck;

// ---------------------------------------------------------------------------
// Type mismatch errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_type_mismatch_return() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> i64 do
        return true
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_type_mismatch_let_annotation() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x: i64 = "hello"
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_type_mismatch_function_arg() {
    let err = should_fail_typecheck(
        r#"
    let f = fn (x: i64) -> i64 do return x end

    let main = fn () -> unit do
        let _ = f(x: true)
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_type_mismatch_if_branches() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x = if true then 1 else "two" end
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_type_mismatch_match_arms() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x: i64 = match true do
            case true -> 1
            case false -> "zero"
        end
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Undefined variable / function errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_undefined_variable() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x = unknown_var
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_undefined_function() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x = nonexistent_fn(a: 1)
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_undefined_type() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x: Nonexistent = 42
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Arity errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_too_few_args() {
    let err = should_fail_typecheck(
        r#"
    let add = fn (a: i64, b: i64) -> i64 do return a + b end

    let main = fn () -> unit do
        let _ = add(a: 1)
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_too_many_args() {
    let err = should_fail_typecheck(
        r#"
    let inc = fn (x: i64) -> i64 do return x + 1 end

    let main = fn () -> unit do
        let _ = inc(x: 1, y: 2)
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_wrong_arg_label() {
    let err = should_fail_typecheck(
        r#"
    let f = fn (name: string) -> string do return name end

    let main = fn () -> unit do
        let _ = f(label: "hello")
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Linear type errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_linear_unconsumed() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let %x = { id: 1 }
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_linear_double_consume() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let %x = { id: 1 }
        match %x do case _ -> () end
        match %x do case _ -> () end
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_linear_branch_mismatch() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let %x = { id: 1 }
        if true then
            match %x do case _ -> () end
        else
            ()
        end
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Effect system errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_effect_leak_pure_calls_impure() {
    let err = should_fail_typecheck(
        r#"
    type IO = {}

    let impure = fn () -> unit throws { IO } do return () end

    let pure_fn = fn () -> unit throws {} do
        impure()
        return ()
    end

    let main = fn () -> unit do return () end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_raise_without_throws() {
    let err = should_fail_typecheck(
        r#"
    exception Boom(code: i64)

    let f = fn () -> unit do
        raise Boom(code: 42)
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Exhaustiveness errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_match_non_exhaustive_option() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x: Option<i64> = Some(val: 1)
        match x do
            case Some(val: v) -> return ()
        end
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_match_non_exhaustive_bool() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let b = true
        match b do
            case true -> return ()
        end
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_match_non_exhaustive_enum() {
    let err = should_fail_typecheck(
        r#"
    type Color = Red | Green | Blue

    let main = fn () -> unit do
        let c: Color = Red
        match c do
            case Red -> return ()
            case Green -> return ()
        end
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Ref / mutability errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_assign_to_immutable() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x = 10
        x <- 20
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_ref_type_mismatch_on_assign() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let ~x = 10
        ~x <- true
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Permission / capability errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_missing_permission() {
    let err = should_fail_typecheck(
        r#"
    import { Fs }, * as fs_mod from "stdlib/filesystem.nx"

    let main = fn () -> unit do
        inject fs_mod.system_handler do
            let _ = Fs.exists(path: "/tmp")
        end
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Constructor errors
// ---------------------------------------------------------------------------

#[test]
fn snapshot_unknown_constructor() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x = Bogus(value: 1)
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn snapshot_constructor_wrong_field_count() {
    let err = should_fail_typecheck(
        r#"
    type Pair = Pair(a: i64, b: i64)

    let main = fn () -> unit do
        let p = Pair(a: 1)
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

// ---------------------------------------------------------------------------
// Implicit return soundness
// ---------------------------------------------------------------------------

#[test]
fn snapshot_non_unit_function_without_return() {
    let err = should_fail_typecheck(
        r#"
    let f = fn () -> i64 do
        let x = 42
    end
    "#,
    );
    insta::assert_snapshot!(err);
}
