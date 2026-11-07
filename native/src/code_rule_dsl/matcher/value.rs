use super::resolve::{
    ValueKey, binding_value, constant_value, followed_table, memoize_structure, memoize_value,
    memoized_structure, memoized_value, unshadowed_global,
};
use super::util::{UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
use crate::code_rule_dsl::budget;
use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules, with_adapter};
use crate::code_rule_dsl::language::{Family, GrammarDialect, kind_ids};
use crate::code_rule_dsl::literal;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::text_regex::TextRegex;
use crate::code_rule_dsl::value::{BigInt, Context, Value};
use ast_grep_core::{Doc, Node};
use num_traits::Zero;
use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

/// How a node's value is read: its grammar and the query's language options, which decide what
/// is a builtin.
#[derive(Clone)]
pub(crate) struct Reading {
    pub(crate) language: GrammarDialect,
    /// The language's family, whose adapter decodes the values.
    pub(crate) family: Family,
    pub(crate) options: Option<Arc<LanguageOptions>>,
    pub(crate) python: Option<Arc<PythonOptions>>,
    /// The grammar's expression kinds: only these have values.
    pub(crate) expressions: Arc<BTreeSet<u16>>,
    /// `follow`: a name is the value its binding carries (see `ScopeTable::binding`).
    pub(crate) follow: bool,
    /// A parsed literal's nodes (`parse`): each has the value its grammar gives it, if any.
    pub(crate) literal: bool,
    /// Stable identity for this reading's cache entries, including its language options.
    cache_id: usize,
}

static NEXT_READING_ID: AtomicUsize = AtomicUsize::new(1);

impl Reading {
    pub(crate) fn new(
        language: GrammarDialect,
        options: Option<Arc<LanguageOptions>>,
        python: Option<Arc<PythonOptions>>,
    ) -> Self {
        let family = Family::of(language);
        let kinds =
            with_adapter!(family, A => <<A as Adapter>::Value as ValueRules>::EXPRESSION_KINDS);
        Self {
            language,
            family,
            options,
            python,
            expressions: Arc::new(kind_ids(language, kinds)),
            follow: false,
            literal: false,
            cache_id: NEXT_READING_ID.fetch_add(1, AtomicOrdering::Relaxed),
        }
    }

    /// How a parsed tree's nodes are read: by the values its grammar gives them, which are values
    /// of the host's language (Python's for a literal, JavaScript's for a directive comment).
    pub(crate) fn literal(language: GrammarDialect) -> Self {
        Self {
            expressions: Arc::new(BTreeSet::new()),
            literal: true,
            ..Self::new(language, None, None)
        }
    }

    /// This reading, following names through their bindings.
    pub(crate) fn following(mut self) -> Self {
        self.follow = true;
        self
    }

    /// This reading's identity in the file's caches.
    pub(super) fn cache_id(&self) -> usize {
        self.cache_id
    }

