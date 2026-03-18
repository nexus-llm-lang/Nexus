use crate::harness::{should_fail_typecheck, should_typecheck};

#[test]
fn test_array_type_mismatch() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let %arr = [| 1, true |]
        match %arr do case _ -> () end
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_array_indexing_non_array() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let x = 10
        let v = x[0]
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_array_assignment_mismatch() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let %arr = [| 1, 2 |]
        %arr[0] <- true
        match %arr do case _ -> () end
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_array_consume_nonlinear_consumer_is_rejected() {
    let err = should_fail_typecheck(
        r#"
    import * as array from stdlib/array.nx

    let ignore_record = fn (val: { id: i64 }) -> unit do
        return ()
    end

    let main = fn () -> unit do
        let %arr = [| { id: 1 }, { id: 2 } |]
        array.consume(arr: %arr, f: ignore_record)
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_array_consume_with_proper_consumer_passes() {
    should_typecheck(
        r#"
    import * as array from stdlib/array.nx

    let consume_record = fn (%val: { id: i64 }) -> unit do
        match %val do case { id: _ } -> () end
        return ()
    end

    let main = fn () -> unit do
        let %arr = [| { id: 1 }, { id: 2 } |]
        array.consume(arr: %arr, f: consume_record)
        return ()
    end
    "#,
    );
}
