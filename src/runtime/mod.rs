use crate::constants::{Permission, NEXUS_CAPABILITIES_SECTION, WASI_SNAPSHOT_MODULE};
use crate::types::Type;
use std::path::PathBuf;
use wasmtime::Linker;
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

pub mod backtrace;
pub mod net_host;
pub mod string_heap;
pub mod wasm_exec;

/// Runtime capability policy used by wasm execution paths.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionCapabilities {
    /// Allows outbound network access for WASI and host HTTP bridge.
    pub allow_net: bool,
    /// Allows filesystem access via preopened directories.
    pub allow_fs: bool,
    /// Allows console I/O (print, println).
    pub allow_console: bool,
    /// Allows random number generation.
    pub allow_random: bool,
    /// Allows clock/time operations.
    pub allow_clock: bool,
    /// Allows process operations (exit, etc.).
    pub allow_proc: bool,
    /// Allows environment variable access.
    pub allow_env: bool,
    /// Host directories to preopen when filesystem access is enabled.
    pub preopen_dirs: Vec<PathBuf>,
}

impl ExecutionCapabilities {
    /// Returns a deny-by-default capability set.
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// Returns a capability set with all permissions enabled.
    pub fn allow_all() -> Self {
        Self {
            allow_net: true,
            allow_fs: true,
            allow_console: true,
            allow_random: true,
            allow_clock: true,
            allow_proc: true,
            allow_env: true,
            preopen_dirs: Vec::new(),
        }
    }

    /// Check if a given permission is allowed.
    pub fn is_allowed(&self, perm: Permission) -> bool {
        match perm {
            Permission::Fs => self.allow_fs,
            Permission::Net => self.allow_net,
            Permission::Console => self.allow_console,
            Permission::Random => self.allow_random,
            Permission::Clock => self.allow_clock,
            Permission::Proc => self.allow_proc,
            Permission::Env => self.allow_env,
        }
    }

    /// Validates flag combinations before runtime initialization.
    pub fn validate(&self) -> Result<(), String> {
        if !self.allow_fs && !self.preopen_dirs.is_empty() {
            return Err("`--preopen` requires `--allow-fs`".to_string());
        }
        Ok(())
    }

