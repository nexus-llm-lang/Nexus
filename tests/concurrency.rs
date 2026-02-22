use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;
use proptest::prelude::*;

fn run(src: &str) -> Result<Value, String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)?;
    let mut interpreter = Interpreter::new(p);
    interpreter.run_function("main", vec![])
}

#[test]
fn test_conc_parallel_execution() {
    let src = r#"
    let main = fn () -> unit effect { IO } do
        let %arr = [| 0, 0 |]
        conc do
            task t1 effect { IO } do
                let lock = borrow %arr
                lock[0] <- 1
            endtask
            task t2 effect { IO } do
                let lock = borrow %arr
                lock[1] <- 2
            endtask
        endconc

        let lock = borrow %arr
        let v1 = lock[0]
        let v2 = lock[1]
        let s1 = i64_to_string(val: v1)
        let s2 = i64_to_string(val: v2)
        perform print(val: s1)
        perform print(val: s2)
        drop %arr
        return ()
    endfn
    "#;

    let res = run(src);
    assert!(res.is_ok(), "Execution failed: {:?}", res.err());
}

#[test]
fn test_net_effect_enforcement() {
    let src = r#"
    let main = fn () -> unit effect { IO } do
        let res = perform get(url: [=[https://example.com]=])
        return ()
    endfn
    "#;

    let program = parser().parse(src).unwrap();
    let mut checker = TypeChecker::new();
    let res = checker.check_program(&program);
    assert!(
        res.is_err(),
        "Should fail typechecking because Net effect is missing"
    );
}

#[test]
fn test_net_request_method_and_headers_runtime() {
    let src = r#"
    import as net from [=[nxlib/stdlib/net.nx]=]

    let main = fn () -> string effect { IO, Net } do
      let h = net.header(name: [=[X-Test]=], value: [=[abc]=])
      let hs = Cons(v: h, rest: Nil())
      return perform net.request(method: [=[POST]=], url: [=[http://127.0.0.1:1/ping]=], headers: hs)
    endfn
    "#;

    let res = run(src).expect("request should run");
    match res {
        Value::String(message) => {
            assert!(
                message.starts_with("http request failed:"),
                "unexpected response body: {message}"
            );
        }
        other => panic!("Expected string result, got {:?}", other),
    }
}

#[test]
fn test_net_request_https_url_is_accepted() {
    let src = r#"
    import as net from [=[nxlib/stdlib/net.nx]=]

    let main = fn () -> string effect { IO, Net } do
      let hs = Nil()
      return perform net.request(method: [=[GET]=], url: [=[https://127.0.0.1:1/]=], headers: hs)
    endfn
    "#;

    let res = run(src).expect("https request should return a string value");
    match res {
        Value::String(_) => {}
        other => panic!("Expected string result, got {:?}", other),
    }
}

#[test]
fn test_net_request_response_status_and_body_with_request_body() {
    let src = r#"
    import as net from [=[nxlib/stdlib/net.nx]=]

    let main = fn () -> string effect { IO, Net } do
      let hs = Cons(v: net.header(name: [=[Content-Type]=], value: [=[application/x-www-form-urlencoded]=]), rest: Nil())
      let res = perform net.request_response(method: [=[POST]=], url: [=[http://127.0.0.1:1/submit]=], headers: hs, body: [=[hello=nx]=])
      let status = net.response_status(res: res)
      let body = net.response_body(res: res)
      let status_s = i64_to_string(val: status)
      return status_s ++ [=[:]=] ++ body
    endfn
    "#;

    let res = run(src).expect("request_response should run");
    match res {
        Value::String(body) => {
            assert!(
                body.starts_with("0:"),
                "expected response to start with status prefix, got: {body}"
            );
        }
        other => panic!("Expected string result, got {:?}", other),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 16,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_conc_independent_array_updates(n in 1usize..10) {
        let mut tasks = String::new();
        for i in 0..n {
            tasks.push_str(&format!(
                r#"
                task t{i} effect {{ IO }} do
                    let lock = borrow %arr
                    lock[{i}] <- 1
                endtask
                "#
            ));
        }

        let initial_array = vec!["0"; n].join(", ");
        let src = format!(
            r#"
            let main = fn () -> unit effect {{ IO }} do
                let %arr = [| {initial_array} |]
                conc do
                    {tasks}
                endconc

                let lock = borrow %arr
                let ok = perform check_all(arr: lock, len: {n}, i: 0)
                drop %arr
                if (ok) then
                    return ()
                else
                    return ()
                endif
            endfn

            let check_all = fn (arr: &[| i64 |], len: i64, i: i64) -> bool effect {{ IO }} do
                if (i < len) then
                    let val = arr[i]
                    if (val != 1) then
                        return false
                    else
                        let next_i = i + 1
                        let res = perform check_all(arr: arr, len: len, i: next_i)
                        return res
                    endif
                else
                    return true
                endif
            endfn
            "#
        );

        let res = run(&src);
        prop_assert!(res.is_ok(), "Execution failed for n={}: {:?}", n, res.err());
    }

    #[test]
    fn prop_conc_task_capture_linearity(_n in 1usize..5) {
        let src = format!(
            r#"
            let main = fn () -> unit effect {{ IO }} do
                let %l = [| 42 |]
                conc do
                    task t1 effect {{ IO }} do
                        let b = borrow %l
                        let v = b[0]
                    endtask
                    task t2 effect {{ IO }} do
                        let b = borrow %l
                        let v = b[0]
                    endtask
                endconc
                let b = borrow %l
                let v = b[0]
                drop %l
                return ()
            endfn
            "#
        );
        let res = run(&src);
        prop_assert!(res.is_ok(), "Linearity check failed: {:?}", res.err());
    }
}
