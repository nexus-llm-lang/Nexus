use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use wasmparser::Payload;

fn imported_modules(path: &Path) -> BTreeSet<String> {
    let wasm = fs::read(path).expect("wasm file should be readable");
    let mut out = BTreeSet::new();
    for payload in wasmparser::Parser::new(0).parse_all(&wasm) {
        let payload = payload.expect("wasm payload should parse");
        if let Payload::ImportSection(section) = payload {
            for import in section {
                let import = import.expect("wasm import should parse");
                out.insert(import.module.to_string());
            }
        }
    }
    out
}

#[test]
fn stdlib_wasm_modules_are_wasi_only_or_self_contained() {
    let stdlib_dir = Path::new("nxlib/stdlib");
    let entries = fs::read_dir(stdlib_dir).expect("nxlib/stdlib should exist");

    let mut checked = 0usize;
    for entry in entries {
        let entry = entry.expect("dir entry should be readable");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }
        checked += 1;

        let modules = imported_modules(&path);
        assert!(
            !modules.contains("nexus_host"),
            "unexpected nexus_host import in {}",
            path.display()
        );
    }

    assert!(checked > 0, "at least one stdlib wasm should be checked");
}
