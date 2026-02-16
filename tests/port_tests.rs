use nexus::interpreter::{Interpreter, Value};
use nexus::typecheck::TypeChecker;
use nexus::parser::parser;
use chumsky::Parser;

fn run(src: &str) -> Result<Value, String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p)?;
    let mut interpreter = Interpreter::new(p);
    interpreter.run_function("main", vec![])
}

#[test]
fn test_port_basic() {
    let src = r#"
    port Logger do
      fn log(msg: str) -> unit
    endport

    handler StdoutLogger for Logger do
      fn log(msg: str) -> unit do
        perform print_str(val: msg)
        return ()
      endfn
    endhandler

    fn main() -> unit do
      perform Logger.log(msg: "test message")
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

    fn main() -> unit do
      let result = Adder.add_one(n: 10)
      perform print_i64(val: result)
      return ()
    endfn
    "#;
    let res = run(src);
    assert!(res.is_ok());
}
