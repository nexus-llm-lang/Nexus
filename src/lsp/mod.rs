//! Nexus Language Server Protocol implementation.
//!
//! - `nexus lsp` — stdio-based LSP server for editors
//! - `nexus check --format json` — one-shot structured diagnostics for CLI / LLM

mod analysis;
mod hover;
mod position;
mod server;
mod symbols;

pub use analysis::{analyze, CheckDiagnostic, CheckResult, CheckSymbol};
pub use position::LineIndex;

use std::path::Path;

/// Start the LSP server over stdio.
pub fn serve() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    server::run()
}

/// One-shot analysis returning structured JSON.
pub fn check_json(path: &Path) -> Result<String, std::io::Error> {
    let source = std::fs::read_to_string(path)?;
    let filename = path.display().to_string();
    let result = analyze(&filename, &source);
    Ok(serde_json::to_string_pretty(&result.check).unwrap())
}

/// Find the project root by walking up from `start` looking for `.git`.
pub fn find_project_root(start: &Path) -> Option<std::path::PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?
    } else {
        start
    };
    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        dir = dir.parent()?;
    }
}
