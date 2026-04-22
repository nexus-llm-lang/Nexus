//! String interning for cheap clone + O(1) comparison of identifiers.
//!
//! `Symbol` is a thin wrapper around `u32` (Copy, Eq, Hash).
//! All compiler name fields (function names, param names, labels, etc.)
//! use Symbol instead of String.

use std::cell::RefCell;
use std::collections::HashMap;

/// An interned string identifier. Copy + O(1) equality.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol(u32);

struct Interner {
    map: HashMap<&'static str, Symbol>,
    strings: Vec<&'static str>,
}

impl Interner {
    fn new() -> Self {
        Interner {
            map: HashMap::new(),
            strings: Vec::new(),
        }
    }

    fn intern(&mut self, s: &str) -> Symbol {
        if let Some(&sym) = self.map.get(s) {
            return sym;
        }
        let leaked: &'static str = Box::leak(s.to_string().into_boxed_str());
        let sym = Symbol(self.strings.len() as u32);
        self.strings.push(leaked);
        self.map.insert(leaked, sym);
        sym
    }

    fn resolve(&self, sym: Symbol) -> &'static str {
        self.strings[sym.0 as usize]
    }
}

thread_local! {
    static INTERNER: RefCell<Interner> = RefCell::new(Interner::new());
}

impl Symbol {
    pub fn intern(s: &str) -> Symbol {
        INTERNER.with(|i| i.borrow_mut().intern(s))
    }

    pub fn as_str(self) -> &'static str {
        INTERNER.with(|i| i.borrow().resolve(self))
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.as_str())
    }
}

impl std::fmt::Display for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl From<&str> for Symbol {
    fn from(s: &str) -> Self {
        Symbol::intern(s)
    }
}

impl From<String> for Symbol {
    fn from(s: String) -> Self {
        Symbol::intern(&s)
    }
}

impl From<&String> for Symbol {
    fn from(s: &String) -> Self {
        Symbol::intern(s)
    }
}

impl PartialEq<&str> for Symbol {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<str> for Symbol {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialOrd for Symbol {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Symbol {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl std::borrow::Borrow<str> for Symbol {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Symbol {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<String> for Symbol {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Symbol> for String {
    fn eq(&self, other: &Symbol) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Symbol> for &str {
    fn eq(&self, other: &Symbol) -> bool {
        *self == other.as_str()
    }
}

impl Symbol {
    pub fn starts_with(self, prefix: &str) -> bool {
        self.as_str().starts_with(prefix)
    }

    pub fn ends_with(self, suffix: &str) -> bool {
        self.as_str().ends_with(suffix)
    }

    pub fn contains(self, pat: &str) -> bool {
        self.as_str().contains(pat)
    }

    pub fn strip_prefix(self, prefix: &str) -> Option<&'static str> {
        self.as_str().strip_prefix(prefix)
    }
}
