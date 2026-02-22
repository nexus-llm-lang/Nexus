use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;
use std::time::{SystemTime, UNIX_EPOCH};

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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> bool effect {{ IO }} do
  let ok = perform fs.create_dir_all(path: [=[{dir}]=])
  if ok then
    return perform fs.exists(path: [=[{dir}]=])
  else
    return false
  endif
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> bool effect {{ IO }} do
  let _ = perform fs.create_dir_all(path: [=[{dir}]=])
  let w = perform fs.write_string(path: [=[{file}]=], content: [=[hello]=])
  if w then
    let a = perform fs.append_string(path: [=[{file}]=], content: [=[ world]=])
    if a then
      let content = perform fs.read_to_string(path: [=[{file}]=])
      let n = string_length(s: content)
      if n == 11 then
        return string_contains(s: content, sub: [=[world]=])
      else
        return false
      endif
    else
      return false
    endif
  else
    return false
  endif
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> bool effect {{ IO }} do
  let _ = perform fs.create_dir_all(path: [=[{dir}]=])
  let w = perform fs.write_string(path: [=[{file}]=], content: [=[x]=])
  if w then
    let removed = perform fs.remove_file(path: [=[{file}]=])
    if removed then
      let exists = perform fs.exists(path: [=[{file}]=])
      if exists then
        return false
      else
        return true
      endif
    else
      return false
    endif
  else
    return false
  endif
endfn
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}

#[test]
fn fs_read_dir_lists_files_and_dirs() {
    let dir_guard = TempDirGuard::new("readdir");
    let dir = dir_guard.path();
    let file = format!("{}/a.txt", dir);
    let sub = format!("{}/sub", dir);
    let src = format!(
        r#"
import as fs from [=[nxlib/stdlib/fs.nx]=]

let string_eq = fn (a: string, b: string) -> bool do
  let la = string_length(s: a)
  let lb = string_length(s: b)
  if la == lb then
    return string_contains(s: a, sub: b)
  else
    return false
  endif
endfn

let contains = fn (xs: List<string>, target: string) -> bool do
  match xs do
    case Nil() -> return false
    case Cons(v: v, rest: rest) ->
      if string_eq(a: v, b: target) then
        return true
      else
        return contains(xs: rest, target: target)
      endif
  endmatch
endfn

let main = fn () -> bool effect {{ IO }} do
  let _ = perform fs.create_dir_all(path: [=[{dir}]=])
  let w = perform fs.write_string(path: [=[{file}]=], content: [=[x]=])
  let d = perform fs.create_dir_all(path: [=[{sub}]=])
  if w then
    if d then
      let entries = perform fs.read_dir(path: [=[{dir}]=])
      let has_file = contains(xs: entries, target: [=[a.txt]=])
      if has_file then
        return contains(xs: entries, target: [=[sub]=])
      else
        return false
      endif
    else
      return false
    endif
  else
    return false
  endif
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> bool effect {{ IO }} do
  let ok = perform fs.create_dir_all(path: [=[{dir}]=])
  if ok then
    let entries = perform fs.read_dir(path: [=[{dir}]=])
    match entries do
      case Nil() -> return true
      case Cons(v: _, rest: _) -> return false
    endmatch
  else
    return false
  endif
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> unit effect {{ IO }} do
  let leak = fn () -> unit effect {{ IO, Exn }} do
    let _ = perform fs.create_dir_all(path: [=[{dir}]=])
    let _ = perform fs.write_string(path: [=[{file}]=], content: [=[abc]=])
    let f = perform fs.open_read(path: [=[{file}]=])
    return ()
  endfn
  try
    perform leak()
  catch e ->
    return ()
  endtry
  return ()
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> unit effect {{ IO }} do
  let bad = fn () -> unit effect {{ IO, Exn }} do
    let _ = perform fs.create_dir_all(path: [=[{dir}]=])
    let _ = perform fs.write_string(path: [=[{file}]=], content: [=[abc]=])
    let f = perform fs.open_read(path: [=[{file}]=])
    fs.close(closer: f)
    fs.close(closer: f)
    return ()
  endfn
  try
    perform bad()
  catch e ->
    return ()
  endtry
  return ()
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> bool effect {{ IO }} do
  try
    let _ = perform fs.create_dir_all(path: [=[{dir}]=])
    let _ = perform fs.write_string(path: [=[{file}]=], content: [=[abc]=])
    let c = perform fs.open_read(path: [=[{file}]=])
    let content = perform fs.read_to_string(path: [=[{file}]=])
    fs.close(closer: c)
    if string_length(s: content) == 3 then
      return true
    else
      return false
    endif
  catch e ->
    return false
  endtry
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> bool effect {{ IO }} do
  try
    let c = perform fs.open_read(path: [=[{file}]=])
    fs.close(closer: c)
    return false
  catch e ->
    match e do
      case RuntimeError(val: _) -> return true
      case InvalidIndex(val: _) -> return false
    endmatch
  endtry
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> bool effect {{ IO }} do
  try
    let c = perform fs.open_write(path: [=[{file}]=])
    fs.close(closer: c)
    return false
  catch e ->
    match e do
      case RuntimeError(val: _) -> return true
      case InvalidIndex(val: _) -> return false
    endmatch
  endtry
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
import as fs from [=[nxlib/stdlib/fs.nx]=]

let main = fn () -> bool effect {{ IO }} do
  try
    let c = perform fs.open_append(path: [=[{file}]=])
    fs.close(closer: c)
    return false
  catch e ->
    match e do
      case RuntimeError(val: _) -> return true
      case InvalidIndex(val: _) -> return false
    endmatch
  endtry
endfn
"#
    );
    assert_eq!(run(&src).unwrap(), Value::Bool(true));
}
