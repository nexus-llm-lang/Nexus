use crate::harness::{exec_with_stdlib, should_fail_typecheck, should_typecheck};

#[test]
fn env_port_typechecks_with_perm_env() {
    should_typecheck(
        r#"
import { Env }, * as env_mod from stdlib/env.nx

let main = fn () -> unit require { PermEnv } do
  inject env_mod.system_handler do
    Env.set(key: "X", value: "1")
  end
  return ()
end
"#,
    );
}

#[test]
fn env_get_requires_perm_env() {
    let err = should_fail_typecheck(
        r#"
import { Env }, * as env_mod from stdlib/env.nx

let main = fn () -> unit do
  inject env_mod.system_handler do
    let _ = Env.get(key: "HOME")
    return ()
  end
end
"#,
    );
    assert!(
        !err.is_empty(),
        "Env.get without PermEnv should fail typechecking"
    );
}

#[test]
fn env_set_typechecks_with_perm_env() {
    should_typecheck(
        r#"
import { Env }, * as env_mod from stdlib/env.nx

let main = fn () -> unit require { PermEnv } do
  inject env_mod.system_handler do
    Env.set(key: "TEST_VAR", value: "hello")
  end
end
"#,
    );
}

#[test]
fn env_mock_handler() {
    exec_with_stdlib(
        r#"
import { Env } from stdlib/env.nx
import { Option } from stdlib/option.nx

let mock_env = handler Env do
  fn get(key: string) -> Option<string> do
    return Some(val: "mock_value")
  end
  fn set(key: string, value: string) -> unit do
    return ()
  end
end

let main = fn () -> unit do
  inject mock_env do
    let result = Env.get(key: "MOCK_VAR")
    match result do
      case Some(val: v) ->
        if v != "mock_value" then raise RuntimeError(val: "expected mock_value") end
        return ()
      case None() -> raise RuntimeError(val: "expected Some")
    end
  end
end
"#,
    );
}
