//! `formatArgs` (a Rerail extension): which arguments of a format call no field of its template
//! consumes, and which fields no argument fills: for Python as Ruff 0.16.9's Pyflakes rules read
//! them (`rules/pyflakes/rules/strings.rs`: F504, F505, F507, F522, F523, F524), for PHP's
//! printf-family calls as PHP 8.5.11 and PHPStan 2.2.16 read them (`printf`), for its `sscanf`
//! and `fscanf` calls as PHP 8.5.11 validates them (`scanf`), and for Rust's `format_args!`
//! family as rustc 1.99.0 binds them (`rust`).

use crate::code_rule_dsl::tree::Tree;
use super::AstMatcher;
use super::each::{self, HostRanges};
use super::exported::export;
use super::parse::parsed_tree;
use super::util::{THREE_VALUED, UNKNOWN};
use super::value::Reading;
use crate::code_rule_dsl::literal::{Grammar, Parsed, SubDoc, offsets};
use crate::code_rule_dsl::origin::Origin;
use crate::code_rule_dsl::transducer::semantic_facts::{Facts, Target, slot};
use crate::code_rule_dsl::value::Value;
use ast_grep_core::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{AstGrep, Doc, Node};
use std::borrow::Cow;
use std::collections::BTreeSet;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::tree_sitter::StrDoc;

/// What a family's format style reads of a call before any sub-rule matches (`Prepared`), one method per document
/// type the engine has; registered per style at load (`install_format_preparers`).
pub(crate) trait FormatPreparer: Send + Sync {
    fn prepare_tape<'t>(&self, format: &FormatArgs, env: &MetaVarEnv<'t, TapeDoc>) -> Prepared<'t, TapeDoc>;
    fn prepare_sub<'t>(&self, format: &FormatArgs, env: &MetaVarEnv<'t, SubDoc>) -> Prepared<'t, SubDoc>;
    fn prepare_str<'t>(
        &self,
        format: &FormatArgs,
        env: &MetaVarEnv<'t, StrDoc<GrammarDialect>>,
    ) -> Prepared<'t, StrDoc<GrammarDialect>>;
}

