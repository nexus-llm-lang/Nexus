//! CLI definitions, argument parsing, and source loading utilities.

use clap::{Parser, Subcommand};
use std::fs;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ExplainCapabilities {
    /// Show capability names (default).
    Yes,
    /// Suppress capability output.
    None,
    /// Show wasmtime run flags needed for this binary.
    Wasmtime,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ExplainCapabilitiesFormat {
    /// Human-readable text (default).
    Text,
    /// Machine-readable JSON.
    Json,
}

#[derive(Debug, Parser)]
#[command(name = "nexus")]
#[command(about = "Nexus language CLI")]
pub struct Cli {
    /// Enable verbose structured timing output to stderr
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Parse, typecheck, and build a WASM Component artifact.
    /// If no file is passed and stdin is piped, reads script from stdin.
    Build {
        /// Nexus source file path. Use '-' to read from stdin.
        input: Option<PathBuf>,
        /// Output path (default: main.wasm).
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Override `wasm-merge` executable path for dependency bundling.
        #[arg(long, value_name = "PATH")]
        wasm_merge: Option<PathBuf>,
        /// Show capability information after build.
        #[arg(long, value_enum, default_value_t = ExplainCapabilities::Yes)]
        explain_capabilities: ExplainCapabilities,
        /// Output format for capability information.
        #[arg(long, value_enum, default_value_t = ExplainCapabilitiesFormat::Text)]
        explain_capabilities_format: ExplainCapabilitiesFormat,
    },
}

pub struct LoadedSource {
    pub display_name: String,
    pub source: String,
}

pub fn load_source(input: Option<PathBuf>) -> Result<LoadedSource, String> {
    if let Some(path) = input {
        if path == Path::new("-") {
            return read_source_from_stdin();
        }
        let source = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
        return Ok(LoadedSource {
            display_name: path.display().to_string(),
            source,
        });
    }

    if io::stdin().is_terminal() {
        return Err("No input provided (pass file path or pipe script to stdin).".to_string());
    }
    read_source_from_stdin()
}

fn read_source_from_stdin() -> Result<LoadedSource, String> {
    let mut buf = String::new();
    io::stdin()
        .read_to_string(&mut buf)
        .map_err(|e| format!("Failed to read stdin: {}", e))?;
    Ok(LoadedSource {
        display_name: "<stdin>".to_string(),
        source: buf,
    })
}

pub fn strip_shebang(source: String) -> String {
    if source.starts_with("#!") {
        if let Some(pos) = source.find('\n') {
            let mut out = String::with_capacity(source.len() - pos);
            out.push('\n');
            out.push_str(&source[pos + 1..]);
            out
        } else {
            String::new()
        }
    } else {
        source
    }
}

pub fn default_wasm_output_path() -> PathBuf {
    PathBuf::from("main.wasm")
}
