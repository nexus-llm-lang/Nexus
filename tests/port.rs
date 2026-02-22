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
fn test_port_basic() {
    let src = r#"
    port Logger do
      fn log(msg: string) -> unit effect { IO }
    endport

    handler StdoutLogger for Logger do
      fn log(msg: string) -> unit effect { IO } do
        perform print(val: msg)
        return ()
      endfn
    endhandler

    let main = fn () -> unit effect { IO } do
      perform Logger.log(msg: [=[test message]=])
      return ()
    endfn
    "#;
    let res = run(src);
    assert!(res.is_ok(), "Execution failed: {:?}", res.err());
}

#[test]
fn test_port_redefinition_wins() {
    let src = r#"
    port Adder do
      fn add_one(n: i64) -> i64
    endport

    handler NormalAdder for Adder do
      fn add_one(n: i64) -> i64 do
        return n + 1
      endfn
    endhandler

    handler WeirdAdder for Adder do
      fn add_one(n: i64) -> i64 do
        return n + 2
      endfn
    endhandler

    let main = fn () -> unit effect { IO } do
      let result = Adder.add_one(n: 10)
      let msg = i64_to_string(val: result)
      perform print(val: msg)
      return ()
    endfn
    "#;
    let res = run(src);
    assert!(res.is_ok(), "Execution failed: {:?}", res.err());
}
