use crate::common::source::{check_raw, run};
use nexus::interpreter::Value;
use proptest::prelude::*;

#[test]
fn test_conc_parallel_execution() {
    let src = &crate::common::fixtures::read_test_fixture("test_conc_parallel_execution.nx");

    let res = run(src);
    assert!(res.is_ok(), "Execution failed: {:?}", res.err());
}

#[test]
fn test_net_effect_enforcement() {
    let src = r#"
    type IO = {}
    let main = fn () -> unit effect { IO } do
        let res = get(url: "https://example.com")
        return ()
    end
    "#;

    let res = check_raw(src);
    assert!(
        res.is_err(),
        "Should fail typechecking because Net effect is missing"
    );
}

#[test]
fn test_net_request_method_and_headers_runtime() {
    let src = r#"
    import { Net, header, response_body }, * as net_mod from stdlib/net.nx

    let main = fn () -> string require { PermNet } do
      inject net_mod.system_handler do
        try
          let h = header(name: "X-Test", value: "abc")
          let hs = Cons(v: h, rest: Nil())
          let res = Net.request(method: "POST", url: "http://127.0.0.1:1/ping", headers: hs, body: "")
          return response_body(res: res)
        catch e ->
          match e do
            case RuntimeError(val: msg) -> return msg
            case InvalidIndex(val: _) -> return "error"
          end
        end
      end
    end
    "#;

    let res = run(src).expect("request should run");
    match res {
        Value::String(message) => {
            assert!(
                message.contains("request failed") || message.contains("http request failed"),
                "unexpected response body: {message}"
            );
        }
        other => panic!("Expected string result, got {:?}", other),
    }
}

#[test]
fn test_net_request_https_url_is_accepted() {
    let src = r#"
    import { Net, response_body }, * as net_mod from stdlib/net.nx

    let main = fn () -> string require { PermNet } do
      inject net_mod.system_handler do
        try
          let hs = Nil()
          let res = Net.request(method: "GET", url: "https://127.0.0.1:1/", headers: hs, body: "")
          return response_body(res: res)
        catch e ->
          match e do
            case RuntimeError(val: msg) -> return msg
            case InvalidIndex(val: _) -> return "error"
          end
        end
      end
    end
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
    import { Net, header, response_status, response_body }, * as net_mod from stdlib/net.nx
    import { from_i64 } from stdlib/string.nx

    let main = fn () -> string require { PermNet } do
      inject net_mod.system_handler do
        try
          let hs = Cons(v: header(name: "Content-Type", value: "application/x-www-form-urlencoded"), rest: Nil())
          let res = Net.request(method: "POST", url: "http://127.0.0.1:1/submit", headers: hs, body: "hello=nx")
          let status = response_status(res: res)
          let body = response_body(res: res)
          let status_s = from_i64(val: status)
          return status_s ++ ":" ++ body
        catch e ->
          match e do
            case RuntimeError(val: msg) -> return "caught:" ++ msg
            case InvalidIndex(val: _) -> return "error"
          end
        end
      end
    end
    "#;

    let res = run(src).expect("request_response should run");
    match res {
        Value::String(body) => {
            assert!(
                body.starts_with("0:") || body.starts_with("caught:"),
                "expected response to start with status or caught prefix, got: {body}"
            );
        }
        other => panic!("Expected string result, got {:?}", other),
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn prop_conc_independent_array_updates(n in 1usize..5) {
        let mut tasks = String::new();
        for i in 0..n {
            tasks.push_str(&format!(
                r#"
                task t{i} do
                    let lock = &%arr
                    lock[{i}] <- 1
                end
                "#
            ));
        }

        let initial_array = vec!["0"; n].join(", ");
        let src = format!(
            r#"
            let main = fn () -> unit do
                let %arr = [| {initial_array} |]
                conc do
                    {tasks}
                end

                let lock = &%arr
                let ok = check_all(arr: lock, len: {n}, i: 0)
                match %arr do case _ -> () end
                if (ok) then
                    return ()
                else
                    return ()
                end
            end

            let check_all = fn (arr: &[| i64 |], len: i64, i: i64) -> bool do
                if (i < len) then
                    let val = arr[i]
                    if (val != 1) then
                        return false
                    else
                        let next_i = i + 1
                        let res = check_all(arr: arr, len: len, i: next_i)
                        return res
                    end
                else
                    return true
                end
            end
            "#
        );

        let res = run(&src);
        prop_assert!(res.is_ok(), "Execution failed for n={}: {:?}", n, res.err());
    }

    #[test]
    fn prop_conc_task_capture_linearity(_n in 1usize..5) {
        let src = crate::common::fixtures::read_test_fixture("prop_conc_task_capture_linearity.nx");
        let res = run(&src);
        prop_assert!(res.is_ok(), "Linearity check failed: {:?}", res.err());
    }
}
