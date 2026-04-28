use crate::harness::{should_fail_typecheck, should_typecheck};

static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn net_server_types_check() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    should_typecheck(
        r#"
    import { Net }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: "127.0.0.1:0")
          Net.stop(server: server)
        catch e ->
          return ()
        end
      end
      return ()
    end
    "#,
    );
}

#[test]
fn net_server_opaque_server_cannot_construct_externally() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let err = should_fail_typecheck(
        r#"
    import { Net }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        let s = Server(id: 0)
        Net.stop(server: s)
      end
      return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn net_server_linear_leak_is_rejected() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let err = should_fail_typecheck(
        r#"
    import { Net }, * as net_mod from "std:network"

    let leak = fn () -> unit require { Net } throws { Exn } do
      let server = Net.listen(addr: "127.0.0.1:0")
      return ()
    end

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          leak()
        catch e ->
          return ()
        end
      end
      return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn net_requires_inject() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let err = should_fail_typecheck(
        r#"
    import { Net } from "std:network"

    let main = fn () -> string do
      let body = Net.get(url: "https://example.com")
      return body
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn net_accept_without_respond_is_rejected() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let err = should_fail_typecheck(
        r#"
    import { Net }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: "127.0.0.1:0")
          let req = Net.accept(server: &server)
          Net.stop(server: server)
        catch e ->
          return ()
        end
      end
      return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn net_respond_consumes_request() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // Hole-2 (issue nexus-7eex.2): a throwable call (`Net.respond`) cannot
    // be invoked while a linear other than its own argument is live —
    // otherwise that linear would leak on raise. To verify that
    // `Net.respond` consumes its `req` argument without tripping that
    // safety check, we close `server` before the throwable call.
    should_typecheck(
        r#"
    import { Net }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: "127.0.0.1:0")
          let req = Net.accept(server: &server)
          Net.stop(server: server)
          Net.respond(req: req, status: 200, body: "ok")
        catch e ->
          return ()
        end
      end
      return ()
    end
    "#,
    );
}

#[test]
fn net_request_double_respond_is_rejected() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // Hole-2 (issue nexus-7eex.2): server must be closed before the first
    // throwable `Net.respond` call to avoid the cross-throwable leak. The
    // double-respond on the linear `req` is then the dominant error.
    let err = should_fail_typecheck(
        r#"
    import { Net }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: "127.0.0.1:0")
          let req = Net.accept(server: &server)
          Net.stop(server: server)
          Net.respond(req: req, status: 200, body: "ok")
          Net.respond(req: req, status: 200, body: "ok")
        catch e ->
          return ()
        end
      end
      return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn net_streaming_response_typechecks() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // nexus-upzz.6 streaming API: respond_streaming_start consumes the linear
    // Request and returns a linear RespondStream, write() borrows the stream,
    // finish() consumes the stream. The full happy path must typecheck.
    should_typecheck(
        r#"
    import { Net }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: "127.0.0.1:0")
          let req = Net.accept(server: &server)
          Net.stop(server: server)
          let stream = Net.respond_streaming_start(req: req, status: 200, headers: [])
          let _ = Net.respond_streaming_write(stream: &stream, chunk: "hello")
          let _ = Net.respond_streaming_write(stream: &stream, chunk: " world")
          Net.respond_streaming_finish(stream: stream)
        catch e ->
          return ()
        end
      end
      return ()
    end
    "#,
    );
}

#[test]
fn net_streaming_finish_required_to_release_linear_stream() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    // nexus-upzz.6: a RespondStream that is started but never finished must
    // be rejected by linearity. Drop the linear stream without calling
    // respond_streaming_finish — typecheck must fail.
    let err = should_fail_typecheck(
        r#"
    import { Net }, * as net_mod from "std:network"

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: "127.0.0.1:0")
          let req = Net.accept(server: &server)
          Net.stop(server: server)
          let stream = Net.respond_streaming_start(req: req, status: 200, headers: [])
          let _ = Net.respond_streaming_write(stream: &stream, chunk: "hello")
          return ()
        catch e ->
          return ()
        end
      end
      return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn net_respond_returns_unit_effect_exn() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    should_typecheck(
        r#"
import { Net }, * as net_mod from "std:network"
import { length } from "std:str"

let main = fn () -> unit require { PermNet } do
  inject net_mod.system_handler do
    try
      let body = Net.get(url: "http://example.com")
      let _ = length(s: body)
      return ()
    catch e ->
      return ()
    end
  end
end
"#,
    );
}
