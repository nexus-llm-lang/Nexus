use crate::harness::exec_with_stdlib;

#[test]
fn backtrace_depth_nonzero_on_raise() {
    exec_with_stdlib(
        r#"
import { backtrace } from "stdlib/exn.nx"

exception Boom(i64)

let inner = fn () -> unit throws { Exn } do
  raise Boom(42)
  return ()
end

let main = fn () -> unit throws { Exn } do
  try
    inner()
  catch e ->
    let frames = backtrace(exn: e)
    match frames do
      | Cons(v: name, rest: _) -> return ()
      | Nil -> raise RuntimeError(val: "expected at least 1 frame")
    end
  end
  return ()
end
"#,
    );
}

#[test]
fn backtrace_cross_function_has_frames() {
    exec_with_stdlib(
        r#"
import { backtrace } from "stdlib/exn.nx"

exception Boom(string)

let deep = fn () -> unit throws { Exn } do
  raise Boom("bang")
  return ()
end

let middle = fn () -> unit throws { Exn } do
  deep()
  return ()
end

let main = fn () -> unit throws { Exn } do
  try
    middle()
  catch e ->
    let frames = backtrace(exn: e)
    // Should have at least deep's frame
    match frames do
      | Cons(v: _, rest: _) -> return ()
      | Nil -> raise RuntimeError(val: "expected frames from cross-function raise")
    end
  end
  return ()
end
"#,
    );
}
