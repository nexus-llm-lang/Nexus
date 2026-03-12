pub mod builder;
pub mod compile;
pub mod execute;
pub mod fixture;
pub mod typecheck;

// Re-export convenience functions for concise test code.
pub use builder::TestRunner;
pub use compile::{compile, try_compile};
pub use execute::{exec, exec_should_trap, exec_with_stdlib};
pub use fixture::{read_fixture, TempDir};
pub use typecheck::{should_fail_parse, should_fail_typecheck, should_typecheck, typecheck_warnings};
