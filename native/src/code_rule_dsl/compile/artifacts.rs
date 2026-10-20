//! Immutable pattern artifacts shared within one policy compilation. Mutable matcher graphs
//! and utility numbering remain local to each query. Nested compilations restore their caller.
use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
use ast_grep_core::Pattern;
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{Arc, OnceLock},
};
#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    dialect: GrammarDialect,
    context: String,
    selector: Option<String>,
}
type Compiled = Result<(Arc<Pattern>, GrammarDialect), String>;
type Cell = Arc<OnceLock<Compiled>>;
#[derive(Default)]
struct Artifacts {
    patterns: parking_lot::Mutex<HashMap<Key, Cell>>,
}
thread_local! { static CURRENT: RefCell<Option<Arc<Artifacts>>> = const { RefCell::new(None) }; }
pub(super) struct Scope(Option<Arc<Artifacts>>);
pub(super) fn scope() -> Scope {
    Scope(CURRENT.with_borrow_mut(|current| current.replace(Arc::new(Artifacts::default()))))
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with_borrow_mut(|current| *current = self.0.take());
    }
}
impl Artifacts {
    fn pattern(&self, key: Key) -> Compiled {
        let cell = self.patterns.lock().entry(key.clone()).or_default().clone();
        cell.get_or_init(|| {
            let source = match &key.selector {
                Some(selector) => PatternSource::Contextual {
                    context: &key.context,
                    selector,
                },
                None => PatternSource::Plain(&key.context),
            };
            rule_pattern(source, key.dialect)
                .map(|(p, d)| (Arc::new(p), d))
                .map_err(|e| e.to_string())
        })
        .clone()
    }
}
pub(super) fn pattern(source: PatternSource<'_>, dialect: GrammarDialect) -> Compiled {
    let key = match source {
        PatternSource::Plain(context) => Key {
            dialect,
            context: context.into(),
            selector: None,
        },
        PatternSource::Contextual { context, selector } => Key {
            dialect,
            context: context.into(),
            selector: Some(selector.into()),
        },
    };
    match CURRENT.with_borrow(|c| c.clone()) {
        Some(cache) => cache.pattern(key),
        None => Artifacts::default().pattern(key),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compilation_scope_shares_immutable_patterns_and_restores_nested_scope() {
        let _scope = scope();
        let get = || {
            pattern(PatternSource::Plain("f($X)"), GrammarDialect::Python)
                .unwrap()
                .0
        };
        let first = get();
        assert!(Arc::ptr_eq(&first, &get()));
        {
            let _nested = scope();
            assert!(!Arc::ptr_eq(&first, &get()));
        }
        assert!(Arc::ptr_eq(&first, &get()));
        let cache = CURRENT.with_borrow(|c| c.clone().unwrap());
        std::thread::scope(|s| {
            for _ in 0..8 {
                let cache = &cache;
                let first = &first;
                s.spawn(move || {
                    let key = Key {
                        dialect: GrammarDialect::Python,
                        context: "f($X)".into(),
                        selector: None,
                    };
                    assert!(Arc::ptr_eq(&cache.pattern(key).unwrap().0, first));
                });
            }
        });
    }
}
