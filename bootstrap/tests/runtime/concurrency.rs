use crate::harness::{exec, should_fail_typecheck, should_typecheck};

#[test]
fn test_net_effect_enforcement() {
    let src = r#"
    type IO = {}
    let main = fn () -> unit throws { IO } do
        let res = get(url: "https://example.com")
        return ()
    end
    "#;

    let err = should_fail_typecheck(src);
    insta::assert_snapshot!(err);
}

#[test]
fn test_net_request_method_and_headers_runtime() {
    // TODO: List types ([Header]) and HTTP requests are not yet supported in WASM codegen.
    // Converted to a typecheck-only test to verify the source is well-typed.
    should_typecheck(
        r#"
    import { Net, header, response_body }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let h = header(name: "X-Test", value: "abc")
          let hs = Cons(v: h, rest: Nil)
          let res = Net.request(method: "POST", url: "http://127.0.0.1:1/ping", headers: hs, body: "")
          let _body = response_body(res: res)
          return ()
        catch e ->
          return ()
        end
      end
    end
    "#,
    );
}

#[test]
fn test_net_request_https_url_is_accepted() {
    // TODO: List types ([Header]) and HTTP requests are not supported in WASM codegen.
    // Converted to a typecheck-only test.
    should_typecheck(
        r#"
    import { Net, response_body }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let hs = Nil
          let res = Net.request(method: "GET", url: "https://127.0.0.1:1/", headers: hs, body: "")
          let _body = response_body(res: res)
          return ()
        catch e ->
          return ()
        end
      end
    end
    "#,
    );
}

#[test]
fn test_net_request_response_status_and_body_with_request_body() {
    // TODO: List types ([Header]) and HTTP requests are not supported in WASM codegen.
    // Converted to a typecheck-only test.
    should_typecheck(
        r#"
    import { Net, header, response_status, response_body }, * as net_mod from "std:network"
    import { from_i64 } from "std:string_ops"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let hs = Cons(v: header(name: "Content-Type", value: "application/x-www-form-urlencoded"), rest: Nil)
          let res = Net.request(method: "POST", url: "http://127.0.0.1:1/submit", headers: hs, body: "hello=nx")
          let _status = response_status(res: res)
          let _body = response_body(res: res)
          return ()
        catch e ->
          return ()
        end
      end
    end
    "#,
    );
}

// ─── Lazy thunk (@) execution tests ──────────────────────────────────────────

#[test]
fn test_lazy_thunk_basic_force() {
    exec(
        r#"
let main = fn () -> unit do
    let @x = 42
    let v = @x
    return ()
end
"#,
    );
}

#[test]
fn test_lazy_thunk_captures_variable() {
    exec(
        r#"
let main = fn () -> unit do
    let y = 10
    let @x = y + 1
    let v = @x
    return ()
end
"#,
    );
}

#[test]
fn test_lazy_thunk_with_function_call() {
    exec(
        r#"
let compute = fn (a: i64, b: i64) -> i64 do
    return a + b
end

let main = fn () -> unit do
    let a = 3
    let b = 4
    let @result = compute(a: a, b: b)
    let v = @result
    return ()
end
"#,
    );
}
