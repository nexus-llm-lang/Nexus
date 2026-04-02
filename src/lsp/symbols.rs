use super::position::LineIndex;
use crate::lang::ast::{Program, TopLevel};
use crate::types::Type;

/// Extract document symbols from a parsed program.
pub fn extract(program: &Program, idx: &LineIndex) -> Vec<lsp_types::DocumentSymbol> {
    let mut symbols = Vec::new();

    for def in &program.definitions {
        match &def.node {
            TopLevel::Let(gl) => {
                let kind = match &gl.value.node {
                    crate::lang::ast::Expr::Lambda { .. } => lsp_types::SymbolKind::FUNCTION,
                    _ => lsp_types::SymbolKind::CONSTANT,
                };
                let detail = gl.typ.as_ref().map(format_type);
                #[allow(deprecated)]
                symbols.push(lsp_types::DocumentSymbol {
                    name: gl.name.clone(),
                    detail,
                    kind,
                    tags: None,
                    deprecated: None,
                    range: idx.span_to_range(&def.span),
                    selection_range: idx.span_to_range(&def.span),
                    children: None,
                });
            }
            TopLevel::TypeDef(td) => {
                let detail = if td.type_params.is_empty() {
                    None
                } else {
                    Some(format!("<{}>", td.type_params.join(", ")))
                };
                #[allow(deprecated)]
                symbols.push(lsp_types::DocumentSymbol {
                    name: td.name.clone(),
                    detail,
                    kind: lsp_types::SymbolKind::STRUCT,
                    tags: None,
                    deprecated: None,
                    range: idx.span_to_range(&def.span),
                    selection_range: idx.span_to_range(&def.span),
                    children: None,
                });
            }
            TopLevel::Enum(ed) => {
                let children: Vec<_> = ed
                    .variants
                    .iter()
                    .map(|v| {
                        #[allow(deprecated)]
                        lsp_types::DocumentSymbol {
                            name: v.name.clone(),
                            detail: None,
                            kind: lsp_types::SymbolKind::ENUM_MEMBER,
                            tags: None,
                            deprecated: None,
                            range: idx.span_to_range(&def.span),
                            selection_range: idx.span_to_range(&def.span),
                            children: None,
                        }
                    })
                    .collect();
                #[allow(deprecated)]
                symbols.push(lsp_types::DocumentSymbol {
                    name: ed.name.clone(),
                    detail: None,
                    kind: lsp_types::SymbolKind::ENUM,
                    tags: None,
                    deprecated: None,
                    range: idx.span_to_range(&def.span),
                    selection_range: idx.span_to_range(&def.span),
                    children: if children.is_empty() {
                        None
                    } else {
                        Some(children)
                    },
                });
            }
            TopLevel::Exception(exc) => {
                #[allow(deprecated)]
                symbols.push(lsp_types::DocumentSymbol {
                    name: exc.name.clone(),
                    detail: None,
                    kind: lsp_types::SymbolKind::EVENT,
                    tags: None,
                    deprecated: None,
                    range: idx.span_to_range(&def.span),
                    selection_range: idx.span_to_range(&def.span),
                    children: None,
                });
            }
            TopLevel::Port(port) => {
                let children: Vec<_> = port
                    .functions
                    .iter()
                    .map(|fsig| {
                        let detail = Some(format_arrow_sig(&fsig.params, &fsig.ret_type));
                        #[allow(deprecated)]
                        lsp_types::DocumentSymbol {
                            name: fsig.name.clone(),
                            detail,
                            kind: lsp_types::SymbolKind::METHOD,
                            tags: None,
                            deprecated: None,
                            range: idx.span_to_range(&def.span),
                            selection_range: idx.span_to_range(&def.span),
                            children: None,
                        }
                    })
                    .collect();
                #[allow(deprecated)]
                symbols.push(lsp_types::DocumentSymbol {
                    name: port.name.clone(),
                    detail: None,
                    kind: lsp_types::SymbolKind::INTERFACE,
                    tags: None,
                    deprecated: None,
                    range: idx.span_to_range(&def.span),
                    selection_range: idx.span_to_range(&def.span),
                    children: if children.is_empty() {
                        None
                    } else {
                        Some(children)
                    },
                });
            }
            TopLevel::ExceptionGroup(_) => {}
            TopLevel::Import(_) => {}
        }
    }

    symbols
}

fn format_type(ty: &Type) -> String {
    ty.to_string()
}

fn format_arrow_sig(params: &[crate::lang::ast::Param], ret: &Type) -> String {
    let params_str: Vec<String> = params
        .iter()
        .map(|p| format!("{}: {}", p.name, p.typ))
        .collect();
    format!("({}) -> {}", params_str.join(", "), ret)
}
