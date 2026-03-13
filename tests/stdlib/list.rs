use crate::harness::{exec, should_fail_typecheck};

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
