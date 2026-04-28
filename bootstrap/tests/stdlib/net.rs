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
