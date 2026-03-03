mod common;

use common::source::check;

#[test]
fn proc_exit_typechecks_with_perm_proc() {
    let src = r#"
import { Proc }, * as proc_mod from nxlib/stdlib/proc.nx

let main = fn () -> unit require { PermProc } do
  inject proc_mod.system_handler do
    Proc.exit(status: 0)
  end
end
"#;
    assert!(check(src).is_ok(), "Proc.exit with PermProc should typecheck");
}

#[test]
fn proc_exit_requires_perm_proc() {
    let src = r#"
import { Proc }, * as proc_mod from nxlib/stdlib/proc.nx

let main = fn () -> unit do
  inject proc_mod.system_handler do
    Proc.exit(status: 0)
  end
end
"#;
    assert!(
        check(src).is_err(),
        "Proc.exit without PermProc should fail typechecking"
    );
}

#[test]
fn proc_port_with_mock_handler() {
    // Test that Proc port can be implemented with a mock handler
    // (doesn't actually exit the process)
    let src = r#"
import { Proc } from nxlib/stdlib/proc.nx

let mock_proc = handler Proc do
  fn exit(status: i64) -> unit do
    return ()
  end
end

let main = fn () -> unit do
  inject mock_proc do
    Proc.exit(status: 0)
  end
end
"#;
    assert!(check(src).is_ok(), "Mock Proc handler should typecheck");
}