    /// Whether the node is an expression, which has a value (known or not).
    pub(crate) fn has_value<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        if self.literal {
            return literal::sub_tree(node)
                .is_some_and(|tree| tree.value(node.node_id()).is_some());
        }
        self.expressions.contains(&node.kind_id())
    }

    /// The language whose semantics the reading's values follow.
    pub(crate) fn lang(&self) -> Family {
        with_adapter!(self.family, A => <<A as Adapter>::Value as ValueRules>::LANG)
    }

    /// The node's value, decoded once per file and reading. Each node decoded spends a step of
    /// the root's utility budget; running out fails the clause for the file (`UtilLimit::Value`)
    /// and a deadline makes the value unknown, never false.
    pub(crate) fn value<D: Doc>(&self, node: &Node<'_, D>) -> Rc<Value> {
        if self.literal {
            return Rc::new(
                literal::sub_tree(node)
                    .and_then(|tree| tree.value(node.node_id()))
                    .cloned()
                    .unwrap_or(Value::Unknown),
            );
        }
        let key: ValueKey = (node.node_id(), self.cache_id);
        if let Some(value) = memoized_value(&key) {
            return value;
        }
        let mut spend = spend_step;
        let mut global = |range, name: &str| {
            unshadowed_global(
                node,
                range,
                name,
                self.language,
                self.options.as_deref(),
                self.python.as_deref(),
            )
        };
        let mut binding = |range, name: &str| {
            binding_value(
                node,
                range,
                name,
                self.language,
                self.options.as_deref(),
                self.python.as_deref(),
            )
        };
        let mut constant = |target| constant_value(node, target, self.language);
        let mut table = || {
            followed_table(
                node,
                self.language,
                self.options.as_deref(),
                self.python.as_deref(),
            )
        };
        let mut context = Context::with_globals(&mut spend, &mut global);
        if self.follow {
            context = context
                .following(&mut binding)
                .following_constants(&mut constant)
                .following_table(&mut table);
        }
        let decoded = with_adapter!(
            self.family,
            A => <<A as Adapter>::Value as ValueRules>::decode(node, &mut context)
        );
        let exhausted = context.exhausted();
        let decoded = Rc::new(decoded);
        // A decode cut short by the budget or a deadline is not the node's value.
        if !exhausted && UTIL_EXCEEDED.get().is_none() {
            memoize_value(key, &decoded);
        }
        decoded
    }

    /// A node's structure under one of the structural equalities (`compare`'s structural modes):
    /// Ruff's for Python, PHP-Parser's for PHP. Read once per file; each node read spends a step
    /// of the root's utility budget, as a decode does. `None` where the structure is not known.
    pub(crate) fn structure<D: Doc>(
        &self,
        node: &Node<'_, D>,
        equality: Equality,
    ) -> Rc<Option<Vec<Tok>>> {
        let key = (node.node_id(), equality);
        if let Some(tokens) = memoized_structure(&key) {
            return tokens;
        }
        let mut spend = spend_step;
        let mut context = Context::new(&mut spend);
        let tokens = with_adapter!(
            self.family,
            A => <<A as Adapter>::Value as ValueRules>::structure(node, equality, &mut context)
        );
        // A read cut short by the budget or a deadline is not the node's structure.
        if context.exhausted() || UTIL_EXCEEDED.get().is_some() {
            return Rc::new(None);
        }
        let tokens = Rc::new(tokens);
        memoize_structure(key, &tokens);
        tokens
    }
}

/// Spends one step of the root's utility budget for a decode; false once it or the deadline ran
/// out (the budget then fails the clause for the file with `UtilLimit::Value`).
pub(super) fn spend_step() -> bool {
    if UTIL_EXCEEDED.get().is_some() || budget::matcher_call_budget_exhausted() {
        return false;
    }
    match UTIL_STEPS_LEFT.get().checked_sub(1) {
        Some(left) => {
            UTIL_STEPS_LEFT.set(left);
            true
        }
        None => {
            UTIL_EXCEEDED.set(Some(UtilLimit::Value));
            false
        }
    }
}

/// A bound on a number: `gt`, `ge`, `lt` or `le`.
#[derive(Clone, Copy)]
pub(crate) enum Bound {
    Gt,
    Ge,
    Lt,
    Le,
}

/// `value` (a Rerail extension): predicates over the node's own value, each three-valued. A node
/// that is not an expression has no value and fails every predicate. An expression whose value is
/// not known satisfies none of them and fails none of them, except `constant`, which asks exactly
/// whether it is known.
#[derive(Clone)]
pub(crate) struct ValueTest {
    pub(crate) reading: Reading,
    pub(crate) eq: Option<Value>,
    pub(crate) types: Vec<String>,
    pub(crate) constant: Option<bool>,
    pub(crate) starts_with: Vec<Value>,
    pub(crate) regex: Option<TextRegex>,
    pub(crate) bits_any: Option<u64>,
    pub(crate) bounds: Vec<(Bound, Value)>,
    pub(crate) truthy: Option<bool>,
}

