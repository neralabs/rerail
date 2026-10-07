//! `parse` (a Rerail extension): a string literal's decoded text parsed by a sub-grammar, and a rule
//! of that grammar matched on the parsed tree's root. The tree is a document of its own with its
//! own environment; captures its rule binds are exported to the host as span captures.

use super::AstMatcher;
#[cfg(test)]
use super::each;
use super::each::HostMap;
#[cfg(test)]
use super::each::HostRanges;
#[cfg(test)]
use super::exported::export;
use super::exported::{exported_span, exported_text, is_span_variable};
use super::facts::{Facts, Target};
#[cfg(test)]
use super::resolve::SubTables;
use super::resolve::{
    file_table, memoize_parse, memoize_value_parse, memoized_parse, memoized_value_parse,
};
#[cfg(test)]
use super::util::UNKNOWN;
use super::util::{UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
use crate::policy_dsl::budget;
use crate::policy_dsl::language::GrammarDialect;
#[cfg(test)]
use crate::policy_dsl::literal::SubDoc;
use crate::policy_dsl::literal::{self, Grammar, LiteralHost, Parsed};
use crate::policy_dsl::scope::Reading;
use crate::policy_dsl::value::Value;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
#[cfg(test)]
use ast_grep_core::{AstGrep, Matcher};
use ast_grep_core::{Doc, Node};
#[cfg(test)]
use std::borrow::Cow;
#[cfg(test)]
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct ParseTest {
    pub(crate) grammar: Grammar,
    /// The rule matched on the parsed tree's root, compiled for the grammar.
    pub(crate) rule: AstMatcher,
    /// The captures the rule may bind, exported to the host.
    pub(crate) exports: Vec<String>,
    /// Host kinds that hold a literal of the grammar.
    pub(crate) host_kinds: Arc<BTreeSet<u16>>,
    /// How the scope table reads the file (`annotation`, whose trees the table parses).
    pub(crate) reading: Reading,
    /// With `host` rules in the tree's rule: whether each is sound under syntax errors and none
    /// is negated (they read the file around the literal); `None` without any.
    pub(crate) host_sound: Option<bool>,
}

/// `host` (inside a text grammar's rule): a rule of the host language matched on the host node the
/// `parse` holds, in the host's environment as the `parse` began, with the tree's captures bound
/// so far exported into it as `parse` exports them (text and host range). It binds nothing the
/// tree's rule sees: a relation over the tree's nodes that holds it tests the host per candidate.
#[derive(Clone)]
pub(crate) struct HostTest {
    pub(crate) rule: AstMatcher,
}

/// A parsed tree's captures as the host reads them: name, text, host range.
#[cfg(test)]
type Exports = [(String, String, Range<usize>)];

/// The host a `host` rule runs against: a matcher of the innermost `parse` being matched, which runs
/// a host rule on its node with the tree's captures, and the tree it holds.
#[cfg(test)]
struct Hook {
    run: *const (dyn Fn(&AstMatcher, &Exports) -> bool + 'static),
    tree: Arc<literal::SubTree>,
}

#[cfg(test)]
thread_local! {
    /// The `parse` terms being matched whose rules hold `host`, innermost last.
    static HOOKS: RefCell<Vec<Hook>> = const { RefCell::new(Vec::new()) };
}

/// A `parse`'s host while its tree's rule is matched; dropping it restores the enclosing one.
#[cfg(test)]
struct Hosted;

#[cfg(test)]
impl Hosted {
    fn enter<'f>(
        run: &'f (dyn Fn(&AstMatcher, &Exports) -> bool + 'f),
        tree: Arc<literal::SubTree>,
    ) -> Self {
        // SAFETY: the pointer is read only while this guard lives, and the guard is dropped before
        // `run`, which the caller declares first; the lifetime is erased only to store it.
        let run: *const (dyn Fn(&AstMatcher, &Exports) -> bool + 'f) = run;
        let run = unsafe {
            std::mem::transmute::<
                *const (dyn Fn(&AstMatcher, &Exports) -> bool + 'f),
                *const (dyn Fn(&AstMatcher, &Exports) -> bool + 'static),
            >(run)
        };
        HOOKS.with_borrow_mut(|hooks| hooks.push(Hook { run, tree }));
        Self
    }
}

#[cfg(test)]
impl Drop for Hosted {
    fn drop(&mut self) {
        HOOKS.with_borrow_mut(|hooks| hooks.pop());
    }
}

#[cfg(test)]
impl HostTest {
    pub(super) fn match_node<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let tree = literal::sub_tree(&node)?;
        let run = HOOKS.with_borrow(|hooks| {
            hooks
                .iter()
                .rev()
                .find(|hook| Arc::ptr_eq(&hook.tree, tree))
                .map(|hook| hook.run)
        })?;
        let exports = host_exports(env, tree);
        // SAFETY: `Hosted` keeps the closure alive while it is on the stack (see `Hosted::enter`).
        let run = unsafe { &*run };
        run(&self.rule, &exports).then_some(node)
    }
}

/// A parsed tree's captures bound so far in `env` as its host reads them: each node capture's
/// text and host range, and each capture a nested tree exported, its span mapped out of `tree`.
pub(super) fn host_exports<D: Doc>(
    env: &MetaVarEnv<'_, D>,
    tree: &literal::SubTree,
) -> Vec<(String, String, Range<usize>)> {
    let mut exports = Vec::new();
    for variable in env.get_matched_variables() {
        let MetaVariable::Capture(name, _) = variable else {
            continue;
        };
        if is_span_variable(&name) {
            continue;
        }
        if let Some(found) = env.get_match(&name) {
            if let Some(range) = tree.host_range(found.node_id()) {
                exports.push((name.clone(), found.text().into_owned(), range));
            }
        } else if let (Some(text), Some(span)) =
            (exported_text(env, &name), exported_span(env, &name))
            && let Some(range) = tree.host_span(span)
        {
            exports.push((name.clone(), text, range));
        }
    }
    exports
}

/// The query engine's view of `parse` (`facts::slot`): the gate, whose row carries the parsed
/// literal (and, for a string type definition, the file table its names are read in).
impl ParseTest {
    pub(super) fn facts<'t, D: Doc>(&self, node: Node<'t, D>, facts: &mut Facts<'t, D>) {
        if !self.host_kinds.contains(&node.kind_id()) {
            return;
        }
        if self.grammar == Grammar::Annotation {
            let Some(table) = file_table(&node, GrammarDialect::Python, self.reading) else {
                return;
            };
            let Some(annotation) = table.annotation_at(&node.range()) else {
                return;
            };
            let tree = Arc::clone(&annotation.tree);
            facts.gate(1, Target::Tree(tree, Some(table)));
            return;
        }
        match parsed_tree(&node, self.grammar) {
            None | Some(Parsed::NotString) => {}
            Some(Parsed::Unknown) => {
                facts.gate(-1, Target::None);
            }
            Some(Parsed::Tree(tree)) => {
                facts.gate(1, Target::Tree(tree, None));
            }
        }
    }
}

#[cfg(test)]
impl ParseTest {
    pub(super) fn match_node<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        if !self.host_kinds.contains(&node.kind_id()) {
            return None;
        }
        if self.grammar == Grammar::Annotation {
            // A string type definition's tree is the table's; any other string has none.
            let table = file_table(&node, GrammarDialect::Python, self.reading)?;
            let tree = Arc::clone(&table.annotation_at(&node.range())?.tree);
            let _tables = SubTables::enter(table);
            return self.match_tree(node, env, tree);
        }
        let tree = match parsed_tree(&node, self.grammar)? {
            Parsed::Tree(tree) => tree,
            Parsed::NotString => return None,
            Parsed::Unknown => {
                UNKNOWN.set(true);
                return None;
            }
        };
        self.match_tree(node, env, tree)
    }

    /// Matches the rule on the tree's root, exporting its captures.
    fn match_tree<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
        tree: Arc<literal::SubTree>,
    ) -> Option<Node<'t, D>> {
        let root = AstGrep::doc(SubDoc { tree: tree.clone() });
        let mut inner = Cow::Owned(MetaVarEnv::new());
        // `finding.each` reads the tree's bindings by their host ranges.
        let _hosts = each::active().then(|| HostRanges::enter(host_map(&tree)));
        // A `host` rule runs on this node in the host's environment as it stands now, with the
        // tree's captures; it binds nothing and excludes nothing `each` enumerates.
        let before = self.host_sound.is_some().then(|| env.as_ref().clone());
        let run = |rule: &AstMatcher, exports: &Exports| {
            let Some(before) = &before else {
                return false;
            };
            let mut host = Cow::Borrowed(before);
            for (name, text, range) in exports {
                export(&mut host, name, text, range.clone());
            }
            each::suspended(|| rule.match_node_with_env(node.clone(), &mut host).is_some())
        };
        let _hosted = self
            .host_sound
            .is_some()
            .then(|| Hosted::enter(&run, tree.clone()));
        self.rule.match_node_with_env(root.root(), &mut inner)?;
        for name in &self.exports {
            if let Some(found) = inner.get_match(name)
                && let Some(range) = tree.host_range(found.node_id())
            {
                export(env, name, &found.text(), range);
            } else if let (Some(text), Some(span)) =
                (exported_text(&inner, name), exported_span(&inner, name))
            {
                // A capture a nested parse exported: its span is in this tree's range space.
                if let Some(range) = tree.host_span(span) {
                    export(env, name, &text, range);
                }
            }
        }
        Some(node)
    }
}

