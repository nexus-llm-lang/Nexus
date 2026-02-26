use chumsky::Parser;
use nexus::lang::parser;
use nexus::lang::typecheck::TypeChecker;

fn check_code(src: &str) -> Result<(), String> {
    let parser = parser::parser();
    let program = parser.parse(src).map_err(|e| format!("{:?}", e))?;

    let mut checker = TypeChecker::new();
    checker.check_program(&program).map_err(|e| e.message)
}

#[test]
fn test_ffi_declaration() {
    let src = r#"
    import external math.wasm
    pub external add_i64 = [=[add]=] : (a: i64, b: i64) -> i64

    let main = fn () -> unit effect { Console } do
      let x = add_i64(a: 1, b: 2)
      // print(val: i64_to_string(val: x))
      return ()
    endfn
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
    pub external get_time = [=[get_time]=] : () -> float effect { Console }

    let main = fn () -> unit effect { Console } do
      let t = get_time()
      // print(val: float_to_string(val: t))
      return ()
    endfn
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
    endfn
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
      match %a do case _ -> () endmatch
      return ()
    endfn
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
    endfn
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
    endfn
    "#;
    match check_code(src) {
        Ok(_) => (),
        Err(e) => panic!("Type check failed: {}", e),
    }
}
