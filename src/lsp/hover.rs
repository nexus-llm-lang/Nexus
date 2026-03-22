use crate::lang::ast::{Expr, GlobalLet, Program, Stmt, TopLevel};
use crate::lang::lexer;
use crate::lang::typecheck::TypeEnv;
use crate::types::Spanned;

/// Find hover information at a byte offset.
///
/// Strategy: tokenise the source, find the identifier token spanning the offset,
/// then look up that name in the type environment.
pub fn hover_at(source: &str, offset: usize, program: &Program, env: &TypeEnv) -> Option<String> {
    // 1. Find the identifier at offset using the lexer
    let name = find_ident_at(source, offset)?;

    // 2. Look up in the type environment
    if let Some(scheme) = env.get(&name) {
        let ty_str = scheme.typ.to_string();
        let qual = if scheme.vars.is_empty() {
            String::new()
        } else {
            format!("<{}> ", scheme.vars.join(", "))
        };
        return Some(format!("```nexus\n{}{}: {}\n```", qual, name, ty_str));
    }

    // 3. Try type definitions
    if let Some(td) = env.get_type(&name) {
        let params = if td.type_params.is_empty() {
            String::new()
        } else {
            format!("<{}>", td.type_params.join(", "))
        };
        let fields: Vec<String> = td
            .fields
            .iter()
            .map(|(n, t)| format!("  {}: {}", n, t))
            .collect();
        return Some(format!(
            "```nexus\ntype {}{}\n{}\nend\n```",
            name,
            params,
            fields.join("\n")
        ));
    }

    // 4. Try enum definitions
    if let Some(ed) = env.get_enum(&name) {
        let params = if ed.type_params.is_empty() {
            String::new()
        } else {
            format!("<{}>", ed.type_params.join(", "))
        };
        let variants: Vec<String> = ed
            .variants
            .iter()
            .map(|v| {
                if v.fields.is_empty() {
                    format!("  | {}", v.name)
                } else {
                    let fs: Vec<String> = v
                        .fields
                        .iter()
                        .map(|(n, t)| match n {
                            Some(name) => format!("{}: {}", name, t),
                            None => t.to_string(),
                        })
                        .collect();
                    format!("  | {}({})", v.name, fs.join(", "))
                }
            })
            .collect();
        return Some(format!(
            "```nexus\nenum {}{}\n{}\nend\n```",
            name,
            params,
            variants.join("\n")
        ));
    }

    // 5. Try to find in the AST (local function/let definitions)
    find_definition_type_in_ast(program, &name)
}

fn find_ident_at(source: &str, offset: usize) -> Option<String> {
    let tokens = lexer::tokenize(source).ok()?;
    for tok in &tokens {
        if tok.span.contains(&offset) || (tok.span.start == offset && tok.span.end > offset) {
            if let lexer::TokenKind::Ident(name) = &tok.kind {
                return Some(name.clone());
            }
        }
    }
    // Also check if the offset is at the end of a token (cursor right after ident)
    if offset > 0 {
        for tok in &tokens {
            if tok.span.end == offset {
                if let lexer::TokenKind::Ident(name) = &tok.kind {
                    return Some(name.clone());
                }
            }
        }
    }
    None
}

fn find_definition_type_in_ast(program: &Program, name: &str) -> Option<String> {
    for def in &program.definitions {
        match &def.node {
            TopLevel::Let(gl) => {
                if gl.name == name {
                    return Some(format_global_let(gl));
                }
            }
            _ => {}
        }
    }
    None
}

fn format_global_let(gl: &GlobalLet) -> String {
    match &gl.value.node {
        Expr::Lambda {
            params, ret_type, ..
        } => {
            let params_str: Vec<String> = params
                .iter()
                .map(|p| format!("{}: {}", p.name, p.typ))
                .collect();
            format!(
                "```nexus\nfn {}({}) -> {}\n```",
                gl.name,
                params_str.join(", "),
                ret_type
            )
        }
        _ => {
            if let Some(ty) = &gl.typ {
                format!("```nexus\n{}: {}\n```", gl.name, ty)
            } else {
                format!("```nexus\nlet {}\n```", gl.name)
            }
        }
    }
}

/// Find the definition location (byte span) of a name in the program.
pub fn find_definition(
    program: &Program,
    source: &str,
    offset: usize,
) -> Option<std::ops::Range<usize>> {
    let name = find_ident_at(source, offset)?;

    for def in &program.definitions {
        match &def.node {
            TopLevel::Let(gl) if gl.name == name => return Some(def.span.clone()),
            TopLevel::TypeDef(td) if td.name == name => return Some(def.span.clone()),
            TopLevel::Enum(ed) => {
                if ed.name == name {
                    return Some(def.span.clone());
                }
                for v in &ed.variants {
                    if v.name == name {
                        return Some(def.span.clone());
                    }
                }
            }
            TopLevel::Exception(exc) if exc.name == name => return Some(def.span.clone()),
            TopLevel::Port(port) if port.name == name => return Some(def.span.clone()),
            TopLevel::Import(imp) => {
                if imp.alias.as_deref() == Some(&name)
                    || imp
                        .items
                        .iter()
                        .any(|item| item.name == name || item.alias.as_deref() == Some(&name))
                {
                    return Some(def.span.clone());
                }
            }
            _ => {}
        }
    }

    // Search in function bodies for local lets
    for def in &program.definitions {
        if let TopLevel::Let(gl) = &def.node {
            if let Expr::Lambda { body, .. } = &gl.value.node {
                if let Some(span) = find_let_in_stmts(body, &name) {
                    return Some(span);
                }
            }
        }
    }

    None
}

fn find_let_in_stmts(stmts: &[Spanned<Stmt>], name: &str) -> Option<std::ops::Range<usize>> {
    for stmt in stmts {
        match &stmt.node {
            Stmt::Let { name: n, value, .. } if n == name => {
                return Some(stmt.span.clone());
            }
            Stmt::Expr(Spanned {
                node:
                    Expr::If {
                        then_branch,
                        else_branch,
                        ..
                    },
                ..
            }) => {
                if let Some(span) = find_let_in_stmts(then_branch, name) {
                    return Some(span);
                }
                if let Some(else_stmts) = else_branch {
                    if let Some(span) = find_let_in_stmts(else_stmts, name) {
                        return Some(span);
                    }
                }
            }
            Stmt::Expr(Spanned {
                node: Expr::Match { cases, .. },
                ..
            }) => {
                for case in cases {
                    if let Some(span) = find_let_in_stmts(&case.body, name) {
                        return Some(span);
                    }
                }
            }
            _ => {}
        }
    }
    None
}
