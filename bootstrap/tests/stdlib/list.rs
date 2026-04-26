use crate::harness::{exec, should_fail_typecheck, should_typecheck};

#[test]
fn test_list_builtin_no_import() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs = [10, 20, 30]
        match xs do
            | Nil -> raise RuntimeError(val: "expected Cons")
            | Cons(v: h, rest: _) ->
                if h != 10 then raise RuntimeError(val: "expected 10") end
                return ()
        end
    end
    "#,
    );
}

#[test]
fn test_list_constructor_returns_list_type() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs: [i64] = Cons(v: 1, rest: Nil)
        match xs do
            | Cons(v: h, rest: _) ->
                if h != 1 then raise RuntimeError(val: "expected 1") end
                return ()
            | Nil -> raise RuntimeError(val: "expected Cons")
        end
    end
    "#,
    );
}

#[test]
fn test_list_type_mismatch() {
    let err = should_fail_typecheck(
        r#"
    let main = fn () -> unit do
        let l = [1, true]
        return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn test_cons_operator_expr() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs = 1 :: 2 :: 3 :: Nil
        match xs do
            | Cons(v: a, rest: Cons(v: b, rest: Cons(v: c, rest: Nil))) ->
                if a != 1 then raise RuntimeError(val: "expected 1") end
                if b != 2 then raise RuntimeError(val: "expected 2") end
                if c != 3 then raise RuntimeError(val: "expected 3") end
                return ()
            | _ -> raise RuntimeError(val: "expected 3-element list")
        end
    end
    "#,
    );
}

#[test]
fn test_cons_operator_pattern() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs = [10, 20, 30]
        match xs do
            | h :: t ->
                if h != 10 then raise RuntimeError(val: "expected 10") end
                match t do
                    | h2 :: _ ->
                        if h2 != 20 then raise RuntimeError(val: "expected 20") end
                        return ()
                    | _ -> raise RuntimeError(val: "expected tail")
                end
            | _ -> raise RuntimeError(val: "expected cons")
        end
    end
    "#,
    );
}

#[test]
fn test_cons_operator_chained_pattern() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs = [1, 2, 3]
        match xs do
            | a :: b :: c :: Nil ->
                if a != 1 then raise RuntimeError(val: "expected 1") end
                if b != 2 then raise RuntimeError(val: "expected 2") end
                if c != 3 then raise RuntimeError(val: "expected 3") end
                return ()
            | _ -> raise RuntimeError(val: "expected 3-element list")
        end
    end
    "#,
    );
}

#[test]
fn test_empty_list_pattern() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs: [i64] = Nil
        match xs do
            | [] -> return ()
            | _ -> raise RuntimeError(val: "expected empty list")
        end
    end
    "#,
    );
}

#[test]
fn test_cons_operator_with_empty_list_pattern() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs = [42]
        match xs do
            | h :: [] ->
                if h != 42 then raise RuntimeError(val: "expected 42") end
                return ()
            | _ -> raise RuntimeError(val: "expected singleton list")
        end
    end
    "#,
    );
}

#[test]
fn test_cons_operator_expr_prepend_to_list() {
    exec(
        r#"
    let main = fn () -> unit do
        let tail = [2, 3]
        let xs = 1 :: tail
        match xs do
            | Cons(v: h, rest: _) ->
                if h != 1 then raise RuntimeError(val: "expected 1") end
                return ()
            | Nil -> raise RuntimeError(val: "expected Cons")
        end
    end
    "#,
    );
}

#[test]
fn test_cons_operator_precedence() {
    // 1 + 2 :: Nil should parse as (1 + 2) :: Nil
    exec(
        r#"
    let main = fn () -> unit do
        let xs = 1 + 2 :: Nil
        match xs do
            | Cons(v: h, rest: Nil) ->
                if h != 3 then raise RuntimeError(val: "expected 3") end
                return ()
            | _ -> raise RuntimeError(val: "expected [3]")
        end
    end
    "#,
    );
}

#[test]
fn test_cons_operator_typechecks() {
    should_typecheck(
        r#"
    let main = fn () -> unit do
        let xs: [i64] = 1 :: 2 :: Nil
        return ()
    end
    "#,
    );
}

