use crate::harness::exec;

/// Regression for nexus-gel8: bootstrap-Rust LIR lowering matched only
/// `LirAtom::Var` for assign targets, so `arr[idx] <- val` was silently
/// dropped (a dead `__array_get` got bound to a temp and the actual store
/// never happened). The fix dispatches on the MIR target shape — emitting
/// a `__array_set` intrinsic call that codegen lowers to an `i64.store`
/// at `(ptr + (idx+1)*8)`. Mirrors src/ir/lir.nx's MirStmtAssign branch.
#[test]
fn codegen_array_indexed_assign_writes_back() {
    exec(
        r#"
let main = fn () -> unit do
    let a = [| 0, 0, 0 |]
    a[1] <- 42
    let v = a[1]
    if v != 42 then raise RuntimeError(val: "expected 42") end
    return ()
end
"#,
    );
}

/// Each index is exercised independently against a fresh array — owned
/// arrays are linear and the bootstrap typechecker would refuse a write
/// followed by a same-array read otherwise. Mirrors the self-host fixture
/// at tests/codegen/codegen_array_indexed_assign_compiles_test.nx.
#[test]
fn codegen_array_indexed_assign_at_every_index() {
    exec(
        r#"
let main = fn () -> unit do
    let a0 = [| 0, 0, 0 |]
    a0[0] <- 10
    let v0 = a0[0]
    if v0 != 10 then raise RuntimeError(val: "expected 10 at idx 0") end

    let a1 = [| 0, 0, 0 |]
    a1[1] <- 20
    let v1 = a1[1]
    if v1 != 20 then raise RuntimeError(val: "expected 20 at idx 1") end

    let a2 = [| 0, 0, 0 |]
    a2[2] <- 30
    let v2 = a2[2]
    if v2 != 30 then raise RuntimeError(val: "expected 30 at idx 2") end
    return ()
end
"#,
    );
}
