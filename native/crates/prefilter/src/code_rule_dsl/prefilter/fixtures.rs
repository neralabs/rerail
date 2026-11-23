//! The prefilter tests' fixtures: a stored source and its published tape, a rule's matcher outside any document.
//! This crate's tests and the engine's read them.

use super::*;
use crate::code_rule_dsl::language::GrammarDialect;

pub use crate::code_rule_dsl::fixtures::{document, open, temp};

/// Stores `source` and its tape, publishes the tape's prefilter rows, and returns both.
pub fn put(store: &ContentStore, source: &str, lang: GrammarDialect) -> (TapeId, TapeDoc) {
    let (doc, bytes) = document(source, lang);
    let grammar = i64::from(language::tape_tag(lang).unwrap());
    let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
    let tape = store.put_tape(source_id, grammar, 1, i64::from(crate::code_rule_dsl::tape::formats::stored_version()), &bytes).unwrap();
    publish(store, &[tape], std::slice::from_ref(&doc)).unwrap();
    (tape, doc)
}

/// A rule's compiled matcher, outside any rule document.
pub fn matcher(rule: &serde_json::Value, lang: GrammarDialect) -> crate::code_rule_dsl::matcher::AstMatcher {
    let ids = std::collections::BTreeMap::new();
    let utils = crate::code_rule_dsl::compile::UtilScope {
        table: std::sync::Weak::new(),
        ids: &ids,
        options: None,
        python: None,
        transform: None,
        conflicts: None,
        captures: Default::default(),
        calls: None,
        instances: None,
        documents: None,
        side: crate::code_rule_dsl::compile::documents::Side::Host,
        patterns: None,
    };
    crate::code_rule_dsl::compile::compile_ast_rule(rule, lang, "test/rule", &mut Vec::new(), &utils).unwrap()
}
