use lsp_types::{CompletionItem, CompletionItemKind};

use crate::lang::typecheck::TypeEnv;

/// Nexus keywords
const KEYWORDS: &[&str] = &[
    "let", "fn", "do", "end", "return", "if", "then", "else", "match", "case",
    "type", "enum", "port", "import", "from", "export", "require", "throws",
    "raise", "try", "catch", "handler", "inject", "exception", "external",
    "while", "for", "task", "conc", "true", "false", "borrow",
];

/// Build completion items from keywords + type environment.
pub fn completions(env: Option<&TypeEnv>) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = KEYWORDS
        .iter()
        .map(|kw| CompletionItem {
            label: kw.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            ..Default::default()
        })
        .collect();

    if let Some(env) = env {
        // Variables / functions
        for (name, scheme) in &env.vars {
            if name.starts_with("__") {
                continue; // skip internal names
            }
            let kind = if matches!(&scheme.typ, crate::types::Type::Arrow(..)) {
                CompletionItemKind::FUNCTION
            } else {
                CompletionItemKind::VARIABLE
            };
            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(kind),
                detail: Some(scheme.typ.to_string()),
                ..Default::default()
            });
        }

        // Types
        for (name, td) in &env.types {
            let detail = if td.type_params.is_empty() {
                None
            } else {
                Some(format!("<{}>", td.type_params.join(", ")))
            };
            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::STRUCT),
                detail,
                ..Default::default()
            });
        }

        // Enums + constructors
        for (name, ed) in &env.enums {
            items.push(CompletionItem {
                label: name.clone(),
                kind: Some(CompletionItemKind::ENUM),
                ..Default::default()
            });
            for v in &ed.variants {
                items.push(CompletionItem {
                    label: v.name.clone(),
                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                    detail: Some(name.clone()),
                    ..Default::default()
                });
            }
        }

        // Module members
        for (mod_name, mod_env) in &env.modules {
            for (fname, scheme) in &mod_env.vars {
                items.push(CompletionItem {
                    label: format!("{}.{}", mod_name, fname),
                    kind: Some(CompletionItemKind::FUNCTION),
                    detail: Some(scheme.typ.to_string()),
                    ..Default::default()
                });
            }
        }
    }

    items
}
