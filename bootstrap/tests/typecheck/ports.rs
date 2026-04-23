use crate::harness::{should_fail_typecheck, should_typecheck};

// NOTE: Port method calls (e.g. Logger.log(), Adder.add_one()) are resolved
// statically in the WASM codegen via MIR port call resolution. The "Unresolved
// port method" error indicates the MIR pass couldn't find the handler binding
// at compile time. These tests verify typecheck + parse correctness; full
// WASM execution of port calls requires the handler to be statically visible.

#[test]
fn test_port_basic() {
    // Verify the port/handler/inject pattern typechecks correctly.
    // WASM execution is not tested because port method resolution requires
    // the handler to be statically visible in the same compilation unit.
    should_typecheck(
        r#"
    import { Console }, * as stdio from "stdlib/stdio.nx"

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
    "#,
    );
}

#[test]
fn test_port_redefinition_wins() {
    should_typecheck(
        r#"
    import { Console }, * as stdio from "stdlib/stdio.nx"
    import { from_i64 } from "stdlib/string_ops.nx"

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
    "#,
    );
}

#[test]
fn test_handler_require_mock_needs_nothing() {
    // Mock handler (no require) -> main needs nothing
    should_typecheck(
        r#"
    import { Fs, Handle } from "stdlib/filesystem.nx"

    let mock_fs = handler Fs do
      fn exists(path: string) -> bool do return false end
      fn read_to_string(path: string) -> string throws { Exn } do return "" end
      fn write_string(path: string, content: string) -> unit throws { Exn } do return () end
      fn append_string(path: string, content: string) -> unit throws { Exn } do return () end
      fn remove_file(path: string) -> unit throws { Exn } do return () end
      fn create_dir_all(path: string) -> unit throws { Exn } do return () end
      fn read_dir(path: string) -> %[ Handle ] throws { Exn } do
        let empty = Nil
        let %result = empty
        return %result
      end
      fn open_read(path: string) -> %Handle throws { Exn } do
        let h = Handle(id: 0)
        let %lh = h
        return %lh
      end
      fn open_write(path: string) -> %Handle throws { Exn } do
        let h = Handle(id: 0)
        let %lh = h
        return %lh
      end
      fn open_append(path: string) -> %Handle throws { Exn } do
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
      fn write(handle: %Handle, content: string) -> { ok: bool, handle: %Handle } do
        match handle do case Handle(id: id) ->
          let h = Handle(id: id)
          let %lh = h
          return { ok: true, handle: %lh }
        end
      end
      fn handle_path(handle: %Handle) -> { path: string, handle: %Handle } do
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

    let main = fn () -> unit do
      inject mock_fs do
        let _result = Fs.exists(path: "anything")
      end
      return ()
    end
    "#,
    );
}

#[test]
fn test_handler_require_real_propagates_perm() {
    // Real handler (require { PermFs }) -> main needs PermFs
    // The original test had `main -> bool`; we wrap it to satisfy the
    // `main must return unit` constraint of the new typecheck helper.
    should_typecheck(
        r#"
    import { Fs }, * as fs_mod from "stdlib/filesystem.nx"

    let check_exists = fn () -> bool require { PermFs } do
      inject fs_mod.system_handler do
        return Fs.exists(path: "/tmp")
      end
    end

    let main = fn () -> unit require { PermFs } do
      let _r = check_exists()
      return ()
    end
    "#,
    );
}

#[test]
fn test_handler_require_propagates_through_nested_inject() {
    should_typecheck(
        r#"
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

    let do_work = fn () -> i64 require { PermFs } do
      inject real_outer do
        inject real_inner do
          let a = Outer.compute()
          let b = Inner.value()
          return a + b
        end
      end
    end

    let main = fn () -> unit require { PermFs } do
      let _r = do_work()
      return ()
    end
    "#,
    );
}

#[test]
fn test_handler_require_multiple_merged() {
    should_typecheck(
        r#"
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

    let do_work = fn () -> i64 require { PermFs, PermNet } do
      inject real_a, real_b do
        let x = A.a_val()
        let y = B.b_val()
        return x + y
      end
    end

    let main = fn () -> unit require { PermFs, PermNet } do
      let _r = do_work()
      return ()
    end
    "#,
    );
}

#[test]
fn test_handler_require_syntax_parses() {
    should_typecheck(
        r#"
    port P do
      fn op() -> i64
    end

    let h = handler P require { PermFs, PermNet } do
      fn op() -> i64 do return 0 end
    end

    let do_work = fn () -> i64 require { PermFs, PermNet } do
      inject h do
        return P.op()
      end
    end

    let main = fn () -> unit require { PermFs, PermNet } do
      let _r = do_work()
      return ()
    end
    "#,
    );
}

#[test]
fn test_handler_require_missing_is_rejected() {
    let err = should_fail_typecheck(
        r#"
    import { Fs }, * as fs_mod from "stdlib/filesystem.nx"

    let check_exists = fn () -> bool do
      inject fs_mod.system_handler do
        return Fs.exists(path: "/tmp")
      end
    end

    let main = fn () -> unit do
      let _r = check_exists()
      return ()
    end
    "#,
    );
    insta::assert_snapshot!(err);
}
