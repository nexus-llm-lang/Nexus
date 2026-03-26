mod binary;
mod emit;
mod error;
mod function;
mod layout;
mod module;
mod stmt;
mod string;

pub use error::{CodegenError, CompileError, CompileMetrics};
pub use module::compile_lir_to_wasm;

use std::borrow::Cow;

use wasm_encoder::{CustomSection, Module, ValType};

use super::passes::hir_build::{build_hir, HirBuildError};
use super::passes::lir_lower::{lower_mir_to_lir, LirLowerError};
use super::passes::lir_opt::optimize_lir;
use crate::constants::{Permission, ENTRYPOINT, NEXUS_CAPABILITIES_SECTION};
use crate::lang::ast::Program;
use crate::types::Type;

const STRING_DATA_BASE: u32 = 16;
const OBJECT_HEAP_GLOBAL_INDEX: u32 = 0;
const CONC_MODULE: &str = "nexus:runtime/conc";
const CONC_SPAWN_NAME: &str = "__nx_conc_spawn";
const CONC_JOIN_NAME: &str = "__nx_conc_join";
const CONC_TASK_PREFIX: &str = "__conc_";
const ALLOCATE_WASM_NAME: &str = "allocate";

#[derive(Debug, Clone, Copy)]
struct LocalInfo {
    index: u32,
    val_type: ValType,
}

#[derive(Debug, Clone, Copy)]
struct FunctionTemps {
    packed_tmp_i64: u32,
    object_ptr_i32: u32,
    concat_lhs_packed_i64: u32,
    concat_rhs_packed_i64: u32,
    concat_lhs_ptr_i32: u32,
    concat_lhs_len_i32: u32,
    concat_rhs_ptr_i32: u32,
    concat_rhs_len_i32: u32,
    concat_out_ptr_i32: u32,
    concat_out_len_i32: u32,
    concat_idx_i32: u32,
    /// Temp for closure pointer during call_indirect
    closure_ptr_i64: u32,
    /// Temp for table index loaded from closure during call_indirect
    closure_table_idx_i64: u32,
}

/// Compiles a parsed Nexus program through HIR -> MIR -> LIR -> WASM pipeline,
/// returning per-pass timing metrics alongside the WASM bytes.
#[tracing::instrument(skip_all, name = "compile_program_to_wasm")]
pub fn compile_program_to_wasm_with_metrics(
    program: &Program,
) -> Result<(Vec<u8>, CompileMetrics), CompileError> {
    use std::time::Instant;

    validate_main_returns_unit(program)?;
    let caps = extract_main_require_ports_from_ast(program);

    let t = Instant::now();
    let mir = build_hir(program).map_err(CompileError::HirBuild)?;
    let hir_build = t.elapsed();

    let t = Instant::now();
    let mut lir = lower_mir_to_lir(&mir, &mir.enum_defs).map_err(CompileError::LirLower)?;
    let lir_lower = t.elapsed();

    let t = Instant::now();
    optimize_lir(&mut lir);
    let optimize = t.elapsed();

    let t = Instant::now();
    let mut wasm = compile_lir_to_wasm(&lir).map_err(CompileError::Codegen)?;
    let codegen = t.elapsed();

    if !caps.is_empty() {
        append_capabilities_section(&mut wasm, &caps);
    }

    let metrics = CompileMetrics {
        hir_build,
        lir_lower,
        optimize,
        codegen,
    };

    Ok((wasm, metrics))
}

/// Compiles a parsed Nexus program through HIR -> MIR -> LIR -> WASM pipeline.
pub fn compile_program_to_wasm(program: &Program) -> Result<Vec<u8>, CompileError> {
    compile_program_to_wasm_with_metrics(program).map(|(wasm, _)| wasm)
}

fn perm_to_capability(name: &str) -> Option<&'static str> {
    Permission::from_perm_name(name).map(|p| p.cap_name())
}

fn validate_main_returns_unit(program: &Program) -> Result<(), CompileError> {
    use crate::lang::ast::{Expr, TopLevel};
    for def in &program.definitions {
        if let TopLevel::Let(gl) = &def.node {
            if gl.name == ENTRYPOINT {
                if let Expr::Lambda {
                    ret_type, params, ..
                } = &gl.value.node
                {
                    if *ret_type != Type::Unit {
                        return Err(CompileError::MainSignature(format!(
                            "main must return unit, got '{}'",
                            ret_type
                        )));
                    }
                    if params.len() > 1 {
                        return Err(CompileError::MainSignature(
                            "main must have 0 or 1 parameter (args: [string])".into(),
                        ));
                    }
                    if params.len() == 1 {
                        let param_type = &params[0].typ;
                        if *param_type != Type::List(Box::new(Type::String)) {
                            return Err(CompileError::MainSignature(format!(
                                "main parameter must be [string], got '{}'",
                                param_type
                            )));
                        }
                    }
                }
                return Ok(());
            }
        }
    }
    Ok(())
}

fn extract_main_require_ports_from_ast(program: &Program) -> Vec<String> {
    use crate::lang::ast::{Expr, TopLevel};
    for def in &program.definitions {
        if let TopLevel::Let(gl) = &def.node {
            if gl.name == ENTRYPOINT {
                if let Expr::Lambda {
                    requires, params, ..
                } = &gl.value.node
                {
                    let mut caps: Vec<String> = match requires {
                        Type::Row(reqs, _) => reqs
                            .iter()
                            .filter_map(|r| match r {
                                Type::UserDefined(name, args) if args.is_empty() => {
                                    perm_to_capability(name).map(|s| s.to_string())
                                }
                                _ => None,
                            })
                            .collect(),
                        _ => vec![],
                    };
                    if !params.is_empty() && !caps.contains(&"Proc".to_string()) {
                        caps.push("Proc".to_string());
                    }
                    return caps;
                }
            }
        }
    }
    vec![]
}

fn append_capabilities_section(wasm: &mut Vec<u8>, caps: &[String]) {
    let payload = caps.join("\n");
    let section = CustomSection {
        name: std::borrow::Cow::Borrowed(NEXUS_CAPABILITIES_SECTION),
        data: Cow::Borrowed(payload.as_bytes()),
    };
    let mut tmp = Module::new();
    tmp.section(&section);
    let encoded = tmp.finish();
    wasm.extend_from_slice(&encoded[8..]);
}
