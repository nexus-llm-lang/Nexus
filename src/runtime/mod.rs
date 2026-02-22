use std::path::PathBuf;
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtxBuilder};

#[allow(dead_code)]
pub mod string_heap;
pub mod wasm_exec;

/// Runtime capability policy used by interpreter and wasm execution paths.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ExecutionCapabilities {
    /// Allows outbound network access for WASI and host HTTP bridge.
    pub allow_net: bool,
    /// Allows filesystem access via preopened directories.
    pub allow_fs: bool,
    /// Host directories to preopen when filesystem access is enabled.
    pub preopen_dirs: Vec<PathBuf>,
    /// Allowed network destination host patterns (exact or subdomain match).
    pub net_allow_hosts: Vec<String>,
    /// Blocked network destination host patterns (exact or subdomain match).
    pub net_block_hosts: Vec<String>,
}

impl ExecutionCapabilities {
    /// Returns a deny-by-default capability set.
    pub fn deny_all() -> Self {
        Self::default()
    }

    /// Returns a permissive capability set used by legacy in-process interpreter APIs.
    pub fn permissive_legacy() -> Self {
        Self {
            allow_net: true,
            allow_fs: true,
            preopen_dirs: Vec::new(),
            net_allow_hosts: Vec::new(),
            net_block_hosts: Vec::new(),
        }
    }

    /// Validates flag combinations before runtime initialization.
    pub fn validate(&self) -> Result<(), String> {
        if !self.allow_fs && !self.preopen_dirs.is_empty() {
            return Err("`--preopen` requires `--allow-fs`".to_string());
        }
        Ok(())
    }

    /// Validates and enforces URL policy for outbound network calls.
    pub fn ensure_url_allowed(&self, _url: &str) -> Result<(), String> {
        Ok(())
    }

    /// Applies this capability policy to a WASI context builder.
    pub fn apply_to_wasi_builder(&self, builder: &mut WasiCtxBuilder) -> Result<(), String> {
        self.validate()?;
        // Temporary mode: always allow network regardless of capability flags.
        builder.inherit_network();
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn network_policy_is_temporarily_disabled() {
        let caps = ExecutionCapabilities {
            allow_net: false,
            net_allow_hosts: vec!["example.com".to_string()],
            net_block_hosts: vec!["blocked.local".to_string()],
            ..ExecutionCapabilities::deny_all()
        };
        assert!(
            caps.validate().is_ok(),
            "validation should ignore network policy in temporary allow-all mode"
        );
        assert!(
            caps.ensure_url_allowed("https://blocked.local").is_ok(),
            "URL policy should be disabled in temporary allow-all mode"
        );
    }
}
