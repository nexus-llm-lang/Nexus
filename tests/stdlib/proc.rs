use crate::harness::{should_fail_typecheck, should_typecheck};

#[test]
fn proc_exit_typechecks_with_perm_proc() {
    should_typecheck(
        r#"
import { Proc }, * as proc_mod from stdlib/proc.nx

let main = fn () -> unit require { PermProc } do
  inject proc_mod.system_handler do
    Proc.exit(status: 0)
  end
end
"#,
    );
}

#[test]
fn proc_exit_requires_perm_proc() {
    let err = should_fail_typecheck(
        r#"
import { Proc }, * as proc_mod from stdlib/proc.nx

let main = fn () -> unit do
  inject proc_mod.system_handler do
    Proc.exit(status: 0)
  end
end
"#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn proc_port_with_mock_handler() {
    should_typecheck(
        r#"
import { Proc, ExecResult } from stdlib/proc.nx

let mock_proc = handler Proc do
  fn exit(status: i64) -> unit do
    return ()
  end
  fn argv() -> [ string ] do
    return Nil
  end
  fn exec(cmd: string, args: [ string ]) -> ExecResult do
    return ExecResult(exit_code: 0, stdout: "", stderr: "")
  end
end

let main = fn () -> unit do
  inject mock_proc do
    Proc.exit(status: 0)
  end
end
"#,
    );
}
