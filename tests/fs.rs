mod common;

use common::source::{check, run, TempDirGuard};
use nexus::interpreter::Value;
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

#[test]
fn fs_create_dir_and_exists_work() {
    let dir_guard = TempDirGuard::new("mkdir");
    let dir = dir_guard.path();
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      return Fs.exists(path: [=[{dir}]=])
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_append_and_read_roundtrip() {
    let dir_guard = TempDirGuard::new("append");
    let dir = dir_guard.path();
    let file = format!("{}/note.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx
import {{ length, contains }} from nxlib/stdlib/string.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[hello]=])
      Fs.append_string(path: [=[{file}]=], content: [=[ world]=])
      let content = Fs.read_to_string(path: [=[{file}]=])
      let n = length(s: content)
      if n == 11 then
        return contains(s: content, sub: [=[world]=])
      else
        return false
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_remove_file_updates_exists() {
    let dir_guard = TempDirGuard::new("remove");
    let dir = dir_guard.path();
    let file = format!("{}/trash.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[x]=])
      Fs.remove_file(path: [=[{file}]=])
      let exists = Fs.exists(path: [=[{file}]=])
      if exists then
        return false
      else
        return true
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_read_dir_returns_handles() {
    let dir_guard = TempDirGuard::new("readdir");
    let dir = dir_guard.path();
    let file_a = format!("{}/a.txt", dir);
    let file_b = format!("{}/b.txt", dir);
    let src = format!(
        r#"
import {{ Fs, Handle }}, * as fs_mod from nxlib/stdlib/fs.nx
import {{ length, contains }} from nxlib/stdlib/string.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file_a}]=], content: [=[aaa]=])
      Fs.write_string(path: [=[{file_b}]=], content: [=[bbb]=])
      let entries = Fs.read_dir(path: [=[{dir}]=])
      match entries do
        case Cons(v: h1, rest: rest1) ->
          match rest1 do
            case Cons(v: h2, rest: rest2) ->
              match rest2 do
                case Nil() ->
                  Fs.close(handle: h1)
                  Fs.close(handle: h2)
                  return true
                case Cons(v: _, rest: _) -> return false
              end
            case Nil() -> return false
          end
        case Nil() -> return false
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_read_dir_skips_subdirectories() {
    let dir_guard = TempDirGuard::new("readdir_subdir");
    let dir = dir_guard.path();
    let file = format!("{}/a.txt", dir);
    let sub = format!("{}/sub", dir);
    let src = format!(
        r#"
import {{ Fs, Handle }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[x]=])
      Fs.create_dir_all(path: [=[{sub}]=])
      let entries = Fs.read_dir(path: [=[{dir}]=])
      match entries do
        case Cons(v: h1, rest: rest1) ->
          match rest1 do
            case Nil() ->
              Fs.close(handle: h1)
              return true
            case Cons(v: _, rest: _) -> return false
          end
        case Nil() -> return false
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_read_dir_empty_returns_nil() {
    let dir_guard = TempDirGuard::new("readdir_empty");
    let dir = dir_guard.path();
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      let entries = Fs.read_dir(path: [=[{dir}]=])
      match entries do
        case Nil() -> return true
        case Cons(v: _, rest: _) -> return false
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_linear_file_requires_close() {
    let dir_guard = TempDirGuard::new("linear_requires_close");
    let dir = dir_guard.path();
    let file = format!("{}/x.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> unit require {{ PermFs }} do
  inject fs_mod.system_handler do
    let leak = fn () -> unit require {{ Fs }} effect {{ Exn }} do
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[abc]=])
      let %h = Fs.open_read(path: [=[{file}]=])
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
    let program = parser().parse(src.as_str()).expect("program should parse");
    let mut checker = TypeChecker::new();
    let err = checker
        .check_program(&program)
        .expect_err("missing close should be a type error");
    assert!(
        err.message.contains("Unused linear"),
        "expected unused linear error, got: {}",
        err.message
    );
}

#[test]
fn fs_linear_file_double_close_is_rejected() {
    let dir_guard = TempDirGuard::new("linear_double_close");
    let dir = dir_guard.path();
    let file = format!("{}/x.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> unit require {{ PermFs }} do
  inject fs_mod.system_handler do
    let bad = fn () -> unit require {{ Fs }} effect {{ Exn }} do
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[abc]=])
      let %h = Fs.open_read(path: [=[{file}]=])
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
    let program = parser().parse(src.as_str()).expect("program should parse");
    let mut checker = TypeChecker::new();
    let err = checker
        .check_program(&program)
        .expect_err("double close should be a type error");
    assert!(
        err.message.contains("already consumed"),
        "expected consumed error, got: {}",
        err.message
    );
}

#[test]
fn fs_linear_file_read_then_close_works() {
    let dir_guard = TempDirGuard::new("linear_read_close");
    let dir = dir_guard.path();
    let file = format!("{}/x.txt", dir);
    let src = format!(
        r#"
import {{ Fs, Handle }}, * as fs_mod from nxlib/stdlib/fs.nx
import {{ length, contains }} from nxlib/stdlib/string.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[abc]=])
      let %h = Fs.open_read(path: [=[{file}]=])
      let %r = Fs.read(handle: %h)
      match %r do case {{ content: content, handle: %h2 }} ->
        Fs.close(handle: %h2)
        if length(s: content) == 3 then
          return true
        else
          return false
        end
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_open_read_missing_file_raises_exn() {
    let dir_guard = TempDirGuard::new("linear_missing");
    let dir = dir_guard.path();
    let file = format!("{}/missing.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      let %h = Fs.open_read(path: [=[{file}]=])
      Fs.close(handle: %h)
      return false
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      end
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_open_write_missing_parent_raises_exn() {
    let dir_guard = TempDirGuard::new("linear_missing_write");
    let dir = dir_guard.path();
    let file = format!("{}/no_parent/x.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      let %h = Fs.open_write(path: [=[{file}]=])
      Fs.close(handle: %h)
      return false
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      end
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_open_append_missing_parent_raises_exn() {
    let dir_guard = TempDirGuard::new("linear_missing_append");
    let dir = dir_guard.path();
    let file = format!("{}/no_parent/x.txt", dir);
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      let %h = Fs.open_append(path: [=[{file}]=])
      Fs.close(handle: %h)
      return false
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      end
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_write_through_controller() {
    let dir_guard = TempDirGuard::new("write_controller");
    let dir = dir_guard.path();
    let file = format!("{}/out.txt", dir);
    let src = format!(
        r#"
import {{ Fs, Handle }}, * as fs_mod from nxlib/stdlib/fs.nx
import {{ length, contains }} from nxlib/stdlib/string.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      let %h = Fs.open_write(path: [=[{file}]=])
      let %wr = Fs.fd_write(handle: %h, content: [=[hello controller]=])
      match %wr do case {{ ok: ok, handle: %h2 }} ->
        Fs.close(handle: %h2)
        if ok then
          let content = Fs.read_to_string(path: [=[{file}]=])
          if length(s: content) == 16 then
            return true
          else
            return false
          end
        else
          return false
        end
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_path_returns_path() {
    let dir_guard = TempDirGuard::new("path_returns");
    let dir = dir_guard.path();
    let file = format!("{}/x.txt", dir);
    let src = format!(
        r#"
import {{ Fs, Handle }}, * as fs_mod from nxlib/stdlib/fs.nx
import {{ length, contains }} from nxlib/stdlib/string.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[x]=])
      let %h = Fs.open_read(path: [=[{file}]=])
      let %pr = Fs.fd_path(handle: %h)
      match %pr do case {{ path: p, handle: %h2 }} ->
        Fs.close(handle: %h2)
        return contains(s: p, sub: [=[x.txt]=])
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_read_requires_fs_coeffect() {
    let src = r#"
import { Fs } from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  let ok = Fs.exists(path: [=[/tmp]=])
  return ok
end
"#;
    let err = check(src).expect_err("Fs.exists without inject Fs should be a type error");
    assert!(
        err.contains("requires") || err.contains("Fs"),
        "expected coeffect error, got: {}",
        err
    );
}

#[test]
fn fs_mock_handler_replaces_read() {
    let src = format!(
        r#"
import {{ Fs, Handle }}, * as fs_mod from nxlib/stdlib/fs.nx
import {{ length, contains }} from nxlib/stdlib/string.nx

let mock_fs = handler Fs do
  fn exists(path: string) -> bool do return false end
  fn read_to_string(path: string) -> string do return [=[]=] end
  fn write_string(path: string, content: string) -> unit effect {{ Exn }} do return () end
  fn append_string(path: string, content: string) -> unit effect {{ Exn }} do return () end
  fn remove_file(path: string) -> unit effect {{ Exn }} do return () end
  fn create_dir_all(path: string) -> unit effect {{ Exn }} do return () end
  fn read_dir(path: string) -> List<Handle> effect {{ Exn }} do return Nil() end
  fn open_read(path: string) -> %Handle effect {{ Exn }} do
    let h = Handle(id: 0)
    let %lh = h
    return %lh
  end
  fn open_write(path: string) -> %Handle effect {{ Exn }} do
    let h = Handle(id: 0)
    let %lh = h
    return %lh
  end
  fn open_append(path: string) -> %Handle effect {{ Exn }} do
    let h = Handle(id: 0)
    let %lh = h
    return %lh
  end
  fn read(handle: %Handle) -> {{ content: string, handle: %Handle }} do
    match handle do case Handle(id: id) ->
      let h = Handle(id: id)
      let %lh = h
      return {{ content: [=[mock content]=], handle: %lh }}
    end
  end
  fn fd_write(handle: %Handle, content: string) -> {{ ok: bool, handle: %Handle }} do
    match handle do case Handle(id: id) ->
      let h = Handle(id: id)
      let %lh = h
      return {{ ok: true, handle: %lh }}
    end
  end
  fn fd_path(handle: %Handle) -> {{ path: string, handle: %Handle }} do
    match handle do case Handle(id: id) ->
      let h = Handle(id: id)
      let %lh = h
      return {{ path: [=[mock/path.txt]=], handle: %lh }}
    end
  end
  fn close(handle: %Handle) -> unit do
    match handle do case Handle(id: _) -> return () end
  end
end

let main = fn () -> bool do
  inject mock_fs do
    try
      let %h = Fs.open_read(path: [=[anything]=])
      let %r = Fs.read(handle: %h)
      match %r do case {{ content: c, handle: %h2 }} ->
        Fs.close(handle: %h2)
        return contains(s: c, sub: [=[mock]=])
      end
    catch e ->
      return false
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_write_string_raises_on_failure() {
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      Fs.write_string(path: [=[/nonexistent_dir_xyz/file.txt]=], content: [=[x]=])
      return false
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      end
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_read_dir_nonexistent_raises_exn() {
    let src = format!(
        r#"
import {{ Fs }}, * as fs_mod from nxlib/stdlib/fs.nx

let main = fn () -> bool require {{ PermFs }} do
  inject fs_mod.system_handler do
    try
      let entries = Fs.read_dir(path: [=[/nonexistent_dir_xyz_abc]=])
      match entries do
        case Nil() -> return false
        case Cons(v: _, rest: _) -> return false
      end
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      end
    end
  end
end
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}
