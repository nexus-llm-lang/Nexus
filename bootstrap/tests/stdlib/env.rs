use crate::harness::{
    exec_with_stdlib, exec_with_stdlib_envs, should_fail_typecheck, should_typecheck,
};

#[test]
fn env_port_typechecks_with_perm_env() {
    should_typecheck(
        r#"
import { Env }, * as env_mod from "std:env"

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
import { Env }, * as env_mod from "std:env"

let main = fn () -> unit do
  inject env_mod.system_handler do
    let _ = Env.get(key: "HOME")
    return ()
  end
end
"#,
    );
    insta::assert_snapshot!(err);
}

#[test]
fn env_set_typechecks_with_perm_env() {
    should_typecheck(
        r#"
import { Env }, * as env_mod from "std:env"

let main = fn () -> unit require { PermEnv } do
  inject env_mod.system_handler do
    Env.set(key: "TEST_VAR", value: "hello")
  end
end
"#,
    );
}

// Regression: an env var deliberately set to "" must round-trip as Some("")
// rather than being collapsed to None (nexus-9lp4.25).
#[test]
fn env_get_set_to_empty_returns_some_empty() {
    exec_with_stdlib_envs(
        r#"
import { Env }, * as env_mod from "std:env"
import { Option } from "std:option"

let main = fn () -> unit require { PermEnv } do
  inject env_mod.system_handler do
    match Env.get(key: "NX_EMPTY_TEST") do
      | Some(val: v) ->
        if v != "" then raise RuntimeError(val: "expected empty string") end
        return ()
      | None -> raise RuntimeError(val: "expected Some(\"\") but got None")
    end
  end
end
"#,
        &[("NX_EMPTY_TEST", "")],
    );
}

// Baseline: an unset env var resolves to None.
#[test]
fn env_get_unset_returns_none() {
    exec_with_stdlib_envs(
        r#"
import { Env }, * as env_mod from "std:env"
import { Option } from "std:option"

let main = fn () -> unit require { PermEnv } do
  inject env_mod.system_handler do
    match Env.get(key: "NX_NEVER_DEFINED_VAR") do
      | Some(val: _) -> raise RuntimeError(val: "expected None for unset var")
      | None -> return ()
    end
  end
end
"#,
        &[],
    );
}

// Baseline: a non-empty set var round-trips through Env.get.
#[test]
fn env_get_set_to_value_returns_some_value() {
    exec_with_stdlib_envs(
        r#"
import { Env }, * as env_mod from "std:env"
import { Option } from "std:option"

let main = fn () -> unit require { PermEnv } do
  inject env_mod.system_handler do
    match Env.get(key: "NX_SET_VAR") do
      | Some(val: v) ->
        if v != "hello" then raise RuntimeError(val: "expected hello") end
        return ()
      | None -> raise RuntimeError(val: "expected Some")
    end
  end
end
"#,
        &[("NX_SET_VAR", "hello")],
    );
}

#[test]
fn env_mock_handler() {
    exec_with_stdlib(
        r#"
import { Env } from "std:env"
import { Option } from "std:option"

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
      | Some(val: v) ->
        if v != "mock_value" then raise RuntimeError(val: "expected mock_value") end
        return ()
      | None -> raise RuntimeError(val: "expected Some")
    end
  end
end
"#,
    );
}
