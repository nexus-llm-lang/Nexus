use std::process::ExitCode;

fn main() -> ExitCode {
    match nexus::lsp::serve() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("LSP server error: {}", e);
            ExitCode::from(1)
        }
    }
}