/// A parsed tree's coordinates in the document it was parsed from.
pub(super) fn host_map(tree: &Arc<literal::SubTree>) -> HostMap {
    let (nodes, spans) = (Arc::clone(tree), Arc::clone(tree));
    HostMap {
        node: Arc::new(move |id| nodes.host_range(id)),
        span: Arc::new(move |range| spans.host_span(range)),
    }
}

/// The node's tree under a text grammar, parsed once per file; `None` when the budget ran out (the
/// check fails).
pub(super) fn parsed_tree<D: Doc>(node: &Node<'_, D>, grammar: Grammar) -> Option<Parsed> {
    let key = (node.node_id(), grammar);
    if let Some(memo) = memoized_parse(&key) {
        return Some(match memo {
            Some(tree) => Parsed::Tree(tree),
            None => Parsed::Unknown,
        });
    }
    let parsed = match grammar.literal_host() {
        LiteralHost::EcmaComment => literal::parse_comment(node, &mut spend_steps),
        LiteralHost::PythonString => literal::parse_string(node, grammar, &mut spend_steps),
        LiteralHost::PhpString => literal::parse_php_string(node, grammar, &mut spend_steps),
        LiteralHost::PhpComment => literal::parse_php_comment(node, &mut spend_steps),
    };
    let Some(parsed) = parsed else {
        // The budget fails the check; a deadline leaves the parse unknown.
        return UTIL_EXCEEDED.get().is_none().then_some(Parsed::Unknown);
    };
    match &parsed {
        Parsed::Tree(tree) => memoize_parse(key, Some(Arc::clone(tree))),
        Parsed::Unknown => memoize_parse(key, None),
        Parsed::NotString => {}
    }
    Some(parsed)
}

