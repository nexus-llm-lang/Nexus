//! Language frontend layer:
//! - syntax tree (`ast`)
//! - parser (`parser`)
//! - type/effect checker (`typecheck`)
//! - stdlib source loader (`stdlib`)

pub mod ast;
pub mod parser;
pub mod stdlib;
pub mod typecheck;
