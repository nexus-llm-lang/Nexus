use crate::common::check::{should_fail_typecheck, should_typecheck};

#[test]
fn test_ffi_declaration() {
    should_typecheck(
        r#"
    import external math.wasm
    pub external add_i64 = "add" : (a: i64, b: i64) -> i64

    let main = fn () -> unit do
      let x = add_i64(a: 1, b: 2)
      return ()
    end
    "#,
    );
}

#[test]
fn test_ffi_effectful() {
    should_typecheck(
        r#"
    import external time.wasm
    type IO = {}
    pub external get_time = "get_time" : () -> float throws { IO }

    let helper = fn () -> unit throws { IO } do
      let t = get_time()
      return ()
    end

    let main = fn () -> unit do
      return ()
    end
    "#,
    );
}

#[test]
fn test_ffi_mismatch() {
    should_fail_typecheck(
        r#"
    pub external foo = "foo" : (a: i64) -> i64
    let main = fn () -> unit do
      let x = foo(a: true)
    end
    "#,
    );
}

#[test]
fn test_ffi_explicit_type_params() {
    should_typecheck(
        r#"
    import external core.wasm
    pub external array_len = "array_length" : <T>(arr: &[| T |]) -> i64

    let main = fn () -> unit do
      let %a = [| 1, 2, 3 |]
      let r = &%a
      let n = array_len(arr: r)
      match %a do case _ -> () end
      return ()
    end
    "#,
    );
}

#[test]
fn test_ffi_unintroduced_type_var_errors() {
    let err = should_fail_typecheck(
        r#"
    pub external bad = "bad" : (arr: &[| T |]) -> i64
    let main = fn () -> unit do
      return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_ffi_concrete_types_no_type_params_needed() {
    should_typecheck(
        r#"
    pub external add = "add" : (a: i64, b: i64) -> i64
    let main = fn () -> unit do
      let x = add(a: 1, b: 2)
      return ()
    end
    "#,
    );
}