static PREPARERS: std::sync::OnceLock<Vec<(Style, &'static dyn FormatPreparer)>> = std::sync::OnceLock::new();

/// Registers each style's preparer (once, at load).
pub(crate) fn install_format_preparers(preparers: Vec<(Style, &'static dyn FormatPreparer)>) {
    let _ = PREPARERS.set(preparers);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Style {
    /// `str.format` templates (`braceFormat`).
    Brace,
    /// printf-style formats (`percentFormat`).
    Percent,
    /// PHP's printf-family formats (`printfFormat`).
    Printf,
    /// PHP's `sscanf`/`fscanf` formats (`scanfFormat`).
    Scanf,
    /// Rust's `format_args!`-family macro calls (`rustFormat`).
    Rust,
}

impl Style {
    pub(crate) fn grammar(self) -> Grammar {
        match self {
            Self::Brace => Grammar::expect("braceFormat"),
            Self::Percent => Grammar::expect("percentFormat"),
            Self::Printf => Grammar::expect("printfFormat"),
            Self::Scanf => Grammar::expect("scanfFormat"),
            Self::Rust => Grammar::expect("rustFormat"),
        }
    }
}

#[derive(Clone)]
pub(crate) struct FormatArgs {
    pub(crate) style: Style,
    /// The capture holding the format string.
    pub(crate) string: String,
    /// The capture holding the arguments: a call's argument list (a `$$$` capture), or the right
    /// operand of `%`.
    pub(crate) args: String,
    /// Matched on the host arguments no field consumes; binds into the host environment.
    pub(crate) unused: Option<AstMatcher>,
    /// Matched on the template's fields (placeholders and stars for `%`) no argument fills.
    pub(crate) missing: Option<AstMatcher>,
    /// The captures `missing` may bind, exported to the host.
    pub(crate) exports: Vec<String>,
    /// How a PHP format that is no string literal is read: its value through bindings (`printf`
    /// and `scanf`); `None` for Python's styles, which read a literal only.
    pub(crate) reading: Option<Reading>,
}

/// One side's candidates: nodes and whether each is known to be unused (or missing); `incomplete`
/// when others may be that the analysis cannot name (a `**` splat, a dynamic key).
pub(crate) struct Side<T> {
    candidates: Vec<(T, bool)>,
    incomplete: bool,
}

impl<T> Side<T> {
    fn known(candidates: Vec<(T, bool)>) -> Self {
        Self {
            candidates,
            incomplete: false,
        }
    }

    fn unknown() -> Self {
        Self {
            candidates: Vec::new(),
            incomplete: true,
        }
    }

    fn merge(mut self, other: Self) -> Self {
        self.candidates.extend(other.candidates);
        self.incomplete |= other.incomplete;
        self
    }
}

/// A template's fields: `(node id, key)`, the key an argument index or a keyword.
pub(crate) struct Fields {
    fields: Vec<(usize, Key)>,
}

#[derive(Clone, PartialEq)]
pub(crate) enum Key {
    Index(usize),
    Name(String),
}

fn key_of(value: Option<&Value>) -> Option<Key> {
    match value? {
        Value::Str(name) => Some(Key::Name(name.clone())),
        Value::Int(index) => usize::try_from(index).ok().map(Key::Index),
        _ => None,
    }
}

/// What a format call's arguments and template give before any sub-rule is matched.
pub(crate) enum Prepared<'t, D: Doc> {
    NoMatch,
    Unknown,
    Sides {
        tree: std::sync::Arc<crate::code_rule_dsl::literal::SubTree>,
        unused: Side<Node<'t, D>>,
        missing: Side<usize>,
    },
}

impl FormatArgs {
    /// The call's unused arguments and missing fields, read from the captures in `env`.
    fn prepare<'t, D: Doc>(&self, env: &MetaVarEnv<'t, D>) -> Prepared<'t, D> {
        use crate::code_rule_dsl::matcher::term::same;
        use std::any::TypeId;
        let Some(preparer) = PREPARERS
            .get()
            .and_then(|preparers| preparers.iter().find(|(style, _)| *style == self.style))
            .map(|(_, preparer)| *preparer)
        else {
            return Prepared::Unknown;
        };
        if TypeId::of::<D>() == TypeId::of::<TapeDoc>() {
            return unsafe { same(preparer.prepare_tape(self, same(env))) };
        }
        if TypeId::of::<D>() == TypeId::of::<SubDoc>() {
            return unsafe { same(preparer.prepare_sub(self, same(env))) };
        }
        if TypeId::of::<D>() == TypeId::of::<StrDoc<GrammarDialect>>() {
            return unsafe { same(preparer.prepare_str(self, same(env))) };
        }
        unreachable!("formatArgs over a document type the engine has not: {}", std::any::type_name::<D>())
    }

    pub(super) fn match_node<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &mut Cow<MetaVarEnv<'t, D>>,
    ) -> Option<Node<'t, D>> {
        let (tree, unused, missing) = match self.prepare(env) {
            Prepared::NoMatch => return None,
            Prepared::Unknown => {
                UNKNOWN.set(true);
                return None;
            }
            Prepared::Sides {
                tree,
                unused,
                missing,
            } => (tree, unused, missing),
        };
        let root = AstGrep::doc(SubDoc { tree: tree.clone() });
        let parsed = root.root();
        let three = THREE_VALUED.get();
        let saved = three && UNKNOWN.replace(false);
        let mut verdict = Some(true);
        let mut env_after = Cow::Borrowed(env.as_ref());
        if let Some(rule) = &self.unused {
            let found = first(rule, unused.candidates, &mut env_after);
            verdict = and(verdict, open(found, unused.incomplete));
        }
        if let Some(rule) = &self.missing
            && verdict != Some(false)
        {
            let nodes = parsed
                .dfs()
                .filter_map(|part| {
                    missing
                        .candidates
                        .iter()
                        .find(|(id, _)| *id == part.node_id())
                        .map(|(_, known)| (part, *known))
                })
                .collect();
            let mut inner = Cow::Owned(MetaVarEnv::new());
            let hosts = each::active().then(|| HostRanges::enter(tree.clone()));
            let found = first(rule, nodes, &mut inner);
            drop(hosts);
            if found == Some(true) {
                for name in &self.exports {
                    if let Some(capture) = inner.get_match(name)
                        && let Some(range) = tree.of_node(capture.node_id(), capture.range())
                    {
                        export(&mut env_after, name, &capture.text(), range.primary());
                    }
                }
            }
            verdict = and(verdict, open(found, missing.incomplete));
        }
        match verdict {
            Some(true) => {
                if three {
                    UNKNOWN.set(saved);
                }
                if let Cow::Owned(bound) = env_after {
                    *env = Cow::Owned(bound);
                }
                Some(node)
            }
            Some(false) => {
                if three {
                    UNKNOWN.set(saved);
                }
                None
            }
            None => {
                UNKNOWN.set(true);
                None
            }
        }
    }
}