impl ValueTest {
    /// `Some(true)`, `Some(false)`, or `None` when the verdict is unknown.
    pub(crate) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> Option<bool> {
        if !self.reading.has_value(node) {
            return Some(false);
        }
        let value = self.reading.value(node);
        let lang = self.reading.lang();
        let mut verdict = Some(true);
        let mut all = |term: Option<bool>| match term {
            Some(false) => false,
            Some(true) => true,
            None => {
                verdict = None;
                true
            }
        };
        if let Some(constant) = self.constant
            && value.is_constant() != constant
        {
            return Some(false);
        }
        if !value.is_known() {
            let only_constant = self.eq.is_none()
                && self.types.is_empty()
                && self.starts_with.is_empty()
                && self.regex.is_none()
                && self.bits_any.is_none()
                && self.bounds.is_empty()
                && self.truthy.is_none();
            return only_constant.then_some(true);
        }
        if !self.types.is_empty()
            && !all(Some(
                self.types.iter().any(|name| value.is_type(lang, name)),
            ))
        {
            return Some(false);
        }
        if let Some(expected) = &self.eq
            && !all(value.equals(expected, lang))
        {
            return Some(false);
        }
        if !self.starts_with.is_empty() {
            let starts = match value.as_ref() {
                Value::Str(text) => Some(self.starts_with.iter().any(|prefix| {
                    matches!(prefix, Value::Str(prefix) if text.starts_with(prefix.as_str()))
                })),
                Value::Utf16(units) => Some(self.starts_with.iter().any(|prefix| {
                    matches!(prefix, Value::Utf16(prefix) if units.starts_with(prefix))
                })),
                Value::PhpStr(bytes) => Some(self.starts_with.iter().any(|prefix| {
                    matches!(prefix, Value::PhpStr(prefix) if bytes.starts_with(prefix))
                })),
                // Any other known value, bytes included, is not a string.
                _ => Some(false),
            };
            if !all(starts) {
                return Some(false);
            }
        }
        if let Some(regex) = &self.regex {
            let matched = match value.as_ref() {
                Value::Str(text) => regex.matches(text),
                // A PHP string that is not UTF-8 has no text to match.
                Value::PhpStr(bytes) => std::str::from_utf8(bytes)
                    .ok()
                    .and_then(|text| regex.matches(text)),
                // A string with a lone surrogate has no UTF-8 form to match.
                Value::Utf16(units) => String::from_utf16(units)
                    .ok()
                    .and_then(|text| regex.matches(&text)),
                _ => Some(false),
            };
            if !all(matched) {
                return Some(false);
            }
        }
        if let Some(bits) = self.bits_any {
            let integer = with_adapter!(lang, A => <<A as Adapter>::Value as ValueRules>::integer(value.as_ref()));
            if !all(Some(integer.is_some_and(|integer| {
                !(integer & BigInt::from(bits)).is_zero()
            }))) {
                return Some(false);
            }
        }
        for (bound, limit) in &self.bounds {
            let numeric = with_adapter!(lang, A => <<A as Adapter>::Value as ValueRules>::numeric(value.as_ref()));
            let holds = numeric
                && value
                    .order(limit, lang)
                    .flatten()
                    .is_some_and(|order| match bound {
                        Bound::Gt => order == Ordering::Greater,
                        Bound::Ge => order != Ordering::Less,
                        Bound::Lt => order == Ordering::Less,
                        Bound::Le => order != Ordering::Greater,
                    });
            if !all(Some(holds)) {
                return Some(false);
            }
        }
        if let Some(truthy) = self.truthy
            && !all(value.truthy(lang).map(|actual| actual == truthy))
        {
            return Some(false);
        }
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::Reading;
    use crate::code_rule_dsl::language::GrammarDialect;

    #[test]
    fn independently_constructed_readings_have_distinct_cache_keys() {
        let first = Reading::new(GrammarDialect::TypeScript, None, None);
        let second = Reading::new(GrammarDialect::TypeScript, None, None);
        assert_ne!(first.cache_id, second.cache_id);
        assert_eq!(first.cache_id, first.clone().cache_id);
    }
}
