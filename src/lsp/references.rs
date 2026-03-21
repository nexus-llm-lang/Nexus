use crate::lang::lexer::{self, TokenKind};
use crate::types::Span;

/// Find the identifier at byte offset and all its occurrences in the source.
///
/// Returns `(name, ident_span_at_offset, all_spans)`.
pub fn find_all_references(source: &str, offset: usize) -> Option<(String, Span, Vec<Span>)> {
    let tokens = lexer::tokenize(source).ok()?;

    // Find the ident at offset
    let target = tokens.iter().find(|tok| {
        matches!(&tok.kind, TokenKind::Ident(_))
            && (tok.span.contains(&offset) || tok.span.end == offset)
    })?;

    let name = match &target.kind {
        TokenKind::Ident(n) => n.clone(),
        _ => return None,
    };

    let origin_span = target.span.clone();

    // Collect all ident tokens with the same name
    let refs: Vec<Span> = tokens
        .iter()
        .filter_map(|tok| match &tok.kind {
            TokenKind::Ident(n) if n == &name => Some(tok.span.clone()),
            _ => None,
        })
        .collect();

    Some((name, origin_span, refs))
}
