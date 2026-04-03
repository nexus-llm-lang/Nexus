use crate::harness::compile;

#[test]
fn bundle_core_wasm_resolves_stdlib_imports() {
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
    let config = nexus::compiler::bundler::BundleConfig::default();
    let merged = nexus::compiler::bundler::bundle_core_wasm(&wasm, &config)
        .expect("bundle_core_wasm should resolve stdlib imports");
    let merged_imports =
        nexus::compiler::bundler::module_import_names(&merged).expect("parse merged imports");
    assert!(
        !merged_imports.iter().any(|m| m.contains("stdlib")),
        "stdlib imports should be resolved after bundling, got: {:?}",
        merged_imports
    );
}
