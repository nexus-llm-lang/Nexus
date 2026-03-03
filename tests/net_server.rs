mod common;

use common::source::{check, run};
use nexus::interpreter::Value;
static TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());


#[test]
fn net_server_types_check() {
    let src = r#"
    import { Net }, * as net_mod from nxlib/stdlib/net.nx

    let main = fn () -> unit require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: [=[127.0.0.1:0]=])
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
    let src = r#"
    import { Net }, * as net_mod from nxlib/stdlib/net.nx

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
    let src = r#"
    import { Net }, * as net_mod from nxlib/stdlib/net.nx

    let leak = fn () -> unit require { Net } effect { Exn } do
      let server = Net.listen(addr: [=[127.0.0.1:0]=])
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
    assert!(
        err.contains("Unused linear"),
        "expected unused linear error, got: {}",
        err
    );
}

#[test]
fn net_server_listen_and_stop() {
    let src = r#"
    import { Net }, * as net_mod from nxlib/stdlib/net.nx

    let main = fn () -> bool require { PermNet } do
      inject net_mod.system_handler do
        try
          let server = Net.listen(addr: [=[127.0.0.1:0]=])
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
    use std::io::{Read, Write};
    use std::net::TcpStream;

    // Use a time-based port to avoid race conditions with other tests binding to 0
    let port = 40000 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos() % 10000) as u16;
    let addr = format!("127.0.0.1:{}", port);

    let src = format!(
        r#"
    import {{ Net, request_method, request_path }}, * as net_mod from nxlib/stdlib/net.nx

    let main = fn () -> bool require {{ PermNet }} effect {{ Console }} do
      inject net_mod.system_handler do
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
        end
      end
    end
    "#
    );

    let addr_clone = addr.clone();

    // Spawn the server in a thread
    let server_thread = std::thread::spawn(move || {
        println!("===> server thread starting: port {}", addr_clone);
        let res = run(&src);
        println!("===> server thread finished with {:?}", res);
        res.expect("run(&src) failed")
    });

    // Wait for server to start, retrying connection
    let mut stream = None;
    for i in 0..40 {
        if server_thread.is_finished() {
            println!("===> thread is finished at iteration {}", i);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
        match TcpStream::connect(&addr) {
            Ok(s) => {
                println!("===> connection succeeded at iteration {}", i);
                stream = Some(s);
                break;
            }
            Err(e) => {
                if i % 10 == 0 {
                    println!("===> connection failed at iteration {}: {}", i, e);
                }
                continue;
            }
        }
    }
    
    if server_thread.is_finished() && stream.is_none() {
        // Thread died early, let's join to see the panic
        server_thread.join().expect("server thread panicked early");
        panic!("server thread finished but did not panic, yet connection failed");
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
    let src = r#"
    import { Net } from nxlib/stdlib/net.nx

    let main = fn () -> string do
      let body = Net.get(url: [=[https://example.com]=])
      return body
    end
    "#;
    let err = check(src).expect_err("Net.get without inject Net should be a type error");
    assert!(
        err.contains("requires") || err.contains("Net"),
        "expected coeffect error, got: {}",
        err
    );
}
