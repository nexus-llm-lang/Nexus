use crate::harness::compile;

/// Component composition resolves all stdlib imports.
#[test]
fn compose_with_stdlib_resolves_imports() {
    let src = r#"
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "hello")
  end
  return ()
end
"#;
    let wasm = compile(src);
    let composed = nexus::compiler::compose::compose_with_stdlib(&wasm)
        .expect("compose_with_stdlib should resolve stdlib imports");
    // The result should be a valid component WASM.
    assert!(
        wasmparser::Parser::is_component(&composed),
        "composed output should be a component"
    );
}

/// Regression test for nexus-zekc: `compose_with_stdlib` previously derived
/// its staging temp_dir from `(pid, SystemTime nanos)`, so two threads inside
/// the same process that minted the same nanos value pointed at the same
/// directory and overwrote each other's `user.wasm` / `stdlib.wasm` —
/// producing wrong composed bytes that intermittently failed
/// `prop_math_max_symmetry` / `prop_string_length_concat`. This test composes
/// three distinct user modules concurrently from N worker threads — if the
/// staging path is not unique per call, at least one thread reads back wasm
/// bytes a sibling wrote and the byte-equal assertion fails. Mirrors the
/// `compile_fixture_via_nxc_is_thread_safe` idiom from nexus-gyj6.
#[test]
fn compose_with_stdlib_is_thread_safe() {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::thread;

    // Three structurally-distinct programs that exercise different stdlib
    // imports — so a path collision yields visibly-different composed bytes.
    let sources: [(&str, &str); 3] = [
        (
            "stdio_hello",
            r#"
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "hello")
  end
  return ()
end
"#,
        ),
        (
            "stdio_world",
            r#"
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "world")
    Console.println(val: "again")
  end
  return ()
end
"#,
        ),
        (
            "stdio_three_lines",
            r#"
import { Console }, * as stdio from "std:stdio"

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    Console.println(val: "alpha")
    Console.println(val: "beta")
    Console.println(val: "gamma")
  end
  return ()
end
"#,
        ),
    ];

    // Pre-compile each source's user core wasm once (compile is itself
    // thread-safe; we just want a stable input to the composer per source).
    let user_wasms: HashMap<&str, Vec<u8>> = sources
        .iter()
        .map(|(name, src)| (*name, compile(src)))
        .collect();
    let user_wasms = Arc::new(user_wasms);

    // Serial baseline: compose each user wasm once and capture the bytes —
    // any parallel run that disagrees with this baseline = staging-path race.
    let baselines: HashMap<&str, Vec<u8>> = user_wasms
        .iter()
        .map(|(name, wasm)| {
            let composed = nexus::compiler::compose::compose_with_stdlib(wasm)
                .expect("baseline compose should succeed");
            (*name, composed)
        })
        .collect();
    let baselines = Arc::new(baselines);

    // 4 rounds × 3 sources = 12 concurrent composes. Empirically enough to
    // expose the shared-path race on a 10-core M-series machine when the
    // bug is reintroduced; the correct fix passes deterministically.
    const ROUNDS: usize = 4;
    let mut handles = Vec::new();
    for round in 0..ROUNDS {
        for (i, (name, _)) in sources.iter().enumerate() {
            let user_wasms = Arc::clone(&user_wasms);
            let baselines = Arc::clone(&baselines);
            let name = *name;
            handles.push(thread::spawn(move || {
                let user_wasm = user_wasms.get(name).unwrap();
                let composed = nexus::compiler::compose::compose_with_stdlib(user_wasm)
                    .expect("parallel compose should succeed");
                let expected = baselines.get(name).unwrap();
                assert_eq!(
                    composed.len(),
                    expected.len(),
                    "round={round} idx={i} name={name}: parallel compose produced \
                     {} bytes but serial baseline was {} — likely staging-dir \
                     collision overwrite by another thread",
                    composed.len(),
                    expected.len()
                );
                assert_eq!(
                    composed, *expected,
                    "round={round} idx={i} name={name}: parallel compose produced \
                     different bytes than serial baseline"
                );
            }));
        }
    }
    for h in handles {
        h.join().expect("worker thread panicked");
    }
}
