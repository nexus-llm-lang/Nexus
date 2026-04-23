use crate::harness::{exec_with_stdlib, should_fail_typecheck, TempDir};

#[test]
fn fs_create_dir_and_exists_work() {
    let tmp = TempDir::new("mkdir");
    let dir = tmp.path();
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from "stdlib/filesystem.nx"

let main = fn () -> unit require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: "{dir}")
      let ok = Fs.exists(path: "{dir}")
      if ok != true then raise RuntimeError(val: "expected exists true") end
      return ()
    catch e ->
      raise RuntimeError(val: "unexpected exception")
    end
  end
end
"#
    );
    exec_with_stdlib(&src);
}

#[test]
fn fs_append_and_read_roundtrip() {
    let tmp = TempDir::new("append");
    let dir = tmp.path();
    let file = format!("{}/note.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from "stdlib/filesystem.nx"
import {{ length, contains }} from "stdlib/string_ops.nx"

let main = fn () -> unit require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: "{dir}")
      Fs.write_string(path: "{file}", content: "hello")
      Fs.append_string(path: "{file}", content: " world")
      let content = Fs.read_to_string(path: "{file}")
      let n = length(s: content)
      if n != 11 then raise RuntimeError(val: "expected length 11") end
      let ok = contains(s: content, sub: "world")
      if ok != true then raise RuntimeError(val: "expected contains world") end
      return ()
    catch e ->
      raise RuntimeError(val: "unexpected exception")
    end
  end
end
"#
    );
    exec_with_stdlib(&src);
}

#[test]
fn fs_remove_file_updates_exists() {
    let tmp = TempDir::new("remove");
    let dir = tmp.path();
    let file = format!("{}/trash.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from "stdlib/filesystem.nx"

let main = fn () -> unit require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: "{dir}")
      Fs.write_string(path: "{file}", content: "x")
      Fs.remove_file(path: "{file}")
      let exists = Fs.exists(path: "{file}")
      if exists then raise RuntimeError(val: "file should not exist") end
      return ()
    catch e ->
      raise RuntimeError(val: "unexpected exception")
    end
  end
end
"#
    );
    exec_with_stdlib(&src);
}

#[test]
fn fs_linear_file_requires_close() {
    let tmp = TempDir::new("linear_requires_close");
    let dir = tmp.path();
    let file = format!("{}/x.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from "stdlib/filesystem.nx"

let main = fn () -> unit require {{ PermFs }} do
  inject fs_mod.system_handler do
    let leak = fn () -> unit require {{ Fs }} throws {{ Exn }} do
      Fs.create_dir_all(path: "{dir}")
      Fs.write_string(path: "{file}", content: "abc")
      let %h = Fs.open_read(path: "{file}")
      return ()
    end
    try
      leak()
    catch e ->
      return ()
    end
    return ()
  end
end
"#
    );
    let err = should_fail_typecheck(&src);
    insta::assert_snapshot!(err);
}

#[test]
fn fs_linear_file_double_close_is_rejected() {
    let tmp = TempDir::new("linear_double_close");
    let dir = tmp.path();
    let file = format!("{}/x.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from "stdlib/filesystem.nx"

let main = fn () -> unit require {{ PermFs }} do
  inject fs_mod.system_handler do
    let bad = fn () -> unit require {{ Fs }} throws {{ Exn }} do
      Fs.create_dir_all(path: "{dir}")
      Fs.write_string(path: "{file}", content: "abc")
      let %h = Fs.open_read(path: "{file}")
      Fs.close(handle: %h)
      Fs.close(handle: %h)
      return ()
    end
    try
      bad()
    catch e ->
      return ()
    end
    return ()
  end
end
"#
    );
    let err = should_fail_typecheck(&src);
    insta::assert_snapshot!(err);
}

#[test]
fn fs_read_requires_fs_coeffect() {
    let err = should_fail_typecheck(
        r#"
import { Fs } from "stdlib/filesystem.nx"

let main = fn () -> bool do
  let ok = Fs.exists(path: "/tmp")
  return ok
end
"#,
    );
    insta::assert_snapshot!(err);
}
