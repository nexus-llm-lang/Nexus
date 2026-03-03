mod common;

use common::source::run;
use nexus::interpreter::Value;

#[test]
fn result_from_exn_builds_err() {
    let src = r#"
import as result from nxlib/stdlib/result.nx

let main = fn () -> bool do
  let exn = RuntimeError(val: [=[boom]=])
  let r = result.from_exn(exn: exn)
  return result.is_err(res: r)
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn result_to_exn_raises_and_is_catchable() {
    let src = r#"
import as result from nxlib/stdlib/result.nx

let main = fn () -> bool do
  let err: Result<i64, Exn> = Err(err: RuntimeError(val: [=[boom]=]))
  try
    let _ = result.to_exn(res: err)
    return false
  catch e ->
    return true
  end
end
"#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}
