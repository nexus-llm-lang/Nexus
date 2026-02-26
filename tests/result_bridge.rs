use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

fn run(src: &str) -> Result<Value, String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)?;
    let mut interpreter = Interpreter::new(p);
    interpreter.run_function("main", vec![])
}

#[test]
fn result_from_exn_builds_err() {
    let src = r#"
import as result from nxlib/stdlib/result.nx

let main = fn () -> bool do
  let exn = RuntimeError(val: [=[boom]=])
  let r = result.from_exn(exn: exn)
  return result.is_err(res: r)
endfn
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn result_to_exn_raises_and_is_catchable() {
    let src = r#"
import as result from nxlib/stdlib/result.nx

let main = fn () -> bool effect { Console } do
  let err: Result<i64, Exn> = Err(err: RuntimeError(val: [=[boom]=]))
  try
    let _ = result.to_exn(res: err)
    return false
  catch e ->
    return true
  endtry
endfn
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
