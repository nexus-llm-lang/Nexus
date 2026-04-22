use crate::harness::compile;

/// Component composition resolves all stdlib imports.
#[test]
fn compose_with_stdlib_resolves_imports() {
    let src = r#"
import { Console }, * as stdio from "stdlib/stdio.nx"

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
