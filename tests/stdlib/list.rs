use crate::harness::{exec, should_fail_typecheck, should_typecheck};

#[test]
fn test_list_builtin_no_import() {
    exec(
        r#"
    let main = fn () -> unit do
        let xs = [10, 20, 30]
        match xs do
            case Nil -> raise RuntimeError(val: "expected Cons")
            case Cons(v: h, rest: _) ->
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
            case Cons(v: h, rest: _) ->
                if h != 1 then raise RuntimeError(val: "expected 1") end
                return ()
            case Nil -> raise RuntimeError(val: "expected Cons")
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
            case Cons(v: a, rest: Cons(v: b, rest: Cons(v: c, rest: Nil))) ->
                if a != 1 then raise RuntimeError(val: "expected 1") end
                if b != 2 then raise RuntimeError(val: "expected 2") end
                if c != 3 then raise RuntimeError(val: "expected 3") end
                return ()
            case _ -> raise RuntimeError(val: "expected 3-element list")
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
            case h :: t ->
                if h != 10 then raise RuntimeError(val: "expected 10") end
                match t do
                    case h2 :: _ ->
                        if h2 != 20 then raise RuntimeError(val: "expected 20") end
                        return ()
                    case _ -> raise RuntimeError(val: "expected tail")
                end
            case _ -> raise RuntimeError(val: "expected cons")
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
            case a :: b :: c :: Nil ->
                if a != 1 then raise RuntimeError(val: "expected 1") end
                if b != 2 then raise RuntimeError(val: "expected 2") end
                if c != 3 then raise RuntimeError(val: "expected 3") end
                return ()
            case _ -> raise RuntimeError(val: "expected 3-element list")
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
            case [] -> return ()
            case _ -> raise RuntimeError(val: "expected empty list")
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
            case h :: [] ->
                if h != 42 then raise RuntimeError(val: "expected 42") end
                return ()
            case _ -> raise RuntimeError(val: "expected singleton list")
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
            case Cons(v: h, rest: _) ->
                if h != 1 then raise RuntimeError(val: "expected 1") end
                return ()
            case Nil -> raise RuntimeError(val: "expected Cons")
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
            case Cons(v: h, rest: Nil) ->
                if h != 3 then raise RuntimeError(val: "expected 3") end
                return ()
            case _ -> raise RuntimeError(val: "expected [3]")
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
import { Partition } from stdlib/list.nx

let main = fn () -> unit do
  let p = Partition(matched: Cons(v: 1, rest: Nil), rest: Nil)
  match p do
    case Partition(matched: m, rest: _) ->
      match m do
        case Cons(v: v, rest: _) ->
            if v != 1 then raise RuntimeError(val: "expected 1") end
            return ()
        case Nil -> raise RuntimeError(val: "expected Cons")
      end
  end
end
"#,
    );
}