    /// Validates that the program's `main` require clause is satisfied by this policy.
    /// Takes the `requires` type from the main function's AST node.
    pub fn validate_program_requires(&self, requires: &Type) -> Result<(), String> {
        let items = match requires {
            Type::Unit => return Ok(()),
            Type::Row(items, _) => items,
            _ => return Ok(()),
        };

        let mut missing: Vec<Permission> = Vec::new();
        for item in items {
            if let Type::UserDefined(name, args) = item {
                if args.is_empty() {
                    if let Some(perm) = Permission::from_perm_name(name) {
                        if !self.is_allowed(perm) {
                            missing.push(perm);
                        }
                    }
                }
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            let perm_names: Vec<&str> = missing.iter().map(|p| p.perm_name()).collect();
            let flags: Vec<&str> = missing.iter().map(|p| p.flag()).collect();
            Err(format!(
                "main requires {{{}}} but {{{}}} not specified",
                perm_names.join(", "),
                flags.join(", "),
            ))
        }
    }

    /// Validates that the WASM module's declared capabilities are satisfied by this policy.
    /// Returns a list of missing capability names if validation fails.
    pub fn validate_wasm_capabilities(&self, wasm_bytes: &[u8]) -> Result<(), String> {
        let required = parse_nexus_capabilities(wasm_bytes);
        let mut missing: Vec<Permission> = Vec::new();
        for cap in &required {
            if let Some(perm) = Permission::from_cap_name(cap) {
                if !self.is_allowed(perm) {
                    missing.push(perm);
                }
            }
        }
        if missing.is_empty() {
            Ok(())
        } else {
            let cap_names: Vec<&str> = missing.iter().map(|p| p.cap_name()).collect();
            let flags: Vec<&str> = missing.iter().map(|p| p.flag()).collect();
            Err(format!(
                "Missing capabilities: {}. Add {} to enable.",
                cap_names.join(", "),
                flags.join(" ")
            ))
        }
    }

    /// Applies this capability policy to a WASI context builder.
    pub fn apply_to_wasi_builder(&self, builder: &mut WasiCtxBuilder) -> Result<(), String> {
        self.validate()?;

        if self.allow_console {
            builder.inherit_stdio();
        }
        if self.allow_net {
            builder.inherit_network();
        }
        if !self.allow_fs {
            return Ok(());
        }

        if self.preopen_dirs.is_empty() {
            builder
                .preopened_dir(".", "/", DirPerms::all(), FilePerms::all())
                .map_err(|e| format!("failed to preopen default root dir '.': {}", e))?;
            return Ok(());
        }

        for dir in &self.preopen_dirs {
            let canonical = dir.canonicalize().map_err(|e| {
                format!(
                    "failed to canonicalize preopen dir '{}': {}",
                    dir.display(),
                    e
                )
            })?;
            let guest_path = canonical.to_string_lossy().to_string();
            builder
                .preopened_dir(&canonical, &guest_path, DirPerms::all(), FilePerms::all())
                .map_err(|e| {
                    format!(
                        "failed to preopen dir '{}' as '{}': {}",
                        canonical.display(),
                        guest_path,
                        e
                    )
                })?;
        }
        Ok(())
    }

    /// Overrides WASI P1 linker functions with trapping stubs for denied capabilities.
    /// Must be called after `wasmtime_wasi::p1::add_to_linker_sync`.
    pub fn enforce_denied_wasi_functions(
        &self,
        linker: &mut Linker<WasiP1Ctx>,
    ) -> Result<(), String> {
        linker.allow_shadowing(true);

        if !self.allow_clock {
            linker
                .func_wrap(
                    WASI_SNAPSHOT_MODULE,
                    "clock_res_get",
                    |_id: i32, _result_ptr: i32| -> i32 {
                        eprintln!("Clock access denied: --allow-clock not specified");
                        76 // ENOSYS
                    },
                )
                .map_err(|e| format!("failed to override clock_res_get: {e}"))?;
            linker
                .func_wrap(
                    WASI_SNAPSHOT_MODULE,
                    "clock_time_get",
                    |_id: i32, _precision: i64, _result_ptr: i32| -> i32 {
                        eprintln!("Clock access denied: --allow-clock not specified");
                        76 // ENOSYS
                    },
                )
                .map_err(|e| format!("failed to override clock_time_get: {e}"))?;
        }

        if !self.allow_random {
            linker
                .func_wrap(
                    WASI_SNAPSHOT_MODULE,
                    "random_get",
                    |_buf: i32, _buf_len: i32| -> i32 {
                        eprintln!("Random access denied: --allow-random not specified");
                        76 // ENOSYS
                    },
                )
                .map_err(|e| format!("failed to override random_get: {e}"))?;
        }

        linker.allow_shadowing(false);
        Ok(())
    }
}

/// Parses the `nexus:capabilities` custom section from WASM bytes.
/// Returns a list of port names (e.g., ["Fs", "Net"]).
pub fn parse_nexus_capabilities(wasm_bytes: &[u8]) -> Vec<String> {
    // Custom section format: newline-separated port names
    // We scan for the custom section by parsing WASM sections
    use wasmparser::{Parser, Payload};
    let parser = Parser::new(0);
    for payload in parser.parse_all(wasm_bytes) {
        if let Ok(Payload::CustomSection(reader)) = payload {
            if reader.name() == NEXUS_CAPABILITIES_SECTION {
                let data = reader.data();
                if data.is_empty() {
                    return vec![];
                }
                return std::str::from_utf8(data)
                    .unwrap_or("")
                    .split('\n')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .collect();
            }
        }
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_program_requires_unit_is_ok() {
        let caps = ExecutionCapabilities::deny_all();
        assert!(caps.validate_program_requires(&Type::Unit).is_ok());
    }

    #[test]
    fn validate_program_requires_missing_net() {
        let caps = ExecutionCapabilities::deny_all();
        let requires = Type::Row(
            vec![Type::UserDefined(
                Permission::Net.perm_name().to_string(),
                vec![],
            )],
            None,
        );
        let err = caps
            .validate_program_requires(&requires)
            .expect_err("should reject missing --allow-net");
        assert!(err.contains("--allow-net"), "unexpected error: {}", err);
    }

    #[test]
    fn validate_program_requires_net_allowed() {
        let caps = ExecutionCapabilities {
            allow_net: true,
            ..ExecutionCapabilities::deny_all()
        };
        let requires = Type::Row(
            vec![Type::UserDefined(
                Permission::Net.perm_name().to_string(),
                vec![],
            )],
            None,
        );
        assert!(caps.validate_program_requires(&requires).is_ok());
    }

    #[test]
    fn validate_program_requires_partial_missing() {
        let caps = ExecutionCapabilities {
            allow_net: true,
            allow_console: false,
            ..ExecutionCapabilities::deny_all()
        };
        let requires = Type::Row(
            vec![
                Type::UserDefined(Permission::Net.perm_name().to_string(), vec![]),
                Type::UserDefined(Permission::Console.perm_name().to_string(), vec![]),
            ],
            None,
        );
        let err = caps
            .validate_program_requires(&requires)
            .expect_err("should reject missing --allow-console");
        assert!(err.contains("--allow-console"), "unexpected error: {}", err);
        assert!(
            !err.contains("--allow-net"),
            "should not mention --allow-net: {}",
            err
        );
    }
}
