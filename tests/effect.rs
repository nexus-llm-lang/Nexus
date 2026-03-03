mod common;

use common::source::check_raw as check;

#[test]
fn test_effect_propagation() {
    // f has IO effect. g calls f, so g must have IO effect.
    let src = r#"
    type IO = {}

    let f = fn () -> unit effect { IO } do
        return ()
    end

    let g = fn () -> unit effect { IO } do
        f()
    end

    let main = fn () -> unit do
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_call_pure_from_impure() {
    let src = r#"
    type IO = {}
    let pure_fn = fn () -> unit do return () end
    let impure_fn = fn () -> unit effect { IO } do
        pure_fn()
    end
    let main = fn () -> unit do
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_try_catch_removes_exn() {
    let src = r#"
    import { Console }, * as stdio from nxlib/stdlib/stdio.nx
    import { from_i64 } from nxlib/stdlib/string.nx
    exception Oops(string)

    let risky = fn () -> unit effect { Exn } do
        raise Oops([=[oops]=])
        return ()
    end

    let main = fn () -> unit require { PermConsole } do
        inject stdio.system_handler do
            try
                risky()
            catch e ->
                match e do
                    case Oops(msg) -> Console.print(val: msg)
                    case RuntimeError(msg) -> Console.print(val: msg)
                    case InvalidIndex(i) ->
                        let m = from_i64(val: i)
                        Console.print(val: m)
                end
            end
        end
        return ()
    end
    "#;
    if let Err(e) = check(src) {
        panic!("Typecheck failed: {}", e);
    }
}

#[test]
fn test_raise_requires_exn() {
    let src = r#"
    let fail = fn () -> unit do
        raise [=[oops]=] // Should fail: no Exn effect allowed
        return ()
    end
    let main = fn () -> unit do
        return ()
    end
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_main_cannot_declare_exn_effect() {
    let src = r#"
    let main = fn () -> unit effect { Exn } do
        return ()
    end
    "#;
    assert!(check(src).is_err());
}

#[test]
fn test_main_must_return_unit() {
    let src = r#"
    let main = fn () -> i64 do
        return 0
    end
    "#;
    let err = check(src).expect_err("main -> i64 must be rejected");
    assert!(err.contains("main must be a function '() -> unit'"), "unexpected error: {}", err);
}

#[test]
fn test_main_effect_net_only_is_rejected() {
    let src = r#"
    type Net = {}
    let main = fn () -> unit effect { Net } do
        return ()
    end
    "#;
    let err = check(src).expect_err("main with { Net } must fail");
    assert!(err.contains("main function effects must be {}"));
}

#[test]
fn test_main_require_known_perm_is_accepted() {
    // PermFs and PermNet are known runtime permissions allowed in main's require
    let src = r#"
    let main = fn () -> unit require { PermFs } do
        return ()
    end
    "#;
    assert!(check(src).is_ok(), "main with require {{ PermFs }} should be accepted");
}

#[test]
fn test_main_require_unknown_port_is_rejected() {
    let src = r#"
    port Custom do
      fn foo() -> unit
    end
    let main = fn () -> unit require { Custom } do
        return ()
    end
    "#;
    assert!(check(src).is_err(), "main with require {{ Custom }} should be rejected");
}

#[test]
fn test_main_require_port_name_is_rejected() {
    // Port names (Fs, Net) are not allowed in main's require — use PermFs/PermNet
    let src = r#"
    let main = fn () -> unit require { Net } do
        return ()
    end
    "#;
    assert!(check(src).is_err(), "main with require {{ Net }} should be rejected — use PermNet");
}

#[test]
fn test_effect_mismatch() {
    // g is declared pure but calls f (IO). Should fail.
    let src = r#"
    type IO = {}

    let f = fn () -> unit effect { IO } do
        return ()
    end

    let g = fn () -> unit effect {} do // Pure
        f()
    end

    let main = fn () -> unit do
        return ()
    end
    "#;
    assert!(check(src).is_err(), "Should fail because g calls impure f");
}

#[test]
fn test_effect_polymorphism() {
    let src = r#"
    type IO = {}

    let apply = fn <E>(f: () -> unit effect E) -> unit effect E do
        f()
    end

    let pure_fn = fn () -> unit effect {} do
        return ()
    end

    let impure_fn = fn () -> unit effect { IO } do
        return ()
    end

    let test_pure = fn () -> unit effect {} do
        apply(f: pure_fn)
    end

    let test_impure = fn () -> unit effect { IO } do
        apply(f: impure_fn)
    end

    let main = fn () -> unit do
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_effect_polymorphism_mismatch() {
    let src = r#"
    type IO = {}

    let apply = fn <E>(f: () -> unit effect E) -> unit effect E do
        f()
    end

    let impure_fn = fn () -> unit effect { IO } do
        return ()
    end

    let test_fail = fn () -> unit effect {} do // Declared Pure
        apply(f: impure_fn)     // Call is Impure (IO)
    end

    let main = fn () -> unit do
        return ()
    end
    "#;
    assert!(
        check(src).is_err(),
        "Should fail because apply instantiates E=IO, so call becomes IO"
    );
}
