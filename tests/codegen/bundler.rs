use crate::harness::compile;

#[test]
fn bundle_core_wasm_resolves_stdlib_imports() {
    let src = r#"
import { Console }, * as stdio from stdlib/stdio.nx

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

#[test]
fn bundle_core_wasm_resolves_conc_plus_stdlib() {
    let src = r#"
import { Console }, * as stdio from stdlib/stdio.nx

let work = fn () -> i64 do
  return 42
end

let main = fn () -> unit require { PermConsole } do
  inject stdio.system_handler do
    conc do
      task t1 do
        let _ = work()
      end
    end
    Console.println(val: "done")
  end
  return ()
end
"#;
    let wasm = compile(src);
    let imports = nexus::compiler::bundler::module_import_names(&wasm).expect("parse imports");
    assert!(imports.contains("nexus:runtime/conc"));
    assert!(imports.contains("nxlib/stdlib/stdlib.wasm"));

    let config = nexus::compiler::bundler::BundleConfig::default();
    let merged = nexus::compiler::bundler::bundle_core_wasm(&wasm, &config)
        .expect("bundle_core_wasm should succeed for conc+stdlib programs");
    let merged_imports =
        nexus::compiler::bundler::module_import_names(&merged).expect("parse merged imports");
    assert!(
        !merged_imports.iter().any(|m| m.contains("stdlib")),
        "stdlib should be resolved, got: {:?}",
        merged_imports
    );
    assert!(
        merged_imports.contains("nexus:runtime/conc"),
        "nexus:runtime/conc should remain (host-provided), got: {:?}",
        merged_imports
    );
}
