use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use super::ast::Program;
use super::package::{PackageResolver, STD_PACKAGE_ROOT, WIT_NAMESPACE};
use super::parser;

pub const STDLIB_DIR: &str = STD_PACKAGE_ROOT;

static DEFAULT_RESOLVER: LazyLock<PackageResolver> =
    LazyLock::new(PackageResolver::with_default_std);

pub fn default_resolver() -> &'static PackageResolver {
    &DEFAULT_RESOLVER
}

/// Resolves a `pkg:path` import to its physical filesystem location.
/// Bare paths (no `:`) are treated as already-resolved file paths.
pub fn resolve_import_path(path: &str) -> String {
    DEFAULT_RESOLVER.resolve_import_path(path)
}

/// Resolves a WASM import module name to a physical file path on disk.
/// Used by the runtime and test harness to find WASM dependency files.
/// All `nexus:<pkg>/...` WIT names map to the bundled stdlib WASM as long
/// as `<pkg>` is registered with the default resolver.
pub fn resolve_import_to_file(module_name: &str) -> String {
    if let Some(rest) = module_name.strip_prefix(&format!("{}:", WIT_NAMESPACE)) {
        if let Some((pkg, _)) = rest.split_once('/') {
            if let Some(root) = DEFAULT_RESOLVER.package_root(pkg) {
                return format!("{}/stdlib.wasm", root.root.display());
            }
        }
    }
    module_name.to_string()
}

/// Resolves an `import external "<path>"` directive to the WIT module name
/// that subsequent `external` declarations should bind to.
///
/// - `pkg:iface` (e.g. `std:string-ops`)         → `nexus:pkg/iface`
/// - `nexus:foo/bar` (already a WIT name)        → returned unchanged
/// - any other path                              → returned unchanged (legacy)
pub fn resolve_external_wit_module(path: &str) -> String {
    if let Some(wit) = DEFAULT_RESOLVER.wit_name_for(path) {
        return wit;
    }
    path.to_string()
}

/// Returns true when `module_name` belongs to a registered package's WIT
/// namespace (e.g. `nexus:std/...`).
pub fn is_package_wit_module(module_name: &str) -> bool {
    DEFAULT_RESOLVER
        .iter_packages()
        .any(|(pkg, _)| module_name.starts_with(&format!("{}:{}/", WIT_NAMESPACE, pkg)))
}

/// Cached result of parsing every registered package's `.nx` files.
static PACKAGE_PROGRAMS_CACHE: LazyLock<Mutex<Option<Vec<(PathBuf, Program)>>>> =
    LazyLock::new(|| Mutex::new(None));

/// List `.nx` files in `dir` in lexical order. Used to enumerate a package's
/// modules for typecheck/HIR pre-loading.
pub fn list_nx_paths(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries =
        fs::read_dir(dir).map_err(|e| format!("Failed to read {}: {}", dir.display(), e))?;
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read {} entry: {}", dir.display(), e))?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("nx") {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

/// Parses every `.nx` file in every registered package and returns
/// `(path, Program)` pairs. Results are cached after the first successful call.
pub fn load_package_programs() -> Result<Vec<(PathBuf, Program)>, String> {
    let mut guard = PACKAGE_PROGRAMS_CACHE.lock().unwrap();
    if let Some(cached) = guard.as_ref() {
        return Ok(cached.clone());
    }
    let mut out = Vec::new();
    for (_pkg, root) in DEFAULT_RESOLVER.iter_packages() {
        for path in list_nx_paths(&root.root)? {
            let src = fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            let program = parser::parser()
                .parse(&src)
                .map_err(|e| format!("Failed to parse {}: {:?}", path.display(), e))?;
            out.push((path, program));
        }
    }
    *guard = Some(out.clone());
    Ok(out)
}
