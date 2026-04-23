/// Fuzz-style tests for the typechecker.
///
/// The property under test: the typechecker must NEVER panic.
/// It should always return Ok or Err — never crash, overflow the stack,
/// or hit an unreachable!() on any syntactically valid (or invalid) input.
///
/// We use proptest to generate both:
/// 1. Random strings (catching parser + typecheck panic paths)
/// 2. Structurally valid but semantically random programs (catching deeper bugs)
use nexus::lang::parser;
use nexus::lang::typecheck::TypeChecker;
use proptest::prelude::*;

/// Parse and typecheck, catching panics. Returns true if no panic occurred.
fn parse_and_typecheck_no_panic(src: &str) -> bool {
    std::panic::catch_unwind(|| {
        if let Ok(program) = parser::parser().parse(src) {
            let mut checker = TypeChecker::new();
            let _ = checker.check_program(&program);
        }
    })
    .is_ok()
}

// ---------------------------------------------------------------------------
// Random byte string fuzzing — the cheapest, broadest coverage
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn fuzz_random_ascii_no_panic(s in "[a-zA-Z0-9_ \t\n(){}<>:,=+\\-*/|.;\\[\\]&%~!@#$^]{0,200}") {
        prop_assert!(
            parse_and_typecheck_no_panic(&s),
            "parser/typechecker panicked on input: {:?}", s
        );
    }

    #[test]
    fn fuzz_random_keyword_soup_no_panic(
        fragments in proptest::collection::vec(
            prop_oneof![
                Just("let".to_string()),
                Just("fn".to_string()),
                Just("do".to_string()),
                Just("end".to_string()),
                Just("return".to_string()),
                Just("if".to_string()),
                Just("then".to_string()),
                Just("else".to_string()),
                Just("match".to_string()),
                Just("case".to_string()),
                Just("type".to_string()),
                Just("port".to_string()),
                Just("handler".to_string()),
                Just("inject".to_string()),
                Just("import".to_string()),
                Just("from".to_string()),
                Just("raise".to_string()),
                Just("try".to_string()),
                Just("catch".to_string()),
                Just("while".to_string()),
                Just("true".to_string()),
                Just("false".to_string()),
                Just("unit".to_string()),
                Just("i64".to_string()),
                Just("bool".to_string()),
                Just("string".to_string()),
                Just("float".to_string()),
                Just("->".to_string()),
                Just("=>".to_string()),
                Just("<".to_string()),
                Just(">".to_string()),
                Just("(".to_string()),
                Just(")".to_string()),
                Just("{".to_string()),
                Just("}".to_string()),
                Just("[".to_string()),
                Just("]".to_string()),
                Just(":".to_string()),
                Just(",".to_string()),
                Just("=".to_string()),
                Just("+".to_string()),
                Just("-".to_string()),
                Just("*".to_string()),
                Just("/".to_string()),
                Just("++".to_string()),
                Just("%".to_string()),
                Just("~".to_string()),
                Just("&".to_string()),
                Just("_".to_string()),
                Just("\n".to_string()),
                "[a-z]{1,8}".prop_map(|s| s),
                "[A-Z][a-z]{0,7}".prop_map(|s| s),
                "-?[0-9]{1,5}".prop_map(|s| s),
            ],
            1..30,
        )
    ) {
        let src = fragments.join(" ");
        prop_assert!(
            parse_and_typecheck_no_panic(&src),
            "parser/typechecker panicked on keyword soup: {:?}", src
        );
    }
}

// ---------------------------------------------------------------------------
// Structurally valid program fuzzing — deeper typechecker coverage
// ---------------------------------------------------------------------------

fn gen_type() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("i64".to_string()),
        Just("bool".to_string()),
        Just("string".to_string()),
        Just("unit".to_string()),
        Just("float".to_string()),
    ]
}

