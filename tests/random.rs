use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)
}

fn run(src: &str) -> Result<Value, String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)?;
    let mut interpreter = Interpreter::new(p);
    interpreter.run_function("main", vec![])
}

#[test]
fn random_range_returns_in_bounds_value() {
    let src = r#"
import as random from [=[nxlib/stdlib/random.nx]=]

let main = fn () -> bool effect { IO } do
  let n = perform random.range(min: 10, max: 20)
  if n >= 10 then
    return n < 20
  else
    return false
  endif
endfn
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn random_range_requires_perform() {
    let src = r#"
import as random from [=[nxlib/stdlib/random.nx]=]

let main = fn () -> i64 do
  let n = random.range(min: 0, max: 10)
  return n
endfn
"#;
    assert!(
        check(src).is_err(),
        "random.range without perform should fail typechecking"
    );
}
