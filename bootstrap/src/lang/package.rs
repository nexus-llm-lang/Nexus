//! Package-based module resolution.
//!
//! Maps `pkg:path` import strings to physical files on disk and to
//! WASM-component-model WIT interface names.
//!
//! Example: `std:stdio` resolves to `nxlib/stdlib/stdio.nx` and to
//! the WIT name `nexus:std/stdio`.
//!
//! Bare paths (no `:`) are treated as filesystem paths and are returned
//! unchanged from `resolve_import_path`. They cannot be mapped to a WIT
//! name because they carry no package qualifier.

use std::collections::HashMap;
use std::path::PathBuf;

/// Fixed namespace for all Nexus packages in the WIT component-model.
pub const WIT_NAMESPACE: &str = "nexus";

/// Default name of the standard-library package.
pub const STD_PACKAGE: &str = "std";

/// Default root directory for the `std` package (relative to cwd).
pub const STD_PACKAGE_ROOT: &str = "nxlib/stdlib";

/// Source-file extension for Nexus modules.
pub const NX_EXT: &str = "nx";

#[derive(Debug, Clone)]
pub struct PackageRoot {
    pub root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct PackageResolver {
    packages: HashMap<String, PackageRoot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedImport {
    pub package: String,
    pub module_path: String,
    pub file_path: PathBuf,
    pub wit_name: String,
}

impl PackageResolver {
    pub fn new() -> Self {
        Self {
            packages: HashMap::new(),
        }
    }

    pub fn with_default_std() -> Self {
        let mut r = Self::new();
        r.register(STD_PACKAGE, STD_PACKAGE_ROOT);
        r
    }

    pub fn register(&mut self, name: &str, root: impl Into<PathBuf>) {
        self.packages
            .insert(name.to_string(), PackageRoot { root: root.into() });
    }

    pub fn package_root(&self, name: &str) -> Option<&PackageRoot> {
        self.packages.get(name)
    }

    /// Splits a `pkg:path` import into `(pkg, path)`.
    /// Returns `None` if the input has no `:` separator.
    pub fn split_qualified(import_path: &str) -> Option<(&str, &str)> {
        let (pkg, rest) = import_path.split_once(':')?;
        if pkg.is_empty() || rest.is_empty() {
            return None;
        }
        Some((pkg, rest))
    }

    /// Resolves a `pkg:path` import to a `ResolvedImport`.
    /// Returns `Err` for unqualified paths or unknown packages.
    pub fn resolve_qualified(&self, import_path: &str) -> Result<ResolvedImport, String> {
        let (pkg, module_path) = Self::split_qualified(import_path).ok_or_else(|| {
            format!(
                "import '{}' is not package-qualified (expected 'pkg:path')",
                import_path
            )
        })?;
        let root = self
            .package_root(pkg)
            .ok_or_else(|| format!("unknown package '{}' in import '{}'", pkg, import_path))?;
        let mut file_path = root.root.clone();
        let stripped = module_path
            .strip_suffix(&format!(".{}", NX_EXT))
            .unwrap_or(module_path);
        file_path.push(stripped);
        file_path.set_extension(NX_EXT);
        let wit_name = format!("{}:{}/{}", WIT_NAMESPACE, pkg, stripped);
        Ok(ResolvedImport {
            package: pkg.to_string(),
            module_path: stripped.to_string(),
            file_path,
            wit_name,
        })
    }

    /// Resolves any import path. Qualified paths flow through the package
    /// resolver; bare paths are returned unchanged so legacy file-relative
    /// imports continue to work during migration.
    pub fn resolve_import_path(&self, import_path: &str) -> String {
        match self.resolve_qualified(import_path) {
            Ok(resolved) => resolved.file_path.to_string_lossy().into_owned(),
            Err(_) => import_path.to_string(),
        }
    }

    /// Returns the WIT interface name for a qualified import, or `None`
    /// for bare paths and unknown packages.
    pub fn wit_name_for(&self, import_path: &str) -> Option<String> {
        self.resolve_qualified(import_path).ok().map(|r| r.wit_name)
    }

    pub fn iter_packages(&self) -> impl Iterator<Item = (&String, &PackageRoot)> {
        self.packages.iter()
    }
}

impl Default for PackageResolver {
    fn default() -> Self {
        Self::with_default_std()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn split_qualified_extracts_pkg_and_path() {
        assert_eq!(
            PackageResolver::split_qualified("std:stdio"),
            Some(("std", "stdio"))
        );
        assert_eq!(
            PackageResolver::split_qualified("std:sub/foo"),
            Some(("std", "sub/foo"))
        );
        assert_eq!(PackageResolver::split_qualified("stdlib/foo.nx"), None);
        assert_eq!(PackageResolver::split_qualified(":foo"), None);
        assert_eq!(PackageResolver::split_qualified("std:"), None);
    }

    #[test]
    fn resolve_qualified_default_std() {
        let r = PackageResolver::default();
        let resolved = r.resolve_qualified("std:stdio").unwrap();
        assert_eq!(resolved.package, "std");
        assert_eq!(resolved.module_path, "stdio");
        assert_eq!(resolved.file_path, Path::new("nxlib/stdlib/stdio.nx"));
        assert_eq!(resolved.wit_name, "nexus:std/stdio");
    }

    #[test]
    fn resolve_qualified_strips_explicit_extension() {
        let r = PackageResolver::default();
        let with_ext = r.resolve_qualified("std:stdio.nx").unwrap();
        let without_ext = r.resolve_qualified("std:stdio").unwrap();
        assert_eq!(with_ext.file_path, without_ext.file_path);
        assert_eq!(with_ext.wit_name, without_ext.wit_name);
    }

    #[test]
    fn resolve_qualified_supports_subpaths() {
        let r = PackageResolver::default();
        let resolved = r.resolve_qualified("std:sub/foo").unwrap();
        assert_eq!(resolved.file_path, Path::new("nxlib/stdlib/sub/foo.nx"));
        assert_eq!(resolved.wit_name, "nexus:std/sub/foo");
    }

    #[test]
    fn resolve_qualified_rejects_unknown_package() {
        let r = PackageResolver::default();
        let err = r.resolve_qualified("missing:foo").unwrap_err();
        assert!(err.contains("unknown package 'missing'"), "{}", err);
    }

    #[test]
    fn resolve_qualified_rejects_unqualified_path() {
        let r = PackageResolver::default();
        let err = r.resolve_qualified("stdlib/foo.nx").unwrap_err();
        assert!(err.contains("not package-qualified"), "{}", err);
    }

    #[test]
    fn resolve_import_path_passthrough_for_bare_paths() {
        let r = PackageResolver::default();
        assert_eq!(
            r.resolve_import_path("examples/hello.nx"),
            "examples/hello.nx"
        );
    }

    #[test]
    fn register_overrides_default_std_root() {
        let mut r = PackageResolver::new();
        r.register("std", "/custom/lib");
        let resolved = r.resolve_qualified("std:stdio").unwrap();
        assert_eq!(resolved.file_path, Path::new("/custom/lib/stdio.nx"));
    }

    #[test]
    fn register_supports_arbitrary_package_names() {
        let mut r = PackageResolver::with_default_std();
        r.register("mypkg", "vendor/mypkg");
        let resolved = r.resolve_qualified("mypkg:foo/bar").unwrap();
        assert_eq!(resolved.file_path, Path::new("vendor/mypkg/foo/bar.nx"));
        assert_eq!(resolved.wit_name, "nexus:mypkg/foo/bar");
    }

    #[test]
    fn wit_name_for_qualified() {
        let r = PackageResolver::default();
        assert_eq!(
            r.wit_name_for("std:str"),
            Some("nexus:std/str".to_string())
        );
        assert_eq!(r.wit_name_for("stdlib/foo.nx"), None);
    }
}
