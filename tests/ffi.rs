mod common;

use common::source::check_raw as check_code;

#[test]
fn test_ffi_declaration() {
    let src = r#"
    import external math.wasm
    pub external add_i64 = [=[add]=] : (a: i64, b: i64) -> i64

    let main = fn () -> unit do
      let x = add_i64(a: 1, b: 2)
      return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ffi_effectful() {
    let src = r#"
    import external time.wasm
    type IO = {}
    pub external get_time = [=[get_time]=] : () -> float effect { IO }

    let helper = fn () -> unit effect { IO } do
      let t = get_time()
      return ()
    end

    let main = fn () -> unit do
      return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ffi_mismatch() {
    let src = r#"
    pub external foo = [=[foo]=] : (a: i64) -> i64
    let main = fn () -> unit do
      let x = foo(a: true)
    end
    "#;
    assert!(check_code(src).is_err());
}

#[test]
fn test_ffi_explicit_type_params() {
    let src = r#"
    import external core.wasm
    pub external array_len = [=[array_length]=] : <T>(arr: &[| T |]) -> i64

    let main = fn () -> unit do
      let %a = [| 1, 2, 3 |]
      let r = &%a
      let n = array_len(arr: r)
      match %a do case _ -> () end
      return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}

#[test]
fn test_ffi_unintroduced_type_var_errors() {
    let src = r#"
    pub external bad = [=[bad]=] : (arr: &[| T |]) -> i64
    let main = fn () -> unit do
      return ()
    end
    "#;
    let err = check_code(src).unwrap_err();
    assert!(
        err.contains("unintroduced type variable"),
        "expected unintroduced type var error, got: {}",
        err
    );
}

#[test]
fn test_ffi_concrete_types_no_type_params_needed() {
    let src = r#"
    pub external add = [=[add]=] : (a: i64, b: i64) -> i64
    let main = fn () -> unit do
      let x = add(a: 1, b: 2)
      return ()
    end
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}
