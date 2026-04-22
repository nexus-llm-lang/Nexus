//! Build artifact reporting and capability inspection.

use crate::cli::{ExplainCapabilities, ExplainCapabilitiesFormat};
use nexus::constants::Permission;

#[cfg(test)]
pub fn is_component_wasm(wasm: &[u8]) -> bool {
    wasmparser::Parser::is_component(wasm)
}

/// Maps a capability name to the wasmtime CLI flags required.
pub fn capability_wasmtime_flags(cap: &str) -> Vec<&'static str> {
    match Permission::from_cap_name(cap) {
        Some(Permission::Net) => vec!["--wasi", "http", "--wasi", "inherit-network"],
        Some(Permission::Fs) => vec!["--dir", "."],
        // Console, Random, Clock, Proc are provided by the wasmtime CLI by default.
        // At the API level, PermConsole explicitly maps to WasiCtxBuilder::inherit_stdio(),
        // while Clock and Random are inherent to the default Wasmtime WasiCtx.
        _ => vec![],
    }
}

pub fn print_build_result(
    output_name: &str,
    caps: &[String],
    explain: &ExplainCapabilities,
    format: &ExplainCapabilitiesFormat,
) {
    match format {
        ExplainCapabilitiesFormat::Text => {
            print_build_result_text(output_name, caps, explain);
        }
        ExplainCapabilitiesFormat::Json => {
            print_build_result_json(output_name, caps, explain);
        }
    }
}

fn print_build_result_text(output_name: &str, caps: &[String], explain: &ExplainCapabilities) {
    eprintln!("Built {output_name}");
    match explain {
        ExplainCapabilities::None => {}
        ExplainCapabilities::Yes => {
            if !caps.is_empty() {
                eprintln!("Capabilities: {}", caps.join(", "));
            }
        }
        ExplainCapabilities::Wasmtime => {
            if !caps.is_empty() {
                eprintln!("Capabilities: {}", caps.join(", "));
            }
            let mut flags: Vec<&str> = Vec::new();
            for cap in caps {
                flags.extend(capability_wasmtime_flags(cap));
            }
            flags.dedup();
            let mut cmd_parts = vec!["wasmtime", "run"];
            cmd_parts.extend(&flags);
            cmd_parts.push(output_name);
            eprintln!("Run: {}", cmd_parts.join(" "));
        }
    }
}

fn print_build_result_json(output_name: &str, caps: &[String], explain: &ExplainCapabilities) {
    match explain {
        ExplainCapabilities::None => {
            eprintln!("{{\"file\":\"{output_name}\"}}");
        }
        ExplainCapabilities::Yes => {
            let caps_json: Vec<String> = caps.iter().map(|c| format!("\"{c}\"")).collect();
            eprintln!(
                "{{\"file\":\"{output_name}\",\"capabilities\":[{}]}}",
                caps_json.join(",")
            );
        }
        ExplainCapabilities::Wasmtime => {
            let caps_json: Vec<String> = caps.iter().map(|c| format!("\"{c}\"")).collect();
            let mut flags: Vec<&str> = Vec::new();
            for cap in caps {
                flags.extend(capability_wasmtime_flags(cap));
            }
            flags.dedup();
            let mut cmd_parts = vec!["wasmtime", "run"];
            cmd_parts.extend(&flags);
            cmd_parts.push(output_name);
            let flags_json: Vec<String> = flags.iter().map(|f| format!("\"{f}\"")).collect();
            eprintln!(
                "{{\"file\":\"{output_name}\",\"capabilities\":[{}],\"wasmtime\":{{\"command\":\"{}\",\"flags\":[{}]}}}}",
                caps_json.join(","),
                cmd_parts.join(" "),
                flags_json.join(",")
            );
        }
    }
}
