
use crate::common::source::{check, run};

#[test]
fn test_port_basic() {
    let src = r#"
    import { Console }, * as stdio from nxlib/stdlib/stdio.nx

    port Logger do
      fn log(msg: string) -> unit
    end

    let main = fn () -> unit require { PermConsole } do
      let stdout_logger = handler Logger require { Console } do
        fn log(msg: string) -> unit do
          Console.println(val: msg)
          return ()
        end
      end

      inject stdio.system_handler do
        inject stdout_logger do
          Logger.log(msg: "test message")
        end
      end
      return ()
    end
    "#;
    let res = run(src);
    assert!(res.is_ok(), "Execution failed: {:?}", res.err());
}

#[test]
fn test_port_redefinition_wins() {
    let src = r#"
    import { Console }, * as stdio from nxlib/stdlib/stdio.nx
    import { from_i64 } from nxlib/stdlib/string.nx

    port Adder do
      fn add_one(n: i64) -> i64
    end

    let main = fn () -> unit require { PermConsole } do
      let normal_adder = handler Adder do
        fn add_one(n: i64) -> i64 do
          return n + 1
        end
      end

      let weird_adder = handler Adder do
        fn add_one(n: i64) -> i64 do
          return n + 2
        end
      end

      inject stdio.system_handler, weird_adder do
        let result = Adder.add_one(n: 10)
        let msg = from_i64(val: result)
        Console.print(val: msg)
      end
      return ()
    end
    "#;
    let res = run(src);
    assert!(res.is_ok(), "Execution failed: {:?}", res.err());
}

#[test]
fn test_handler_require_mock_needs_nothing() {
    // Mock handler (no require) → main needs nothing
    let src = r#"
    import { Fs, Handle } from nxlib/stdlib/fs.nx

    let mock_fs = handler Fs do
      fn exists(path: string) -> bool do return false end
      fn read_to_string(path: string) -> string do return "" end
      fn write_string(path: string, content: string) -> unit effect { Exn } do return () end
      fn append_string(path: string, content: string) -> unit effect { Exn } do return () end
      fn remove_file(path: string) -> unit effect { Exn } do return () end
      fn create_dir_all(path: string) -> unit effect { Exn } do return () end
      fn read_dir(path: string) -> List<Handle> effect { Exn } do return Nil() end
      fn open_read(path: string) -> %Handle effect { Exn } do
        let h = Handle(id: 0)
        let %lh = h
        return %lh
      end
      fn open_write(path: string) -> %Handle effect { Exn } do
        let h = Handle(id: 0)
        let %lh = h
        return %lh
      end
      fn open_append(path: string) -> %Handle effect { Exn } do
        let h = Handle(id: 0)
        let %lh = h
        return %lh
      end
      fn read(handle: %Handle) -> { content: string, handle: %Handle } do
        match handle do case Handle(id: id) ->
          let h = Handle(id: id)
          let %lh = h
          return { content: "", handle: %lh }
        end
      end
      fn fd_write(handle: %Handle, content: string) -> { ok: bool, handle: %Handle } do
        match handle do case Handle(id: id) ->
          let h = Handle(id: id)
          let %lh = h
          return { ok: true, handle: %lh }
        end
      end
      fn fd_path(handle: %Handle) -> { path: string, handle: %Handle } do
        match handle do case Handle(id: id) ->
          let h = Handle(id: id)
          let %lh = h
          return { path: "", handle: %lh }
        end
      end
      fn close(handle: %Handle) -> unit do
        match handle do case Handle(id: _) -> return () end
      end
    end

    let main = fn () -> bool do
      inject mock_fs do
        return Fs.exists(path: "anything")
      end
    end
    "#;
    // Mock handler has no require, so main needs nothing — should typecheck and run
    let res = run(src);
    assert!(
        res.is_ok(),
        "Mock handler should not require capabilities: {:?}",
        res.err()
    );
}

#[test]
fn test_handler_require_real_propagates_perm() {
    // Real handler (require { PermFs }) → main needs PermFs
    let src = r#"
    import { Fs }, * as fs_mod from nxlib/stdlib/fs.nx

    let main = fn () -> bool require { PermFs } do
      inject fs_mod.system_handler do
        return Fs.exists(path: "/tmp")
      end
    end
    "#;
    // system_handler has require { PermFs }, so main must declare require { PermFs }
    assert!(
        check(src).is_ok(),
        "Real handler with matching require should typecheck"
    );
}

#[test]
fn test_handler_require_propagates_through_nested_inject() {
    let src = r#"
    port Inner do
      fn value() -> i64
    end

    port Outer do
      fn compute() -> i64
    end

    let real_inner = handler Inner require { PermFs } do
      fn value() -> i64 do return 42 end
    end

    let real_outer = handler Outer do
      fn compute() -> i64 do return 1 end
    end

    let main = fn () -> i64 require { PermFs } do
      inject real_outer do
        inject real_inner do
          let a = Outer.compute()
          let b = Inner.value()
          return a + b
        end
      end
    end
    "#;
    assert!(
        check(src).is_ok(),
        "Nested inject should propagate handler requires"
    );
}

#[test]
fn test_handler_require_multiple_merged() {
    let src = r#"
    port A do
      fn a_val() -> i64
    end

    port B do
      fn b_val() -> i64
    end

    let real_a = handler A require { PermFs } do
      fn a_val() -> i64 do return 1 end
    end

    let real_b = handler B require { PermNet } do
      fn b_val() -> i64 do return 2 end
    end

    let main = fn () -> i64 require { PermFs, PermNet } do
      inject real_a, real_b do
        let x = A.a_val()
        let y = B.b_val()
        return x + y
      end
    end
    "#;
    assert!(
        check(src).is_ok(),
        "Multiple handler requires should merge"
    );
}

#[test]
fn test_handler_require_syntax_parses() {
    let src = r#"
    port P do
      fn op() -> i64
    end

    let h = handler P require { PermFs, PermNet } do
      fn op() -> i64 do return 0 end
    end

    let main = fn () -> i64 require { PermFs, PermNet } do
      inject h do
        return P.op()
      end
    end
    "#;
    assert!(
        check(src).is_ok(),
        "Handler with require {{ Fs, Net }} should parse and typecheck"
    );
}

#[test]
fn test_handler_require_missing_is_rejected() {
    let src = r#"
    import { Fs }, * as fs_mod from nxlib/stdlib/fs.nx

    let main = fn () -> bool do
      inject fs_mod.system_handler do
        return Fs.exists(path: "/tmp")
      end
    end
    "#;
    assert!(
        check(src).is_err(),
        "Missing require {{ PermFs }} should be rejected"
    );
}