#[test]
fn partition_type_in_list_module() {
    exec(
        r#"
import { Partition } from "std:list"

let main = fn () -> unit do
  let p = Partition(matched: Cons(v: 1, rest: Nil), rest: Nil)
  match p do
    | Partition(matched: m, rest: _) ->
      match m do
        | Cons(v: v, rest: _) ->
            if v != 1 then raise RuntimeError(val: "expected 1") end
            return ()
        | Nil -> raise RuntimeError(val: "expected Cons")
      end
  end
end
"#,
    );
}

#[test]
fn deforestation_map_map_fusion() {
    exec(
        r#"
import * as list from "std:list"

let double = fn (val: i64) -> i64 do return val * 2 end
let inc = fn (val: i64) -> i64 do return val + 1 end

let main = fn () -> unit do
    let xs = [1, 2, 3]
    // map(inc, map(double, xs)) should fuse into map(inc∘double, xs)
    let result = list.map(xs: list.map(xs: xs, f: double), f: inc)
    // Expected: [3, 5, 7] = [(1*2+1), (2*2+1), (3*2+1)]
    match result do
        | Cons(v: h, rest: _) ->
            if h != 3 then raise RuntimeError(val: "expected 3") end
            return ()
        | Nil -> raise RuntimeError(val: "expected non-empty")
    end
end
"#,
    );
}

#[test]
fn concat_tail_recursive_deep() {
    exec(
        r#"
import * as list from "std:list"

let make_list = fn (n: i64, acc: [ i64 ]) -> [ i64 ] do
  if n == 0 then return acc end
  return make_list(n: n - 1, acc: n :: acc)
end

let main = fn () -> unit do
    // concat must be tail-safe: 50k prefix ++ 3-element suffix should not overflow.
    let xs = make_list(n: 50000, acc: [])
    let ys = make_list(n: 3, acc: [])
    let zs = list.concat(xs: xs, ys: ys)
    // Head of result should be 1 (preserved order).
    match zs do
        | Cons(v: h, rest: _) ->
            if h != 1 then raise RuntimeError(val: "expected 1 at head of concat") end
            return ()
        | Nil -> raise RuntimeError(val: "expected non-empty concat result")
    end
end
"#,
    );
}

#[test]
fn length_tail_recursive_deep() {
    exec(
        r#"
import * as list from "std:list"

let make_list = fn (n: i64, acc: [ i64 ]) -> [ i64 ] do
  if n == 0 then return acc end
  return make_list(n: n - 1, acc: n :: acc)
end

let main = fn () -> unit do
    // length must be tail-safe at 50k elements.
    let xs = make_list(n: 50000, acc: [])
    let n = list.length(xs: xs)
    if n != 50000 then raise RuntimeError(val: "expected length 50000") end
    return ()
end
"#,
    );
}

#[test]
fn take_tail_recursive_deep() {
    exec(
        r#"
import * as list from "std:list"

let make_list = fn (n: i64, acc: [ i64 ]) -> [ i64 ] do
  if n == 0 then return acc end
  return make_list(n: n - 1, acc: n :: acc)
end

let main = fn () -> unit do
    // take must be tail-safe: taking 50k elements from a 60k list.
    let xs = make_list(n: 60000, acc: [])
    let ys = list.take(xs: xs, n: 50000)
    let n = list.length(xs: ys)
    if n != 50000 then raise RuntimeError(val: "expected length 50000 after take") end
    return ()
end
"#,
    );
}

#[test]
fn deforestation_reverse_reverse_identity() {
    exec(
        r#"
import * as list from "std:list"

let main = fn () -> unit do
    let xs = [10, 20, 30]
    // reverse(reverse(xs)) should collapse to xs
    let result = list.reverse(xs: list.reverse(xs: xs))
    match result do
        | Cons(v: h, rest: _) ->
            if h != 10 then raise RuntimeError(val: "expected 10") end
            return ()
        | Nil -> raise RuntimeError(val: "expected non-empty")
    end
end
"#,
    );
}
