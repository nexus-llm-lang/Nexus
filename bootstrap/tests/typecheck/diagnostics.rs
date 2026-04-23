use crate::harness::typecheck_warnings;

#[test]
fn test_linear_primitive_emits_unnecessary_warning() {
    let warnings = typecheck_warnings(
        r#"
let main = fn () -> unit do
    let %x = 42
    return ()
end
"#,
    );
    assert!(
        warnings.iter().any(|w| w.contains("unnecessary")),
        "expected warning about unnecessary linear sigil on primitive, got: {:?}",
        warnings,
    );
}

#[test]
fn test_linear_record_does_not_emit_unnecessary_warning() {
    let warnings = typecheck_warnings(
        r#"
    let main = fn () -> unit do
        let %r = { id: 1 }
        match %r do | _ -> () end
        return ()
    end
"#,
    );
    assert!(
        !warnings.iter().any(|w| w.contains("unnecessary")),
        "unexpected warning for linear record: {:?}",
        warnings,
    );
}
