//! Language frontend layer:
//! - syntax tree (`ast`)
//! - parser (`parser`)
//! - type/throws checker (`typecheck`)
//! - stdlib source loader (`stdlib`)

pub mod ast;
pub mod lexer;
pub mod package;
pub mod parser;
pub mod stdlib;
pub mod typecheck;