/// A search's verdict when more candidates may exist than it saw: a miss is then unknown.
fn open(found: Option<bool>, incomplete: bool) -> Option<bool> {
    match found {
        Some(false) if incomplete => None,
        other => other,
    }
}

/// Three-valued `and`.
fn and(left: Option<bool>, right: Option<bool>) -> Option<bool> {
    match (left, right) {
        (Some(false), _) | (_, Some(false)) => Some(false),
        (Some(true), Some(true)) => Some(true),
        _ => None,
    }
}

/// Whether some candidate matches `rule`, binding from the first known one that does: `Some(true)`,
/// `None` when only unknown candidates (or unknown verdicts) could, `Some(false)` otherwise.
fn first<'t, D: Doc>(
    rule: &AstMatcher,
    candidates: Vec<(Node<'t, D>, bool)>,
    env: &mut Cow<MetaVarEnv<'t, D>>,
) -> Option<bool> {
    let mut unknown = false;
    for (candidate, known) in candidates {
        let mut trial = Cow::Borrowed(env.as_ref());
        UNKNOWN.set(false);
        let matched = rule.match_node_with_env(candidate, &mut trial).is_some();
        // `violation.each`: a candidate that binds the enumerated capture to a reported range is
        // passed over.
        if matched && each::active() && each::excluded(&trial) {
            UNKNOWN.set(false);
            continue;
        }
        if matched && known {
            UNKNOWN.set(false);
            if let Cow::Owned(bound) = trial {
                *env = Cow::Owned(bound);
            }
            return Some(true);
        }
        unknown |= matched || UNKNOWN.get();
        UNKNOWN.set(false);
    }
    (!unknown).then_some(false)
}

/// The expression inside parentheses, as Ruff's AST has none.
fn unparenthesized<'t, D: Doc>(mut node: Node<'t, D>) -> Node<'t, D> {
    while node.kind() == "parenthesized_expression" {
        let mut inner = node
            .named_code();
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}


/// The positional fields (placeholders and stars) whose index is at or past `count`.
fn positional_past(fields: &Fields, count: usize) -> Side<usize> {
    Side::known(
        fields
            .fields
            .iter()
            .filter(|(_, key)| matches!(key, Key::Index(index) if *index >= count))
            .map(|(id, _)| (*id, true))
            .collect(),
    )
}



pub(crate) mod python;
pub(crate) mod printf;
pub(crate) mod rust;
pub(crate) mod scanf;

impl FormatArgs {
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn facts<'t, D: Doc>(&self, env: &MetaVarEnv<'t, D>, facts: &mut Facts<'t, D>) {
        let (tree, unused, missing) = match self.prepare(env) {
            Prepared::NoMatch => return,
            Prepared::Unknown => {
                facts.gate(-1, Target::None);
                return;
            }
            Prepared::Sides {
                tree,
                unused,
                missing,
            } => (tree, unused, missing),
        };
        facts.gate(1, Target::Tree(tree.clone(), None));
        for (candidate, known) in unused.candidates {
            facts.push(slot::UNUSED, 0, i64::from(known), Target::Node(candidate));
        }
        let root = AstGrep::doc(SubDoc { tree: tree.clone() });
        for part in root.root().dfs() {
            if let Some((id, known)) = missing
                .candidates
                .iter()
                .find(|(id, _)| *id == part.node_id())
            {
                facts.push(
                    slot::MISSING,
                    0,
                    i64::from(*known),
                    Target::TreeNode(tree.clone(), *id),
                );
            }
        }
        facts.push(
            slot::INCOMPLETE,
            0,
            i64::from(unused.incomplete),
            Target::None,
        );
        facts.push(
            slot::INCOMPLETE,
            0,
            i64::from(missing.incomplete),
            Target::None,
        );
    }
}


