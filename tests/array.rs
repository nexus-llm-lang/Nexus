mod common;

use common::source::{check, run};
use nexus::interpreter::Value;

#[test]
fn test_array_type_mismatch() {
    let src = r#"
    let main = fn () -> unit do
        let %arr = [| 1, true |]
        match %arr do case _ -> () end
        return ()
    end
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
    end
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_array_assignment_mismatch() {
    let src = r#"
    let main = fn () -> unit do
        let %arr = [| 1, 2 |]
        %arr[0] <- true
        match %arr do case _ -> () end
        return ()
    end
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_array_module_get_set_is_empty() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx
    let main = fn () -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = &%arr
      let empty = array.is_empty(arr: arr_ref)
      array.set(arr: arr_ref, idx: 1, val: 99)
      let v = array.get(arr: arr_ref, idx: 1)
      match %arr do case _ -> () end
      if empty then return 0 else return v end
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(99));
}

#[test]
fn test_array_module_head_last() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx
    let main = fn () -> i64 do
      let %arr = [| 10, 20, 30 |]
      let arr_ref = &%arr
      let h = array.head(arr: arr_ref)
      let t = array.last(arr: arr_ref)
      match %arr do case _ -> () end
      return h + t
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(40));
}

#[test]
fn test_array_module_fold_left_sum() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx

    let add = fn (acc: i64, val: i64) -> i64 do
      return acc + val
    end

    let main = fn () -> i64 do
      let %arr = [| 1, 2, 3, 4 |]
      let arr_ref = &%arr
      let sum = array.fold_left(arr: arr_ref, init: 0, f: add)
      match %arr do case _ -> () end
      return sum
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_array_module_find_index_any_all() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx

    let is_eight = fn (val: i64) -> bool do
      return val == 8
    end

    let gt_ten = fn (val: i64) -> bool do
      return val > 10
    end

    let main = fn () -> i64 do
      let %arr = [| 3, 5, 8, 11 |]
      let arr_ref = &%arr
      let idx = array.find_index(arr: arr_ref, pred: is_eight)
      let has_even = array.any(arr: arr_ref, pred: is_eight)
      let all_gt_ten = array.all(arr: arr_ref, pred: gt_ten)
      match %arr do case _ -> () end

      if has_even then
        if all_gt_ten then
          return -1
        else
          return idx
        end
      else
        return -1
      end
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(2));
}

#[test]
fn test_array_module_map_in_place_and_for_each() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx

    let twice = fn (val: i64) -> i64 do
      return val * 2
    end

    let noop = fn (val: i64) -> unit do
      return ()
    end

    let main = fn () -> i64 do
      let %arr = [| 1, 2, 3 |]
      let arr_ref = &%arr
      array.for_each(arr: arr_ref, f: noop)
      array.map_in_place(arr: arr_ref, f: twice)
      let v = array.get(arr: arr_ref, idx: 2)
      match %arr do case _ -> () end
      return v
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(6));
}

#[test]
fn test_array_module_filter_returns_list() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx
    import as list from nxlib/stdlib/list.nx

    let gt_two = fn (val: i64) -> bool do
      return val > 2
    end

    let main = fn () -> i64 do
      let %arr = [| 1, 2, 3, 4 |]
      let arr_ref = &%arr
      let ys = array.filter(arr: arr_ref, pred: gt_two)
      match %arr do case _ -> () end
      let h = list.head(xs: ys)
      let n = list.length(xs: ys)
      return h * 10 + n
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(32));
}

#[test]
fn test_array_module_partition_returns_two_lists() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx
    import as list from nxlib/stdlib/list.nx

    let gt_two = fn (val: i64) -> bool do
      return val > 2
    end

    let main = fn () -> i64 do
      let %arr = [| 1, 2, 3, 4 |]
      let arr_ref = &%arr
      let parts = array.partition(arr: arr_ref, pred: gt_two)
      match %arr do case _ -> () end
      match parts do
        case Partition(matched: m, rest: r) ->
          let a = list.length(xs: m)
          let b = list.length(xs: r)
          return a * 10 + b
      end
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(22));
}

#[test]
fn test_array_module_zip_with_and_zip() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx
    import as list from nxlib/stdlib/list.nx

    let add_pair = fn (left: i64, right: i64) -> i64 do
      return left + right
    end

    let main = fn () -> i64 do
      let %a = [| 1, 2, 3 |]
      let %b = [| 10, 20 |]
      let ar = &%a
      let br = &%b
      let zipped = array.zip_with(left: ar, right: br, f: add_pair)
      let plain = array.zip(left: ar, right: br)
      match %a do case _ -> () end
      match %b do case _ -> () end
      let v = list.nth(xs: zipped, n: 1)
      let len = list.length(xs: plain)
      return v * 10 + len
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Int(222));
}

#[test]
fn test_array_consume_nonlinear_consumer_is_rejected() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx

    let ignore_record = fn (val: { id: i64 }) -> unit do
        return ()
    end

    let main = fn () -> unit do
        let %arr = [| { id: 1 }, { id: 2 } |]
        array.consume(arr: %arr, f: ignore_record)
        return ()
    end
    "#;
    assert!(
        check(src).is_err(),
        "non-linear consumer should be rejected: consumer param must be %T"
    );
}

#[test]
fn test_array_consume_with_proper_consumer_passes() {
    let src = r#"
    import as array from nxlib/stdlib/array.nx

    let consume_record = fn (%val: { id: i64 }) -> unit do
        match %val do case { id: _ } -> () end
        return ()
    end

    let main = fn () -> unit do
        let %arr = [| { id: 1 }, { id: 2 } |]
        array.consume(arr: %arr, f: consume_record)
        return ()
    end
    "#;
    match check(src) {
        Ok(_) => (),
        Err(e) => panic!("proper consumer should pass: {}", e),
    }
}
