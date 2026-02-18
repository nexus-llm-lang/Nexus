use chumsky::Parser;
use nexus::parser::parser;
use nexus::typecheck::TypeChecker;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)
}

#[test]
fn test_array_basic() {
    let src = r#"
    fn main() -> unit effect { IO } do
        let %arr = [| 1, 2, 3 |]
        %arr[0] <- 42
        let val = (borrow %arr)[0]
        let msg = i64_to_string(val: val)
        perform print(val: msg)
        drop %arr
        return ()
    endfn
    "#;
    if let Err(e) = check(src) {
        panic!("Typecheck failed: {}", e);
    }
}

#[test]
fn test_array_type_mismatch() {
    let src = r#"
    fn main() -> unit do
        let %arr = [| 1, true |]
        drop %arr
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_array_indexing_non_array() {
    let src = r#"
    fn main() -> unit do
        let x = 10
        let v = x[0]
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_array_assignment_mismatch() {
    let src = r#"
    fn main() -> unit do
        let %arr = [| 1, 2 |]
        %arr[0] <- true // Should fail: assigning bool to i64 array
        drop %arr
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}
