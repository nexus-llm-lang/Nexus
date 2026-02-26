use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;
use std::time::{SystemTime, UNIX_EPOCH};

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

fn unique_temp_dir(label: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic")
        .as_nanos();
    format!("/tmp/nexus_fs_test_{}_{}", label, nanos)
}

struct TempDirGuard {
    path: String,
}

impl TempDirGuard {
    fn new(label: &str) -> Self {
        Self {
            path: unique_temp_dir(label),
        }
    }

    fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn fs_create_dir_and_exists_work() {
    let dir_guard = TempDirGuard::new("mkdir");
    let dir = dir_guard.path();
    let src = format!(
        r#"
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      return Fs.exists(path: [=[{dir}]=])
    catch e ->
      return false
    endtry
  endinject
endfn
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
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx
import {{ string_length, string_contains }} from nxlib/stdlib/string.nx

let main = fn () -> bool do
  inject default_fs do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[hello]=])
      Fs.append_string(path: [=[{file}]=], content: [=[ world]=])
      let content = Fs.read_to_string(path: [=[{file}]=])
      let n = string_length(s: content)
      if n == 11 then
        return string_contains(s: content, sub: [=[world]=])
      else
        return false
      endif
    catch e ->
      return false
    endtry
  endinject
endfn
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
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[x]=])
      Fs.remove_file(path: [=[{file}]=])
      let exists = Fs.exists(path: [=[{file}]=])
      if exists then
        return false
      else
        return true
      endif
    catch e ->
      return false
    endtry
  endinject
endfn
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
import {{ default_fs, Fs, Handle }} from nxlib/stdlib/fs.nx
import {{ string_contains }} from nxlib/stdlib/string.nx

let main = fn () -> bool do
  inject default_fs do
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
              endmatch
            case Nil() -> return false
          endmatch
        case Nil() -> return false
      endmatch
    catch e ->
      return false
    endtry
  endinject
endfn
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
import {{ default_fs, Fs, Handle }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
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
          endmatch
        case Nil() -> return false
      endmatch
    catch e ->
      return false
    endtry
  endinject
endfn
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
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      let entries = Fs.read_dir(path: [=[{dir}]=])
      match entries do
        case Nil() -> return true
        case Cons(v: _, rest: _) -> return false
      endmatch
    catch e ->
      return false
    endtry
  endinject
endfn
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
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> unit do
  inject default_fs do
    let leak = fn () -> unit require {{ Fs }} effect {{ Exn }} do
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[abc]=])
      let %h = Fs.open_read(path: [=[{file}]=])
      return ()
    endfn
    try
      leak()
    catch e ->
      return ()
    endtry
    return ()
  endinject
endfn
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
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> unit do
  inject default_fs do
    let bad = fn () -> unit require {{ Fs }} effect {{ Exn }} do
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[abc]=])
      let %h = Fs.open_read(path: [=[{file}]=])
      Fs.close(handle: %h)
      Fs.close(handle: %h)
      return ()
    endfn
    try
      bad()
    catch e ->
      return ()
    endtry
    return ()
  endinject
endfn
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
import {{ default_fs, Fs, Handle }} from nxlib/stdlib/fs.nx
import {{ string_length }} from nxlib/stdlib/string.nx

let main = fn () -> bool do
  inject default_fs do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[abc]=])
      let %h = Fs.open_read(path: [=[{file}]=])
      let %r = Fs.read(handle: %h)
      match %r do case {{ content: content, handle: %h2 }} ->
        Fs.close(handle: %h2)
        if string_length(s: content) == 3 then
          return true
        else
          return false
        endif
      endmatch
    catch e ->
      return false
    endtry
  endinject
endfn
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
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
    try
      let %h = Fs.open_read(path: [=[{file}]=])
      Fs.close(handle: %h)
      return false
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      endmatch
    endtry
  endinject
endfn
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
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
    try
      let %h = Fs.open_write(path: [=[{file}]=])
      Fs.close(handle: %h)
      return false
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      endmatch
    endtry
  endinject
endfn
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
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
    try
      let %h = Fs.open_append(path: [=[{file}]=])
      Fs.close(handle: %h)
      return false
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      endmatch
    endtry
  endinject
endfn
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
import {{ default_fs, Fs, Handle }} from nxlib/stdlib/fs.nx
import {{ string_length }} from nxlib/stdlib/string.nx

