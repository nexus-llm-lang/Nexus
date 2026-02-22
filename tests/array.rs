use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

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

#[test]
fn test_array_basic() {
    let src = r#"
    let main = fn () -> unit effect { IO } do
        let %arr = [| 1, 2, 3 |]
        %arr[0] <- 42
        let val = (borrow %arr)[0]
        let msg = i64_to_string(val: val)
        perform print(val: msg)
        drop %arr
        return ()
    endfn
    "#;
    if let Err(e) = check(src) {
        panic!("Typecheck failed: {}", e);
    }
}

#[test]
fn test_array_type_mismatch() {
    let src = r#"
    let main = fn () -> unit do
        let %arr = [| 1, true |]
        drop %arr
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_array_indexing_non_array() {
    let src = r#"
    let main = fn () -> unit do
        let x = 10
        let v = x[0]
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_array_assignment_mismatch() {
    let src = r#"
    let main = fn () -> unit do
        let %arr = [| 1, 2 |]
        %arr[0] <- true // Should fail: assigning bool to i64 array
        drop %arr
        return ()
    endfn
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_array_module_is_empty() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]
    let main = fn () -> bool do
      let %arr = [| 1, 2, 3 |]
      let arr_ref = borrow %arr
      let r = array.is_empty(arr: arr_ref)
      drop %arr
      return r
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(false));
}

#[test]
fn test_array_module_get() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]
    let main = fn () -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = borrow %arr
      let v = array.get(arr: arr_ref, idx: 1)
      drop %arr
      return v
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(20));
}

#[test]
fn test_array_module_set() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]
    let main = fn () -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = borrow %arr
      array.set(arr: arr_ref, idx: 1, val: 99)
      let v = array.get(arr: arr_ref, idx: 1)
      drop %arr
      return v
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(99));
}

#[test]
fn test_array_module_head_and_last() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]
    let main = fn () -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = borrow %arr
      let h = array.head(arr: arr_ref)
      let t = array.last(arr: arr_ref)
      drop %arr
      return h + t
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(40));
}

#[test]
fn test_array_module_fold_left_sum() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]

    let add = fn (acc: i64, val: i64) -> i64 do
      return acc + val
    endfn

    let main = fn () -> i64 do
      let %arr = [| 1, 2, 3, 4 |]
      let arr_ref = borrow %arr
      let sum = array.fold_left(arr: arr_ref, init: 0, f: add)
      drop %arr
      return sum
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_array_module_find_index_any_all() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]

    let is_eight = fn (val: i64) -> bool do
      return val == 8
    endfn

    let gt_ten = fn (val: i64) -> bool do
      return val > 10
    endfn

    let main = fn () -> i64 do
      let %arr = [| 3, 5, 8, 11 |]
      let arr_ref = borrow %arr
      let idx = array.find_index(arr: arr_ref, pred: is_eight)
      let has_even = array.any(arr: arr_ref, pred: is_eight)
      let all_gt_ten = array.all(arr: arr_ref, pred: gt_ten)
      drop %arr

      if has_even then
        if all_gt_ten then
          return -1
        else
          return idx
        endif
      else
        return -1
      endif
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(2));
}

#[test]
fn test_array_module_map_in_place() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]

    let twice = fn (val: i64) -> i64 do
      return val * 2
    endfn

    let main = fn () -> i64 do
      let %arr = [| 1, 2, 3 |]
      let arr_ref = borrow %arr
      array.map_in_place(arr: arr_ref, f: twice)
      let v = array.get(arr: arr_ref, idx: 2)
      drop %arr
      return v
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(6));
}

#[test]
fn test_array_module_for_each_noop() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]

    let noop = fn (val: i64) -> unit do
      return ()
    endfn

    let main = fn () -> i64 do
      let %arr = [| 5, 6, 7 |]
      let arr_ref = borrow %arr
      array.for_each(arr: arr_ref, f: noop)
      let v = array.get(arr: arr_ref, idx: 1)
      drop %arr
      return v
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(6));
}

#[test]
fn test_array_module_filter_returns_list() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]
    import as list from [=[nxlib/stdlib/list.nx]=]

    let gt_two = fn (val: i64) -> bool do
      return val > 2
    endfn

    let main = fn () -> i64 do
      let %arr = [| 1, 2, 3, 4 |]
      let arr_ref = borrow %arr
      let ys = array.filter(arr: arr_ref, pred: gt_two)
      drop %arr
      let h = list.head(xs: ys)
      let n = list.length(xs: ys)
      return h * 10 + n
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(32));
}

#[test]
fn test_array_module_partition_returns_two_lists() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]
    import as list from [=[nxlib/stdlib/list.nx]=]

    let gt_two = fn (val: i64) -> bool do
      return val > 2
    endfn

    let main = fn () -> i64 do
      let %arr = [| 1, 2, 3, 4 |]
      let arr_ref = borrow %arr
      let parts = array.partition(arr: arr_ref, pred: gt_two)
      drop %arr
      match parts do
        case Partition(matched: m, rest: r) ->
          let a = list.length(xs: m)
          let b = list.length(xs: r)
          return a * 10 + b
      endmatch
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(22));
}

#[test]
fn test_array_module_zip_with() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]
    import as list from [=[nxlib/stdlib/list.nx]=]

    let add_pair = fn (left: i64, right: i64) -> i64 do
      return left + right
    endfn

    let main = fn () -> i64 do
      let %a = [| 1, 2, 3 |]
      let %b = [| 10, 20 |]
      let ar = borrow %a
      let br = borrow %b
      let zs = array.zip_with(left: ar, right: br, f: add_pair)
      drop %a
      drop %b
      return list.nth(xs: zs, n: 1)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(22));
}

#[test]
fn test_array_module_zip_length_is_min() {
    let src = r#"
    import as array from [=[nxlib/stdlib/array.nx]=]
    import as list from [=[nxlib/stdlib/list.nx]=]

    let main = fn () -> i64 do
      let %a = [| 1, 2, 3, 4 |]
      let %b = [| 10, 20 |]
      let ar = borrow %a
      let br = borrow %b
      let zs = array.zip(left: ar, right: br)
      drop %a
      drop %b
      return list.length(xs: zs)
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(2));
}
