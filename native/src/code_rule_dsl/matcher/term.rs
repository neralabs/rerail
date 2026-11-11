//! A semantic term a language registers (ECMAScript's `importTarget`): W compiles it by its key, matches,
//! anchors and profiles it as any term (`Semantic::Term`), without reading it.

use super::{AstMatcher, Reading};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::literal::SubDoc;
use crate::code_rule_dsl::scope::options::LanguageOptions;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::tree_sitter::StrDoc;
use ast_grep_core::{Doc, Node};
use std::any::TypeId;
use std::sync::{Arc, OnceLock};

/// A compiled language term.
pub(crate) trait LanguageTerm: Send + Sync {
    /// The rule key it is compiled from.
    fn key(&self) -> &'static str;
    /// How it reads the node's value (its anchors are the nodes with one).
    fn reading(&self) -> &Reading;
    /// Whether it holds of `node` (one method per document type the engine has: `holds`).
    fn holds_tape(&self, node: &Node<'_, TapeDoc>) -> bool;
    fn holds_sub(&self, node: &Node<'_, SubDoc>) -> bool;
    fn holds_str(&self, node: &Node<'_, StrDoc<GrammarDialect>>) -> bool;
    /// What the profile reports of it.
    fn profile(&self) -> serde_json::Value;
    /// Whether it reads the files resolutions read (`resolution`), installed around the evaluations that read it.
    fn reads_resolution(&self) -> bool;
}

/// `value` as a `B`, where `A` and `B` are one type.
pub(crate) unsafe fn same<A, B>(a: A) -> B {
    debug_assert_eq!(std::mem::size_of::<A>(), std::mem::size_of::<B>());
    let a = std::mem::ManuallyDrop::new(a);
    unsafe { std::ptr::read(&*a as *const A as *const B) }
}

/// Whether `term` holds of `node`.
pub(crate) fn holds<D: Doc>(term: &dyn LanguageTerm, node: &Node<'_, D>) -> bool {
    if TypeId::of::<D>() == TypeId::of::<TapeDoc>() {
        return term.holds_tape(unsafe { same(node) });
    }
    if TypeId::of::<D>() == TypeId::of::<SubDoc>() {
        return term.holds_sub(unsafe { same(node) });
    }
    if TypeId::of::<D>() == TypeId::of::<StrDoc<GrammarDialect>>() {
        return term.holds_str(unsafe { same(node) });
    }
    unreachable!("a language term over a document type the engine has not: {}", std::any::type_name::<D>())
}

/// What compiles a language term from its rule key.
pub(crate) struct TermCompiler {
    pub(crate) key: &'static str,
    /// Checks the key's value before compilation (`validate_ast_rule`).
    pub(crate) validate: fn(&serde_json::Value, &str) -> napi::Result<()>,
    /// The term of the key's value, in `language`.
    pub(crate) compile:
        fn(&serde_json::Value, GrammarDialect, &str, Option<&Arc<LanguageOptions>>) -> napi::Result<AstMatcher>,
}

static TERMS: OnceLock<Vec<TermCompiler>> = OnceLock::new();

/// Registers the language terms (once, at load).
pub(crate) fn install_terms(terms: Vec<TermCompiler>) {
    let _ = TERMS.set(terms);
}

/// The registered language terms.
pub(crate) fn terms() -> &'static [TermCompiler] {
    TERMS.get().map_or(&[], Vec::as_slice)
}

/// The language term compiled from `key`, if one is registered.
pub(crate) fn term(key: &str) -> Option<&'static TermCompiler> {
    terms().iter().find(|term| term.key == key)
}
