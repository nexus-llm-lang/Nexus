mod common;

use common::source::check_raw as check;

#[test]
fn test_anonymous_record() {
    let src = r#"
    import { Console }, * as stdio from nxlib/stdlib/stdio.nx
    import { from_i64 } from nxlib/stdlib/string.nx
    let main = fn () -> unit require { PermConsole } do
        inject stdio.system_handler do
            let r = { x: 1, y: [=[hello]=] }
            let i = r.x
            let i_s = from_i64(val: i)
            let msg = [=[i=]=] ++ i_s
            Console.print(val: msg)
        end
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_record_unification() {
    // Structural typing: Order should not matter
    let src = r#"
    let take_record = fn (r: { x: i64, y: i64 }) -> unit do
        return ()
    end

    let main = fn () -> unit do
        let r1 = { x: 1, y: 2 }
        let r2 = { y: 2, x: 1 } // Different order
        take_record(r: r1)
        take_record(r: r2)
        return ()
    end
    "#;
    assert!(check(src).is_ok());
}

#[test]
fn test_record_fail() {
    let src = r#"
    let main = fn () -> unit do
        let r = { x: 1 }
        let y = r.y // Field missing
        return ()
    end
    "#;
    assert!(check(src).is_err());
}
