//! Compiled regexes, shared by pattern while a document compiles. A rule generated from a table
//! repeats a few patterns across thousands of terms, and each query compiles the shared utilities
//! it reaches again for each grammar and `languageOptions` (`compile::shared`). One regex built
//! per term held each pattern's automata (about 14 KiB) and match cache pool (about 3.5 KiB)
//! thousands of times over. Under a `Sharing`, the document's regex terms, value tests and anchor
//! plans hold one `Arc<Regex>` per pattern: a regex is immutable, and its pool gives each thread
//! that matches with it a cache of its own.

use regex::Regex;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// Each pattern compiled on this thread since its outermost `Sharing` began, with its regex
/// (`None` where it does not compile); `None` outside a `Sharing`.
type Compiled = Option<HashMap<String, Option<Arc<Regex>>>>;

thread_local! {
    static COMPILED: RefCell<Compiled> = const { RefCell::new(None) };
}

/// While alive, `compile` gives every compilation of a pattern on this thread the same regex. A
/// `Sharing` begun under another shares the outer one's regexes, and the outermost one's end
/// drops them: what the document's terms hold stays theirs.
pub(crate) struct Sharing {
    outermost: bool,
}

impl Sharing {
    pub(crate) fn begin() -> Self {
        let outermost = COMPILED.with(|compiled| {
            let mut compiled = compiled.borrow_mut();
            let outermost = compiled.is_none();
            if outermost {
                *compiled = Some(HashMap::new());
            }
            outermost
        });
        Self { outermost }
    }
}

impl Drop for Sharing {
    fn drop(&mut self) {
        if self.outermost {
            COMPILED.with(|compiled| compiled.borrow_mut().take());
        }
    }
}

/// `pattern` as a Rust regex, the one compiled for it already under the thread's `Sharing` if
/// any; `None` where it does not compile.
pub(crate) fn compile(pattern: &str) -> Option<Arc<Regex>> {
    COMPILED.with(|compiled| {
        let mut compiled = compiled.borrow_mut();
        let Some(compiled) = compiled.as_mut() else {
            return Regex::new(pattern).ok().map(Arc::new);
        };
        if let Some(regex) = compiled.get(pattern) {
            return regex.clone();
        }
        let regex = Regex::new(pattern).ok().map(Arc::new);
        compiled.insert(pattern.to_owned(), regex.clone());
        regex
    })
}

#[cfg(test)]
mod tests;
