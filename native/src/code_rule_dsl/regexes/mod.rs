//! Compiled regexes, one per pattern for the whole process. A rule generated from a table repeats
//! a few patterns across thousands of terms, each query compiles the shared utilities it reaches
//! again for each grammar and `languageOptions` (`compile::shared`), and the rules of the packs a
//! repository installs write many of the same patterns. One regex built per term held each
//! pattern's automata (about 14 KiB) and match cache pool (about 3.5 KiB) that many times over.
//! Every term, value test and anchor plan of every rule holds its pattern's one `Regex`, a shared
//! handle on the compiled regex: a regex is immutable, and its pool gives each thread that matches
//! with it a cache of its own.
//!
//! The table keeps each regex until a `sweep` finds nothing else holding it. A compilation sweeps
//! as it ends (`CodeRuleDslRuntime::compile`), so a regex its validation compiled is the one its
//! terms take, and the ones only validation took, or a rejected document's, go then. A compiled
//! rule sweeps as it drops (`CompiledCodeRule`), so its regexes go with it, as they did when each
//! term built its own, and a process that drops its last rule holds none. A rule dropped on
//! another thread while a document compiles can sweep a regex that document validated before its
//! terms take it; they compile it again. A sweep each time the table doubles, as `calls::names`
//! sweeps names, would drop regexes a document validated before its terms take them, since one
//! document can bring hundreds of new patterns.

use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

/// Each pattern compiled and its regex, until a sweep finds nothing else holding it.
static TABLE: LazyLock<RwLock<HashMap<Box<str>, Arc<regex::Regex>>>> =
    LazyLock::new(Default::default);

/// A compiled regex, the one every holder of its pattern holds. It reads, and prints, as the
/// `regex` crate's `Regex`, which it dereferences to.
#[derive(Clone)]
pub(crate) struct Regex(Arc<regex::Regex>);

impl Regex {
    /// `pattern`'s regex, shared with every other holder of the pattern; the `regex` crate's error
    /// where it does not compile.
    pub(crate) fn new(pattern: &str) -> Result<Self, regex::Error> {
        if let Some(regex) = TABLE.read().get(pattern) {
            return Ok(Self(Arc::clone(regex)));
        }
        // Compiled outside the lock; where another thread added the pattern meanwhile, its regex
        // stays the one.
        let regex = Arc::new(regex::Regex::new(pattern)?);
        let mut table = TABLE.write();
        let regex = table.entry(pattern.into()).or_insert(regex);
        Ok(Self(Arc::clone(regex)))
    }
}

impl std::ops::Deref for Regex {
    type Target = regex::Regex;

    fn deref(&self) -> &regex::Regex {
        &self.0
    }
}

impl std::fmt::Debug for Regex {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&self.0, formatter)
    }
}

/// Drops the regexes nothing outside the table holds.
pub(crate) fn sweep() {
    TABLE
        .write()
        .retain(|_, regex| Arc::strong_count(regex) > 1);
}

#[cfg(test)]
mod tests;