fn gen_literal() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<i64>().prop_map(|n| n.to_string()),
        any::<bool>().prop_map(|b| b.to_string()),
        "[a-zA-Z0-9 ]{0,20}".prop_map(|s| format!("[=[{}]=]", s)),
        Just("()".to_string()),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn fuzz_valid_let_binding_no_panic(
        name in "[a-z][a-z0-9_]{0,7}",
        ty in gen_type(),
        val in gen_literal(),
    ) {
        let src = format!(
            r#"
let main = fn () -> unit do
    let {name}: {ty} = {val}
    return ()
end
"#
        );
        prop_assert!(
            parse_and_typecheck_no_panic(&src),
            "panicked on: {}", src
        );
    }

    #[test]
    fn fuzz_valid_function_def_no_panic(
        fname in "[a-z][a-z0-9_]{0,5}",
        pname in "[a-z][a-z0-9_]{0,5}",
        pty in gen_type(),
        rty in gen_type(),
        body_val in gen_literal(),
    ) {
        let src = format!(
            r#"
let {fname} = fn ({pname}: {pty}) -> {rty} do
    return {body_val}
end
"#
        );
        prop_assert!(
            parse_and_typecheck_no_panic(&src),
            "panicked on: {}", src
        );
    }

    #[test]
    fn fuzz_deeply_nested_if_no_panic(depth in 1usize..10) {
        let mut src = String::from("let main = fn () -> unit do\n    return ");
        for _ in 0..depth {
            src.push_str("if true then ");
        }
        src.push_str("()");
        for _ in 0..depth {
            src.push_str(" else () end");
        }
        src.push_str("\nend\n");
        prop_assert!(
            parse_and_typecheck_no_panic(&src),
            "panicked on depth {}: {}", depth, src
        );
    }

    #[test]
    fn fuzz_deeply_nested_match_no_panic(depth in 1usize..8) {
        let mut src = String::from("let main = fn () -> unit do\n");
        for i in 0..depth {
            src.push_str(&format!(
                "    let x{} = match true do\n        | true ->\n",
                i
            ));
        }
        src.push_str("            ()");
        for _ in 0..depth {
            src.push_str("\n        | false -> ()\n    end");
        }
        src.push_str("\n    return ()\nend\n");
        prop_assert!(
            parse_and_typecheck_no_panic(&src),
            "panicked on depth {}: {}", depth, src
        );
    }

    #[test]
    fn fuzz_many_params_no_panic(count in 1usize..15) {
        let params: Vec<String> = (0..count)
            .map(|i| format!("p{}: i64", i))
            .collect();
        let src = format!(
            r#"
let f = fn ({}) -> i64 do
    return 0
end
"#,
            params.join(", ")
        );
        prop_assert!(
            parse_and_typecheck_no_panic(&src),
            "panicked on {} params: {}", count, src
        );
    }

    #[test]
    fn fuzz_many_type_params_no_panic(count in 1usize..10) {
        let tparams: Vec<String> = (0..count)
            .map(|i| format!("T{}", i))
            .collect();
        let params: Vec<String> = (0..count)
            .map(|i| format!("x{}: T{}", i, i))
            .collect();
        let src = format!(
            r#"
let f = fn <{}>({}) -> T0 do
    return x0
end
"#,
            tparams.join(", "),
            params.join(", "),
        );
        prop_assert!(
            parse_and_typecheck_no_panic(&src),
            "panicked on {} type params: {}", count, src
        );
    }

    #[test]
    fn fuzz_enum_many_variants_no_panic(count in 1usize..12) {
        let variants: Vec<String> = (0..count)
            .map(|i| format!("V{}", i))
            .collect();
        let cases: Vec<String> = (0..count)
            .map(|i| format!("            | V{} -> return ()", i))
            .collect();
        let src = format!(
            r#"
type MyEnum = {}

let main = fn () -> unit do
    let x: MyEnum = V0
    match x do
{}
    end
end
"#,
            variants.join(" | "),
            cases.join("\n"),
        );
        prop_assert!(
            parse_and_typecheck_no_panic(&src),
            "panicked on {} variants", count
        );
    }
}