/// The tree of the string `reading` gives `node`, an expression that is no string literal (a PHP
/// format through concatenation or a variable's binding), parsed once per file and reading, every
/// node mapped to `node`'s range. `NotString` where the value is known and no string; `None` when
/// the budget ran out (the check fails).
pub(super) fn parsed_value_tree<D: Doc>(
    node: &Node<'_, D>,
    reading: &super::value::Reading,
    grammar: Grammar,
) -> Option<Parsed> {
    let key = ((node.node_id(), reading.cache_id()), grammar);
    if let Some(memo) = memoized_value_parse(&key) {
        return Some(match memo {
            Some(tree) => Parsed::Tree(tree),
            None => Parsed::Unknown,
        });
    }
    let value = reading.value(node);
    if UTIL_EXCEEDED.get().is_some() {
        return None;
    }
    let bytes = match &*value {
        Value::PhpStr(bytes) => bytes.clone(),
        // A decode cut short by a deadline is unknown too, and not kept.
        Value::Unknown => return Some(Parsed::Unknown),
        _ => return Some(Parsed::NotString),
    };
    let Some(parsed) = literal::parse_php_value(bytes, node.range(), grammar, &mut spend_steps)
    else {
        return UTIL_EXCEEDED.get().is_none().then_some(Parsed::Unknown);
    };
    match &parsed {
        Parsed::Tree(tree) => memoize_value_parse(key, Some(Arc::clone(tree))),
        Parsed::Unknown => memoize_value_parse(key, None),
        Parsed::NotString => {}
    }
    Some(parsed)
}

/// Spends `count` steps of the root's utility budget for a parse; false once it ran out (the check
/// then fails for the file with `UtilLimit::Value`). A deadline makes the parse unknown.
fn spend_steps(count: usize) -> bool {
    if UTIL_EXCEEDED.get().is_some() {
        return false;
    }
    if budget::matcher_call_budget_exhausted() {
        return false;
    }
    match UTIL_STEPS_LEFT.get().checked_sub(count) {
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
