use chumsky::Parser;
use std::fs;
use std::path::{Path, PathBuf};

use super::ast::Program;
use super::parser;

pub const STDLIB_DIR: &str = "nxlib/stdlib";

/// Lists all `.nx` stdlib source files in lexical order.
pub fn list_stdlib_nx_paths() -> Result<Vec<PathBuf>, String> {
    let dir = Path::new(STDLIB_DIR);
    let entries = fs::read_dir(dir).map_err(|e| format!("Failed to read {}: {}", STDLIB_DIR, e))?;

    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read {} entry: {}", STDLIB_DIR, e))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("nx") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Parses every stdlib `.nx` file and returns `(path, Program)` pairs.
pub fn load_stdlib_nx_programs() -> Result<Vec<(PathBuf, Program)>, String> {
    let mut out = Vec::new();
    for path in list_stdlib_nx_paths()? {
        let src = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        let program = parser::parser()
            .parse(src)
            .map_err(|e| format!("Failed to parse {}: {:?}", path.display(), e))?;
        out.push((path, program));
    }
    Ok(out)
}