let main = fn () -> bool do
  inject default_fs do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      let %h = Fs.open_write(path: [=[{file}]=])
      let %wr = Fs.fd_write(handle: %h, content: [=[hello controller]=])
      match %wr do case {{ ok: ok, handle: %h2 }} ->
        Fs.close(handle: %h2)
        if ok then
          let content = Fs.read_to_string(path: [=[{file}]=])
          if string_length(s: content) == 16 then
            return true
          else
            return false
          endif
        else
          return false
        endif
      endmatch
    catch e ->
      return false
    endtry
  endinject
endfn
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
import {{ default_fs, Fs, Handle }} from nxlib/stdlib/fs.nx
import {{ string_length, string_contains }} from nxlib/stdlib/string.nx

let main = fn () -> bool do
  inject default_fs do
    try
      Fs.create_dir_all(path: [=[{dir}]=])
      Fs.write_string(path: [=[{file}]=], content: [=[x]=])
      let %h = Fs.open_read(path: [=[{file}]=])
      let %pr = Fs.fd_path(handle: %h)
      match %pr do case {{ path: p, handle: %h2 }} ->
        Fs.close(handle: %h2)
        return string_contains(s: p, sub: [=[x.txt]=])
      endmatch
    catch e ->
      return false
    endtry
  endinject
endfn
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
endfn
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
import {{ default_fs, Fs, Handle }} from nxlib/stdlib/fs.nx
import {{ string_length, string_contains }} from nxlib/stdlib/string.nx

let mock_fs = handler Fs do
  fn exists(path: string) -> bool do return false endfn
  fn read_to_string(path: string) -> string do return [=[]=] endfn
  fn write_string(path: string, content: string) -> unit effect {{ Exn }} do return () endfn
  fn append_string(path: string, content: string) -> unit effect {{ Exn }} do return () endfn
  fn remove_file(path: string) -> unit effect {{ Exn }} do return () endfn
  fn create_dir_all(path: string) -> unit effect {{ Exn }} do return () endfn
  fn read_dir(path: string) -> List<Handle> effect {{ Exn }} do return Nil() endfn
  fn open_read(path: string) -> %Handle effect {{ Exn }} do
    let h = Handle(id: 0)
    let %lh = h
    return %lh
  endfn
  fn open_write(path: string) -> %Handle effect {{ Exn }} do
    let h = Handle(id: 0)
    let %lh = h
    return %lh
  endfn
  fn open_append(path: string) -> %Handle effect {{ Exn }} do
    let h = Handle(id: 0)
    let %lh = h
    return %lh
  endfn
  fn read(handle: %Handle) -> {{ content: string, handle: %Handle }} do
    match handle do case Handle(id: id) ->
      let h = Handle(id: id)
      let %lh = h
      return {{ content: [=[mock content]=], handle: %lh }}
    endmatch
  endfn
  fn fd_write(handle: %Handle, content: string) -> {{ ok: bool, handle: %Handle }} do
    match handle do case Handle(id: id) ->
      let h = Handle(id: id)
      let %lh = h
      return {{ ok: true, handle: %lh }}
    endmatch
  endfn
  fn fd_path(handle: %Handle) -> {{ path: string, handle: %Handle }} do
    match handle do case Handle(id: id) ->
      let h = Handle(id: id)
      let %lh = h
      return {{ path: [=[mock/path.txt]=], handle: %lh }}
    endmatch
  endfn
  fn close(handle: %Handle) -> unit do
    match handle do case Handle(id: _) -> return () endmatch
  endfn
endhandler

let main = fn () -> bool do
  inject mock_fs do
    try
      let %h = Fs.open_read(path: [=[anything]=])
      let %r = Fs.read(handle: %h)
      match %r do case {{ content: c, handle: %h2 }} ->
        Fs.close(handle: %h2)
        return string_contains(s: c, sub: [=[mock]=])
      endmatch
    catch e ->
      return false
    endtry
  endinject
endfn
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_write_string_raises_on_failure() {
    let src = format!(
        r#"
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
    try
      Fs.write_string(path: [=[/nonexistent_dir_xyz/file.txt]=], content: [=[x]=])
      return false
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      endmatch
    endtry
  endinject
endfn
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_read_dir_nonexistent_raises_exn() {
    let src = format!(
        r#"
import {{ default_fs, Fs }} from nxlib/stdlib/fs.nx

let main = fn () -> bool do
  inject default_fs do
    try
      let entries = Fs.read_dir(path: [=[/nonexistent_dir_xyz_abc]=])
      match entries do
        case Nil() -> return false
        case Cons(v: _, rest: _) -> return false
      endmatch
    catch e ->
      match e do
        case RuntimeError(val: _) -> return true
        case InvalidIndex(val: _) -> return false
      endmatch
    endtry
  endinject
endfn
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}
