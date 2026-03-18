/// Read a test fixture from `tests/fixtures/`.
pub fn read_fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/{}", name))
        .unwrap_or_else(|e| panic!("fixture tests/fixtures/{} should exist: {}", name, e))
}

/// Read a nxc test fixture from `tests/fixtures/nxc/`.
pub fn read_nxc_fixture(name: &str) -> String {
    std::fs::read_to_string(format!("tests/fixtures/nxc/{}", name))
        .unwrap_or_else(|e| panic!("fixture tests/fixtures/nxc/{} should exist: {}", name, e))
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
