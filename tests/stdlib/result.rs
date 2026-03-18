use crate::harness::exec_with_stdlib;

#[test]
fn result_from_exn_builds_err() {
    exec_with_stdlib(
        r#"
import * as result from stdlib/result.nx

let main = fn () -> unit do
  let exn = RuntimeError(val: "boom")
  let r = result.from_exn(exn: exn)
  let ok = result.is_err(res: r)
  if ok != true then raise RuntimeError(val: "expected is_err to be true") end
  return ()
end
"#,
    );
}
