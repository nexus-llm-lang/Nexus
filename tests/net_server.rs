use chumsky::Parser;
use nexus::interpreter::{Interpreter, Value};
use nexus::lang::parser::parser;
use nexus::lang::typecheck::TypeChecker;

fn check(src: &str) -> Result<(), String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)
}

fn run(src: &str) -> Result<Value, String> {
    let p = parser().parse(src).map_err(|e| format!("{:?}", e))?;
    let mut checker = TypeChecker::new();
    checker.check_program(&p).map_err(|e| e.message)?;
    let mut interpreter = Interpreter::new(p);
    interpreter.run_function("main", vec![])
}

#[test]
fn net_server_types_check() {
    let src = r#"
    import { default_net, Net } from nxlib/stdlib/net.nx

    let main = fn () -> unit effect { Console } do
      inject default_net do
        try
          let server = Net.listen(addr: [=[127.0.0.1:0]=])
          Net.stop(server: server)
        catch e ->
          return ()
        endtry
      endinject
      return ()
    endfn
    "#;
    assert!(check(src).is_ok(), "Server API should typecheck");
}

#[test]
fn net_server_opaque_server_cannot_construct_externally() {
    let src = r#"
    import { default_net, Net } from nxlib/stdlib/net.nx

    let main = fn () -> unit do
      inject default_net do
        let s = Server(id: 0)
        Net.stop(server: s)
      endinject
      return ()
    endfn
    "#;
    assert!(
        check(src).is_err(),
        "constructing opaque Server externally should be rejected"
    );
}

#[test]
fn net_server_linear_leak_is_rejected() {
    let src = r#"
    import { default_net, Net } from nxlib/stdlib/net.nx

    let leak = fn () -> unit require { Net } effect { Exn } do
      let server = Net.listen(addr: [=[127.0.0.1:0]=])
      return ()
    endfn

    let main = fn () -> unit effect { Console } do
      inject default_net do
        try
          leak()
        catch e ->
          return ()
        endtry
      endinject
      return ()
    endfn
    "#;
    let err = check(src).expect_err("leaking linear Server should be a type error");
    assert!(
        err.contains("Unused linear"),
        "expected unused linear error, got: {}",
        err
    );
}

#[test]
fn net_server_listen_and_stop() {
    let src = r#"
    import { default_net, Net } from nxlib/stdlib/net.nx

    let main = fn () -> bool effect { Console } do
      inject default_net do
        try
          let server = Net.listen(addr: [=[127.0.0.1:0]=])
          Net.stop(server: server)
          return true
        catch e ->
          return false
        endtry
      endinject
    endfn
    "#;
    assert_eq!(run(src).unwrap(), Value::Bool(true));
}

#[test]
fn net_server_accept_and_respond() {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    // Find a free port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let addr = format!("127.0.0.1:{}", port);

    let src = format!(
        r#"
    import {{ default_net, Net, request_method, request_path }} from nxlib/stdlib/net.nx

    let main = fn () -> bool effect {{ Console }} do
      inject default_net do
        try
          let server = Net.listen(addr: [=[{addr}]=])
          let req = Net.accept(server: &server)
          let method = request_method(req: &req)
          let path = request_path(req: &req)
          let _ = Net.respond(req: req, status: 200, body: method ++ [=[ ]=] ++ path)
          Net.stop(server: server)
          return true
        catch e ->
          return false
        endtry
      endinject
    endfn
    "#
    );

    let addr_clone = addr.clone();

    // Spawn the server in a thread
    let server_thread = std::thread::spawn(move || run(&src));

    // Wait for server to start, retrying connection
    let mut stream = None;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(250));
        match TcpStream::connect(&addr_clone) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => continue,
        }
    }
    let mut stream = stream.expect("could not connect to server after retries");
    stream
        .write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("write failed");
    stream
        .shutdown(std::net::Shutdown::Write)
        .expect("shutdown failed");
    let mut response = String::new();
    stream.read_to_string(&mut response).expect("read failed");

    let server_result = server_thread.join().expect("server thread panicked");
    assert_eq!(server_result.unwrap(), Value::Bool(true));

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
    let src = r#"
    import { Net } from nxlib/stdlib/net.nx

    let main = fn () -> string do
      let body = Net.get(url: [=[https://example.com]=])
      return body
    endfn
    "#;
    let err = check(src).expect_err("Net.get without inject Net should be a type error");
    assert!(
        err.contains("requires") || err.contains("Net"),
        "expected coeffect error, got: {}",
        err
    );
}
