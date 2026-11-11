use super::resolve::{
    ValueKey, binding_value, constant_value, followed_table, memoize_structure, memoize_value,
    memoized_structure, memoized_value, unshadowed_global,
};
use super::util::{UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
use crate::code_rule_dsl::budget;
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
use crate::code_rule_dsl::language::adapter::pack as language_pack;

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
            { let pack = language_pack::pack(family); pack.value_expression_kinds() };
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
        { let pack = language_pack::pack(self.family); pack.value_lang() }
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
        let decoded = { let pack = language_pack::pack(self.family); language_pack::value_decode(pack, node, &mut context) };
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
        let tokens = { let pack = language_pack::pack(self.family); language_pack::value_structure(pack, node, equality, &mut context) };
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
    /// `name: true`: the predicates read what the node names rather than its value: ESTree's
    /// name (an identifier's, escapes decoded; a private name's without `#`) or a string literal's
    /// value, as `compare`'s `name` text reads it; PHP's symbol name. A node without one fails.
    pub(crate) name: bool,
}

impl ValueTest {
    /// `Some(true)`, `Some(false)`, or `None` when the verdict is unknown.
    pub(crate) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> Option<bool> {
        let value = if self.name {
            let family = self.reading.family;
            let Some(units) =
                ({ let pack = language_pack::pack(family); language_pack::value_node_name(pack, node) })
            else {
                return Some(false);
            };
            // A name holding a lone surrogate has no text to compare.
            let text = String::from_utf16(&units).ok()?;
            Rc::new(
                { let pack = language_pack::pack(family); pack.value_string(&text) },
            )
        } else {
            if !self.reading.has_value(node) {
                return Some(false);
            }
            self.reading.value(node)
        };
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
            let integer = { let pack = language_pack::pack(lang); pack.value_integer(value.as_ref()) };
            if !all(Some(integer.is_some_and(|integer| {
                !(integer & BigInt::from(bits)).is_zero()
            }))) {
                return Some(false);
            }
        }
        for (bound, limit) in &self.bounds {
            let numeric = { let pack = language_pack::pack(lang); pack.value_numeric(value.as_ref()) };
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

/// W's values of a file's expression nodes, as a clause's `value` reads them (`ValueTest::holds`) in its per-file
/// context: the file's scopes installed as W's evaluation installs them (`FileScopeSlot`: its module, its path, its
/// syntax errors hiding bindings), the query's language options the defaults, and no utility budget (past W's, its
/// clause fails the file; here every node is decoded). For each reading asked (`plain`; `follow`: names followed
/// through their file-local bindings, W's `value.follow`), `each(followed, ordinal, text, types)` for every tape node
/// that is an expression (`Reading::has_value`) whose value is known: `text` the value as its language converts it to a
/// string (`ValueRules::to_str`: Python's `str(v)` of a scalar, so a str, int, float, bool, None or Ellipsis; none for
/// a container or bytes), `types` the type names W's `type` accepts of it (`Value::is_type`: its `type_name`, and
/// `object` for a JavaScript RegExp). Each node is decoded once per reading, memoized per file as W memoizes it.
pub(in crate::code_rule_dsl) fn file_values(
    tape: &crate::code_rule_dsl::tape::TapeDoc,
    file: &crate::code_rule_dsl::model::FileInput,
    plain: bool,
    follow: bool,
    each: &mut ValueSink<'_>,
) {
    let language = tape.file_dialect();
    let root = ast_grep_core::AstGrep::doc(tape.clone());
    let hide = crate::code_rule_dsl::soundness::SyntaxErrors::of_doc(tape).hide_bindings();
    let _scopes = super::FileScopeSlot::install(Vec::new(), file.module.clone(), hide).file(file);
    for followed in [false, true] {
        if !(if followed { follow } else { plain }) {
            continue;
        }
        let reading = Reading::new(language, None, None);
        let reading = if followed { reading.following() } else { reading };
        // The reading's scope-table charges are its own, as one clause's are.
        super::resolve::CURRENT_CLAUSE.set(&reading as *const Reading as usize);
        let lang = reading.lang();
        for ordinal in 0..tape.transducer_len() as u32 {
            let node = super::node_at(&root, ordinal);
            if !reading.has_value(&node) {
                continue;
            }
            UTIL_EXCEEDED.set(None);
            UTIL_STEPS_LEFT.set(usize::MAX);
            let value = reading.value(&node);
            if !value.is_known() {
                continue;
            }
            let text = { let pack = language_pack::pack(lang); language_pack::value_to_str(pack, &value, &node) };
            let own;
            let types: &[&'static str] = match value.type_name(lang) {
                None => &[],
                Some("regexp") => &["regexp", "object"],
                Some(name) => {
                    own = [name];
                    &own
                }
            };
            each(followed, ordinal, text, types);
        }
    }
    super::resolve::CURRENT_CLAUSE.set(0);
    UTIL_EXCEEDED.set(None);
}

/// [`file_values`]' `each(followed, ordinal, text, types)`.
pub(in crate::code_rule_dsl) type ValueSink<'a> = dyn FnMut(bool, u32, Option<String>, &[&'static str]) + 'a;

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
