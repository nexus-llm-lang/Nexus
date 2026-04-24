pub mod builder;
pub mod compile;
pub mod execute;
pub mod fixture;
pub mod typecheck;

// Re-export convenience functions for concise test code.
pub use builder::TestRunner;
pub use compile::{compile, try_compile};
pub use execute::{
    exec, exec_should_trap, exec_with_stdlib, exec_with_stdlib_caps,
    exec_with_stdlib_caps_should_trap, exec_with_stdlib_envs, exec_with_stdlib_should_trap,
};
pub use fixture::{read_fixture, TempDir};
pub use typecheck::{
    parse_and_check, should_fail_parse, should_fail_typecheck, should_typecheck, typecheck_warnings,
};

// Cargo runs integration tests with CWD = CARGO_MANIFEST_DIR (bootstrap/), but
// the compiler resolves `nxlib/stdlib/*` and `src/*.nx` imports relative to CWD.
// Chdir once to the repo root so those paths resolve the same way they do when
// the nexus CLI is invoked from the project root.
pub fn ensure_repo_root() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .expect("bootstrap/ manifest has a parent");
        std::env::set_current_dir(repo_root).expect("failed to chdir to repo root for tests");
    });
}
