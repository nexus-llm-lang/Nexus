use crate::common::source::{check, run};
use nexus::interpreter::Value;
static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn net_server_types_check() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let src = r#"
    import { Net }, * as net_mod from stdlib/net.nx

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
    "#;
    assert!(check(src).is_ok(), "Server API should typecheck");
}

#[test]
fn net_server_opaque_server_cannot_construct_externally() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let src = r#"
    import { Net }, * as net_mod from stdlib/net.nx

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        let s = Server(id: 0)
        Net.stop(server: s)
      end
      return ()
    end
    "#;
    assert!(
        check(src).is_err(),
        "constructing opaque Server externally should be rejected"
    );
}

#[test]
fn net_server_linear_leak_is_rejected() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let src = r#"
    import { Net }, * as net_mod from stdlib/net.nx

    let leak = fn () -> unit require { Net } effect { Exn } do
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
    "#;
    let err = check(src).expect_err("leaking linear Server should be a type error");
    insta::assert_snapshot!(err);
}

#[test]
fn net_server_listen_and_stop() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let src = r#"
    import { Net }, * as net_mod from stdlib/net.nx

    let main = fn () -> bool require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: "127.0.0.1:0")
          Net.stop(server: server)
          return true
        catch e ->
          return false
        end
      end
    end
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn net_server_accept_and_respond() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};

    // Bind to port 0 to get an OS-assigned free port, then release it
    let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind ephemeral port");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let addr = format!("127.0.0.1:{}", port);

    let src = format!(
        r#"
    import {{ Net, request_method, request_path }}, * as net_mod from stdlib/net.nx

    let main = fn () -> bool require {{ PermNet }} effect {{ Console }} do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: "{addr}")
          let req = Net.accept(server: &server)
          let method = request_method(req: &req)
          let path = request_path(req: &req)
          Net.respond(req: req, status: 200, body: method ++ " " ++ path)
          Net.stop(server: server)
          return true
        catch e ->
          return false
        end
      end
    end
    "#
    );

    let (tx, rx) = std::sync::mpsc::channel();

    // Spawn the server in a thread
    let server_thread = std::thread::spawn(move || {
        let res = run(&src);
        let _ = tx.send(());
        res.expect("run(&src) failed")
    });

    // Wait for server to start, retrying connection (max ~15s)
    let mut stream = None;
    for _ in 0..60 {
        if server_thread.is_finished() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        match TcpStream::connect(&addr) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => continue,
        }
    }

    if stream.is_none() {
        if server_thread.is_finished() {
            server_thread.join().expect("server thread panicked early");
        }
        panic!("could not connect to server at {} after retries", addr);
    }

    let mut stream = stream.unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .ok();
    stream
        .write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("write failed");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown failed");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read failed");

    // Wait for server thread with timeout
    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(_) => {}
        Err(_) => panic!("server thread did not finish within 10s"),
    }
    let server_result = server_thread.join().expect("server thread panicked");
    assert_eq!(server_result, Value::Bool(true));

    assert!(
        response.contains("200 OK"),
        "expected 200 OK in response, got: {}",
        response
    );
    assert!(
        response.contains("GET /hello"),
        "expected 'GET /hello' in response body, got: {}",
        response
    );
}

#[test]
fn net_requires_inject() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let src = r#"
    import { Net } from stdlib/net.nx

    let main = fn () -> string do
      let body = Net.get(url: "https://example.com")
      return body
    end
    "#;
    let err = check(src).expect_err("Net.get without inject Net should be a type error");
    insta::assert_snapshot!(err);
}

#[test]
fn net_respond_returns_unit_effect_exn() {
    let _lock = TEST_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let src = r#"
import { Net }, * as net_mod from stdlib/net.nx

let main = fn () -> string require { PermNet } do
  inject net_mod.system_handler do
    try
      let body = Net.get(url: "http://example.com")
      return body
    catch e ->
      return "error"
    end
  end
end
"#;
    assert!(
        check(src).is_ok(),
        "Net.get with effect Exn should typecheck: {:?}",
        check(src).err()
    );
}
