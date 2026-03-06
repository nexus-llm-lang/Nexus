use crate::common::source::{check, run};
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
    import as array from stdlib/array.nx
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
    import as array from stdlib/array.nx
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
    let src = &crate::common::fixtures::read_test_fixture("test_array_module_fold_left_sum.nx");
    assert_eq!(run(src).unwrap(), Value::Int(10));
}

#[test]
fn test_array_module_find_index_any_all() {
    let src =
        &crate::common::fixtures::read_test_fixture("test_array_module_find_index_any_all.nx");
    assert_eq!(run(src).unwrap(), Value::Int(2));
}

#[test]
fn test_array_module_map_in_place_and_for_each() {
    let src = &crate::common::fixtures::read_test_fixture(
        "test_array_module_map_in_place_and_for_each.nx",
    );
    assert_eq!(run(src).unwrap(), Value::Int(6));
}

#[test]
fn test_array_module_filter_returns_list() {
    let src =
        &crate::common::fixtures::read_test_fixture("test_array_module_filter_returns_list.nx");
    assert_eq!(run(src).unwrap(), Value::Int(32));
}

#[test]
fn test_array_module_partition_returns_two_lists() {
    let src = &crate::common::fixtures::read_test_fixture(
        "test_array_module_partition_returns_two_lists.nx",
    );
    assert_eq!(run(src).unwrap(), Value::Int(22));
}

#[test]
fn test_array_module_zip_with_and_zip() {
    let src = &crate::common::fixtures::read_test_fixture("test_array_module_zip_with_and_zip.nx");
    assert_eq!(run(src).unwrap(), Value::Int(222));
}

#[test]
fn test_array_consume_nonlinear_consumer_is_rejected() {
    let src = r#"
    import as array from stdlib/array.nx

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
    import as array from stdlib/array.nx

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
