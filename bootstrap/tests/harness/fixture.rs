/// Read a test fixture. Paths are anchored to the repo root by
/// `ensure_repo_root()`, which chdirs out of `bootstrap/` during tests.
pub fn read_fixture(name: &str) -> String {
    super::ensure_repo_root();
    let path = format!("bootstrap/tests/fixtures/{}", name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} should exist: {}", path, e))
}

/// Read a nxc test fixture from `bootstrap/tests/fixtures/nxc/`.
pub fn read_nxc_fixture(name: &str) -> String {
    super::ensure_repo_root();
    let path = format!("bootstrap/tests/fixtures/nxc/{}", name);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("fixture {} should exist: {}", path, e))
}

/// RAII guard for temporary directories — cleans up on drop.
pub struct TempDir {
    path: String,
}

impl TempDir {
    pub fn new(label: &str) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        Self {
            path: format!("/tmp/nexus_test_{}_{}", label, nanos),
        }
    }

    pub fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
