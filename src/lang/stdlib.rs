use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use super::ast::Program;
use super::parser;

pub const STDLIB_DIR: &str = "nxlib/stdlib";
const STDLIB_PREFIX: &str = "stdlib/";

/// Resolves `stdlib/X` import paths to the physical `nxlib/stdlib/X` location.
pub fn resolve_import_path(path: &str) -> String {
    if path.starts_with(STDLIB_PREFIX) {
        format!("nxlib/{}", path)
    } else {
        path.to_string()
    }
}

/// Cached result of parsing all stdlib `.nx` files.
static STDLIB_CACHE: LazyLock<Mutex<Option<Vec<(PathBuf, Program)>>>> =
    LazyLock::new(|| Mutex::new(None));

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
/// Results are cached after the first successful call.
pub fn load_stdlib_nx_programs() -> Result<Vec<(PathBuf, Program)>, String> {
    let mut guard = STDLIB_CACHE.lock().unwrap();
    if let Some(cached) = guard.as_ref() {
        return Ok(cached.clone());
    }
    let result = load_stdlib_nx_programs_uncached()?;
    *guard = Some(result.clone());
    Ok(result)
}

fn load_stdlib_nx_programs_uncached() -> Result<Vec<(PathBuf, Program)>, String> {
    let mut out = Vec::new();
    for path in list_stdlib_nx_paths()? {
        let src = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        let program = parser::parser()
            .parse(&src)
            .map_err(|e| format!("Failed to parse {}: {:?}", path.display(), e))?;
        out.push((path, program));
    }
    Ok(out)
}
