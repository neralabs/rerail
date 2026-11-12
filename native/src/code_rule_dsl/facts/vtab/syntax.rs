//! T's base relations served from a batch of core facts (decision 0008; 05 Phase 7 row 3): the
//! relations Typing-support's exporter (`spikes/datalog-engines/exporter/src/main.rs`) wrote as
//! rows, here read off `facts::core` columns when a statement filters them.
//!
//! The rows are T's: over each document's named nodes other than comments, in preorder,
//!
//! ```text
//! doc(d, path, module)   pkg(d, p)             node(d, n, kind, start, end_)
//! child(p, c, idx)       field(p, f, c)        tape(n, par, size, depth)
//! text(n, s)             intval(n, v)          op(n, s)
//! ```
//!
//! with `idx` a child's index among those nodes, `par` -1 at the root and `size` the descendants
//! among them. Every value is an integer: a node is `d << 32 | its granule's slot and its index there`
//! (`granules`; under `RERAIL_NODE_IDS=preorder`, `d << 32 | its preorder index`, where a subtree is the ids
//! `n + 1 ..= n + size`), so a document is an interval of ids; `d` is the caller's key for the
//! document (its path atom in the push path); `kind` is the kind atom (`core::kind_atom`), `f` the
//! field atom (`tag << 16 | field code`), and paths, modules, packages, identifier text and
//! operators are name atoms (C12), interned when the batch is built.
//!
//! `node`, `child`, `field` and `tape` are views over the core columns (no node rows, design topic
//! 2); `doc`, `pkg`, `text`, `intval` and `op` are small and kept as [`Relation`]s.
//!
//! That is T's projection ([`Shape::T`]), which T's programs and the programs that declare the base relations
//! themselves read. A program that includes `core.rr` reads `core.rr`'s ([`Shape::Core`], decision 0046, `core_rr`):
//! comments are nodes, `text` is every node's, `op` every anonymous token's, and the source-text, token and literal
//! relations are made. T's `op` is `op_t` in both.

use super::{Modules, Relation, Source};
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::core::{Facts, KIND_STRIDE, NONE, TEXT, kind_atom, kind_code};
use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::Connection;
use crate::code_rule_dsl::logic::front::ast::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

pub mod within;

/// The language's `within(n, a)`, `n` strictly below `a` (`within::Within`, tree mode).
pub const WITHIN: &str = "within";
/// `span_within(inner, outer)`, `outer`'s span holding `inner`'s (`within::Within`, span mode).
pub const SPAN_WITHIN: &str = "span_within";
pub mod core_rr;
pub mod stored_spike;
pub mod normalized;
// cratify: `python_syntax` is rerail-lang-python's (it mounts it)
// cratify: `python_demand` is rerail-lang-python's (it mounts it)
// cratify: `candidates` is rerail-lang-python's (it mounts it)
// cratify: `python_parts` is rerail-lang-python's (it mounts it)
use core_rr::Family;
pub mod guests;
pub mod granules;
use guests::GuestRows;

/// Which projection of the tree a batch serves.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Shape {
    /// T's: named nodes without comments, `text` of identifiers, `op` of operator fields.
    #[default]
    T,
    /// `core.rr`'s (decision 0046).
    Core,
}

/// The projection a run binds: its shape, and the relations its program reads (`None`: all). A relation no rule
/// reads is not made where making it costs (`text` of every node, the tokens); one the program derives is not bound
/// (`derived`).
#[derive(Clone, Debug, Default)]
pub struct Projection {
    pub shape: Shape,
    pub reads: Option<BTreeSet<String>>,
    /// The regexes the program runs over `source` or `src_line` spans (`front::Program::rx_spans`): their matches'
    /// groups are spans, rows of `rx_span`.
    pub rx_spans: Vec<(crate::code_rule_dsl::logic::front::ast::SpanIn, String)>,
    /// The AST patterns the program matches (`front::Program::ast_patterns`): rows of `ast_match`.
    pub ast_patterns: Vec<String>,
    /// Source/line spans whose complete text a rule reads, traced through copied relation columns.
    /// None is the unprojected test/export interface; AST subtree text keeps its independent bound.
    pub text_spans: Option<BTreeSet<crate::code_rule_dsl::logic::front::ast::SpanIn>>,
    /// The node kinds whose text a rule reads (`text_kind_demand`): their nodes' `text` is made at any length. Any
    /// other node's is made up to `core_rr::TEXT_INLINE` bytes, unless it is token-like, or `long_text_any`: some rule
    /// reads the text of a node of any kind.
    pub long_text: BTreeSet<String>,
    pub long_text_any: bool,
    /// The nodes whose `text` a rule reads (`TextDemand::read`): those of these kinds, or in these fields of their
    /// parent. `None` is every node.
    pub text_nodes: Option<TextNodes>,
    /// The grammars whose guests keep their partial tree when they do not parse (`.on guest_error partial`,
    /// decision 0055 §6); `"*"` is every grammar. Any other broken guest is withheld.
    pub partial: BTreeSet<String>,
    /// In T's shape, `syntax_error` is core.rr's `syntax_error(d, l, msg)`, CPython's verdict, not T's
    /// `syntax_error(d)`: the program declares that one (a program of core.rr's base relations, not T's).
    pub verdict: bool,
    /// The relations the program's rules derive: the batch binds none of them, made or viewed (T's programs derive
    /// `docof` from `node`, `rr::lower::projections`; a program may name an output as a base relation is named). A
    /// bound table of that name would be the planner's to drop.
    pub derived: BTreeSet<String>,
}

impl Projection {
    pub fn core() -> Self {
        Projection { shape: Shape::Core, reads: None, rx_spans: Vec::new(), ast_patterns: Vec::new(), text_spans: None, long_text: BTreeSet::new(), long_text_any: false, text_nodes: None, partial: BTreeSet::new(), verdict: false, derived: BTreeSet::new() }
    }
    fn wants(&self, rel: &str) -> bool {
        self.reads.as_ref().is_none_or(|r| r.contains(rel))
    }
    /// Whether a guest of `grammar` that does not parse keeps its partial tree.
    fn keeps_partial(&self, grammar: &str) -> bool {
        self.partial.contains("*") || self.partial.contains(grammar)
    }
    /// `core.rr`'s shape for a program that includes `core.rr` (declares every relation it declares as an input),
    /// T's for one that declares its base relations itself (T's programs, the converter's, O74).
    pub fn of(program: &crate::code_rule_dsl::logic::front::ast::Program, reads: BTreeSet<String>) -> Self {
        crate::code_rule_dsl::logic::profile::projection_plan(program, &reads);
        let core = crate::code_rule_dsl::facts::core::core_names();
        let shape = if !core.is_empty() && core.iter().all(|r| program.decls.contains_key(r)) { Shape::Core } else { Shape::T };
        let partial = program.facts.get(GUEST_POLICY).into_iter().flatten().filter_map(|row| match row.as_slice() {
            [Value::Str(k), Value::Str(v)] if v == "partial" => k.strip_prefix("errors").map(|g| g.strip_prefix(':').unwrap_or("*").to_owned()),
            _ => None,
        });
        let verdict = shape == Shape::T && program.decls.get("syntax_error").is_some_and(|cols| cols.len() == 3);
        let derived: BTreeSet<String> = program.rules.iter().map(|r| r.head.rel.to_string()).collect();
        let reads = reads.into_iter().filter(|r| !derived.contains(r)).collect();
        Projection { shape, reads: Some(reads), rx_spans: program.rx_spans.iter().cloned().collect(), ast_patterns: program.ast_patterns.iter().cloned().collect(), text_spans: Some(span_text_demand(program)), long_text: BTreeSet::new(), long_text_any: false, text_nodes: None, partial: partial.collect(), verdict, derived }
    }

    /// This projection, with `text` made at any length for the node kinds whose text the program's rules read
    /// ([`text_kind_demand`]; `matchers` are the program's `ast` relations, by relation, as `matchers::lower` gives them).
    pub fn with_long_text(mut self, program: &crate::code_rule_dsl::logic::front::ast::Program, matchers: &[(String, String, Vec<String>)]) -> Self {
        let TextDemand { long, any, read, wide } = text_kind_demand(program, matchers);
        self.long_text = long;
        self.long_text_any = !any.is_empty();
        crate::code_rule_dsl::logic::profile::decision("text past the inline bound", || {
            if self.long_text_any { format!("every node ({} rules read the text of a node of any kind)", any.len()) } else { format!("{} kinds: {}", self.long_text.len(), self.long_text.iter().take(12).cloned().collect::<Vec<_>>().join(", ")) }
        });
        crate::code_rule_dsl::logic::profile::decision("nodes with text", || match &read {
            None => format!("every node ({} reads the text of a node it does not narrow)", wide.as_deref().unwrap_or("a rule")),
            Some(TextNodes { kinds, fields }) => format!("{} kinds ({}), and the nodes in {} fields ({})", kinds.len(), kinds.iter().take(8).cloned().collect::<Vec<_>>().join(", "), fields.len(), fields.iter().take(8).cloned().collect::<Vec<_>>().join(", ")),
        });
        if !std::env::var("RERAIL_TEXT_NODES").is_ok_and(|v| v == "all") {
            self.text_nodes = read;
        }
        self
    }

    /// The columns the batch binds base relation `rel` with (`select::input_columns`'), but T's `syntax_error(d)`
    /// where the program declares that one and not core.rr's verdict (`verdict`): the cross-file part binds a base
    /// relation with the chunks' columns, on the same connection.
    pub fn columns(&self, rel: &str) -> Option<&'static [&'static str]> {
        if rel == "syntax_error" && self.shape == Shape::T && !self.verdict {
            return Some(&["d"]);
        }
        crate::code_rule_dsl::facts::core::input_columns(rel)
    }

    fn wants_span_text(&self, within: crate::code_rule_dsl::logic::front::ast::SpanIn) -> bool {
        use crate::code_rule_dsl::logic::front::ast::SpanIn;
        self.wants("text") && self.text_spans.as_ref().map_or_else(
            || self.wants(match within { SpanIn::Source => "source", SpanIn::Line => "src_line", _ => "text" }),
            |spans| spans.contains(&within),
        )
    }
}

/// The nodes whose text a rule reads: those of `kinds`, and those in a field of `fields` of their parent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextNodes {
    pub kinds: BTreeSet<String>,
    pub fields: BTreeSet<String>,
}

/// What the rules read of `text` (`text_kind_demand`).
pub struct TextDemand {
    /// The kinds whose text is read at any length.
    long: BTreeSet<String>,
    /// The text atoms (rule, body position) of a node of any kind, no twin reading its long text.
    pub any: Vec<(usize, usize)>,
    /// The nodes whose text (within the bound) is read; `None` every node.
    pub read: Option<TextNodes>,
    /// Where `read` is every node: the first rule that reads the text of a node it does not narrow.
    wide: Option<String>,
}

/// Whether `v` is a node in a field whose every kind is a leaf (`field(x, "attribute", v)`): a token's text, which
/// `text` holds at any length (`core_rr::TEXT_INLINE`).
fn leaf_field(body: &[crate::code_rule_dsl::logic::front::ast::Literal], v: &str) -> bool {
    use crate::code_rule_dsl::logic::front::ast::Term;
    use crate::code_rule_dsl::facts::grammar_shape::{field_kinds, leaf_kind};
    body.iter().filter_map(crate::code_rule_dsl::logic::front::ast::Literal::atom).filter(|a| !a.neg && a.rel == "field").any(|a| match (a.args.get(1), a.args.get(2)) {
        (Some(Term::Str(f)), Some(Term::Var(x))) if x == v => field_kinds(f).is_some_and(|k| !k.is_empty() && k.iter().all(|k| leaf_kind(k))),
        _ => false,
    })
}

/// Whether `text(v, "s")` names a text within the bound: no long text equals it.
fn short_constant(text: &crate::code_rule_dsl::logic::front::ast::Atom) -> bool {
    matches!(text.args.get(1), Some(crate::code_rule_dsl::logic::front::ast::Term::Str(s)) if s.len() <= core_rr::TEXT_INLINE)
}

/// Rules reading `text(v, t)` of a node `v` whose kind no body fixes see `text` within its bound only
/// (`core_rr::TEXT_INLINE`): making every node's text at any length costs a run 40% more work (decision 0109). Each
/// such rule gets twins, one per non-empty set of those atoms, reading `t = @rr_node_text(v)` in their place: the
/// text of the nodes the rule binds, when past the bound (`long_node_text`). The rule and its twins together read every
/// text, each once. More than `MAX` such atoms in one rule keep the any-kind projection instead. Returns the twins made.
pub fn long_text_twins(program: &mut crate::code_rule_dsl::logic::front::ast::Program, matchers: &[(String, String, Vec<String>)]) -> usize {
    use crate::code_rule_dsl::logic::front::ast::{Atom, Literal, Term};
    const MAX: usize = 3;
    let TextDemand { any, .. } = text_kind_demand(program, matchers);
    let mut by_rule: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    // A twin binds `v` by the rest of its body: an atom whose `text` alone binds `v` (`list_constant(v) :- text(v, s),
    // glob(…, s)`) is not twinned, and its rule keeps the any-kind projection.
    let bound = |r: usize, i: usize| {
        let body = &program.rules[r].body;
        let Some(Literal::Atom(text)) = body.get(i) else { return false };
        let Some(Term::Var(v)) = text.args.first() else { return false };
        body.iter().enumerate().filter(|(j, _)| *j != i).filter_map(|(_, l)| l.atom()).filter(|a| !a.neg && a.rel != "text").any(|a| a.args.iter().any(|t| matches!(t, Term::Var(x) if x == v)))
    };
    for (r, i) in any.into_iter().filter(|&(r, i)| bound(r, i)) {
        by_rule.entry(r).or_default().push(i);
    }
    // Each rule's twins follow it, so a rule's place does not depend on what other rules the program holds.
    let mut twins: std::collections::BTreeMap<usize, Vec<crate::code_rule_dsl::logic::front::ast::Rule>> = Default::default();
    let mut fresh = 0usize;
    for (r, atoms) in by_rule.into_iter().filter(|(_, a)| a.len() <= MAX) {
        for set in 1..(1u32 << atoms.len()) {
            let mut twin = program.rules[r].clone();
            for (bit, &i) in atoms.iter().enumerate().filter(|(b, _)| set >> b & 1 == 1) {
                let Literal::Atom(text) = &twin.body[i] else { continue };
                let v = text.args[0].clone();
                let t = match &text.args[1] {
                    Term::Var(t) if t != "_" => Term::Var(t.clone()),
                    _ => {
                        fresh += 1;
                        Term::Var(format!("rr_long_text{fresh}_{bit}"))
                    }
                };
                twin.body[i] = Literal::Cmp { l: t, op: "=", r: Term::Call("rr_node_text".into(), vec![v.clone()]) };
                // Expose the sparse domain to the join planner before evaluating the text functor.
                twin.body.push(Literal::Atom(Atom { rel: "rr_long_text_node".into(), args: vec![v], neg: false, gated: false }));
            }
            twins.entry(r).or_default().push(twin);
        }
    }
    let made = twins.values().map(Vec::len).sum();
    if made > 0 {
        program.decls.insert("rr_long_text_node".into(), vec![("n".into(), "node".into())]);
        if !program.inputs.iter().any(|r| r == "rr_long_text_node") {
            program.inputs.push("rr_long_text_node".into());
        }
        let rules = std::mem::take(&mut program.rules);
        for (r, rule) in rules.into_iter().enumerate() {
            program.rules.push(rule);
            program.rules.extend(twins.remove(&r).unwrap_or_default());
        }
    }
    made
}

/// The batches whose documents' texts `long_node_text` reads, by document key: the latest batch to hold each
/// document, weakly (a chunk's batch is dropped when its run is done; its entries then read nothing).
static TEXTS: std::sync::LazyLock<std::sync::RwLock<HashMap<u32, (std::sync::Weak<Batch>, usize)>>> = std::sync::LazyLock::new(Default::default);

/// Hold `batch`'s documents for `long_node_text`.
fn hold_texts(batch: &Arc<Batch>) {
    let mut texts = TEXTS.write().unwrap_or_else(|p| p.into_inner());
    // Entries of dropped batches go once they are as many as the live ones.
    if texts.len() > 4096 && texts.len() > 2 * batch.docs.len() {
        texts.retain(|_, (b, _)| b.strong_count() > 0);
    }
    for (i, p) in batch.docs.iter().enumerate() {
        texts.insert(p.key, (Arc::downgrade(batch), i));
    }
}

/// Node `n`'s text when it is past `text`'s inline bound (`core_rr::TEXT_INLINE`), read from its document in the
/// batch that holds it; none for a shorter node (whose text `text` holds), a span, or a document no batch holds.
/// Only the nodes a rule binds are read and interned (decision 0108 §1).
pub(crate) fn long_node_text(n: i64) -> Option<String> {
    let (key, low) = ((n >> 32) as u32, n as u32);
    let (batch, i) = TEXTS.read().unwrap_or_else(|p| p.into_inner()).get(&key).cloned()?;
    let batch = batch.upgrade()?;
    let p = &batch.docs[i];
    let local = p.local_of(low)?;
    let core = *p.core.get(local as usize)?;
    let text = p.facts.text(core);
    (text.len() > core_rr::TEXT_INLINE).then(|| text.to_owned())
}

/// The node kinds whose text a rule reads, and whether some rule reads the text of a node of any kind. A node's kinds
/// are followed through the relations' columns to a fixpoint: a column of a derived relation holds the kinds its
/// rules' head variables can have, each the intersection of what its body fixes (`kind(v, "K")`, a matcher atom's
/// root, a column whose kinds are known), the union over its rules. An input's column is any kind. A positive
/// `text(v, …)` then demands `v`'s kinds (`core_rr::TEXT_INLINE` holds below them only). Fixed by the same body only,
/// `text(w, t)` over `classes(c, _), child(w, c, _), kind(w, "decorated_definition")` was demanded, but a rule reading
/// `text(n, t)` of `nodes(n)` was not, and saw no text past the bound.
pub fn text_kind_demand(program: &crate::code_rule_dsl::logic::front::ast::Program, matchers: &[(String, String, Vec<String>)]) -> TextDemand {
    use crate::code_rule_dsl::logic::front::ast::{Literal, Term};
    // A kind as the program holds it (`python:call`) or as a grammar names it (`call`).
    let bare = |k: &str| k.rsplit(':').next().unwrap_or(k).to_owned();
    // A matcher's root kind: its rule's own `kind`, or one of its top-level `all`'s.
    let root_kind = |text: &str| -> Option<String> {
        let rule = serde_json::from_str::<serde_json::Value>(text).ok()?.get("rule")?.clone();
        if let Some(k) = rule.get("kind").and_then(|k| k.as_str()) {
            return Some(k.to_owned());
        }
        rule.get("all")?.as_array()?.iter().find_map(|r| r.get("kind").and_then(|k| k.as_str()).map(str::to_owned))
    };
    // `None` is any kind.
    type Kinds = Option<BTreeSet<String>>;
    let roots: HashMap<&str, Kinds> = matchers.iter().map(|(rel, text, _)| (rel.as_str(), root_kind(text).map(|k| BTreeSet::from([bare(&k)])))).collect();
    let derived: BTreeSet<&str> = program.rules.iter().map(|r| r.head.rel.as_str()).collect();
    let mut columns: HashMap<(String, usize), Kinds> = HashMap::new();
    let column = |columns: &HashMap<(String, usize), Kinds>, rel: &str, c: usize| -> Kinds {
        if derived.contains(rel) {
            columns.get(&(rel.to_owned(), c)).cloned().unwrap_or(Some(BTreeSet::new()))
        } else if c == 0 && let Some(root) = roots.get(rel) {
            root.clone()
        } else {
            None
        }
    };
    let of_var = |columns: &HashMap<(String, usize), Kinds>, body: &[Literal], v: &str| -> Kinds {
        let mut kinds: Kinds = None;
        for a in body.iter().filter_map(Literal::atom).filter(|a| !a.neg) {
            for (c, t) in a.args.iter().enumerate() {
                if !matches!(t, Term::Var(x) if x == v) {
                    continue;
                }
                let fixed = match (a.rel.as_str(), c, a.args.get(1)) {
                    ("kind", 0, Some(Term::Str(k))) => Some(BTreeSet::from([bare(k)])),
                    ("kind", _, _) => None,
                    (rel, c, _) => column(columns, rel, c),
                };
                kinds = match (kinds, fixed) {
                    (None, k) => k,
                    (k, None) => k,
                    (Some(a), Some(b)) => Some(a.intersection(&b).cloned().collect()),
                };
            }
        }
        kinds
    };
    loop {
        let mut changed = false;
        for rule in &program.rules {
            for (c, t) in rule.head.args.iter().enumerate() {
                let Term::Var(v) = t else { continue };
                let kinds = of_var(&columns, &rule.body, v);
                let entry = columns.entry((rule.head.rel.to_string(), c)).or_insert(Some(BTreeSet::new()));
                let joined = match (&*entry, kinds) {
                    (None, _) | (_, None) => None,
                    (Some(a), Some(b)) => Some(a.union(&b).cloned().collect()),
                };
                if *entry != joined {
                    *entry = joined;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    // A rule (by head relation) that reads `v`'s long text by `@rr_node_text(v)`: its twin (`long_text_twins`).
    let twinned = |head: &str, v: &str| {
        program.rules.iter().filter(|r| r.head.rel == head).any(|r| {
            r.body.iter().any(|l| matches!(l, Literal::Cmp { r: Term::Call(f, args), .. } if f == "rr_node_text" && matches!(args.first(), Some(Term::Var(x)) if x == v)))
        })
    };
    // Whether `v` is a span (a `span` column binds it): its text is a span's (`text_spans`), not a node's.
    let span = |body: &[Literal], v: &str| {
        body.iter().filter_map(Literal::atom).filter(|a| !a.neg && a.rel != "text").any(|a| {
            let types = program.decls.get(a.rel.as_str());
            a.args.iter().enumerate().any(|(c, t)| matches!(t, Term::Var(x) if x == v) && types.and_then(|ts| ts.get(c)).is_some_and(|(_, ty)| ty == "span"))
        })
    };
    // A matcher capture's node: the kinds and fields its matcher gives it (`{"pattern": "$N", "field": "name"}`, or a
    // `kind` or `field` beside the pattern in an `all`). Columns past the root are the captures, in order. An
    // `inside`'s `field` is where the node it holds sits in that ancestor, not the ancestor's own.
    let captured = |body: &[Literal], v: &str| -> (BTreeSet<String>, BTreeSet<String>) {
        fn own(o: &serde_json::Map<String, serde_json::Value>, ancestor: bool, kinds: &mut BTreeSet<String>, fields: &mut BTreeSet<String>) {
            kinds.extend(o.get("kind").and_then(|k| k.as_str()).map(str::to_owned));
            if !ancestor {
                fields.extend(o.get("field").and_then(|k| k.as_str()).map(str::to_owned));
            }
        }
        // The constraints on the node `$name` matches, read where its pattern sits.
        type Outer<'a> = Option<(&'a serde_json::Map<String, serde_json::Value>, bool)>;
        fn find(v: &serde_json::Value, name: &str, ancestor: bool, outer: Outer<'_>, out: &mut Option<(BTreeSet<String>, BTreeSet<String>)>) {
            match v {
                serde_json::Value::Object(o) => {
                    if o.get("pattern").and_then(|p| p.as_str()) == Some(name) {
                        let (mut kinds, mut fields) = (BTreeSet::new(), BTreeSet::new());
                        for (m, ancestor) in [Some((o, ancestor)), outer].into_iter().flatten() {
                            own(m, ancestor, &mut kinds, &mut fields);
                            for sib in m.get("all").and_then(|a| a.as_array()).into_iter().flatten().filter_map(|x| x.as_object()) {
                                own(sib, false, &mut kinds, &mut fields);
                            }
                        }
                        // Two sites of one capture: either constrains it, or none does.
                        *out = Some(match out.take() {
                            None => (kinds, fields),
                            Some((k, f)) if k.is_empty() && f.is_empty() || kinds.is_empty() && fields.is_empty() => Default::default(),
                            Some((k, f)) => (k.union(&kinds).cloned().collect(), f.union(&fields).cloned().collect()),
                        });
                    }
                    for (key, x) in o {
                        if key == "all" {
                            for m in x.as_array().into_iter().flatten() {
                                find(m, name, false, Some((o, ancestor)), out);
                            }
                        } else {
                            find(x, name, key == "inside", None, out);
                        }
                    }
                }
                serde_json::Value::Array(a) => a.iter().for_each(|x| find(x, name, false, None, out)),
                _ => {}
            }
        }
        let mut got: Option<(BTreeSet<String>, BTreeSet<String>)> = None;
        for a in body.iter().filter_map(Literal::atom).filter(|a| !a.neg) {
            let Some((_, text, names)) = matchers.iter().find(|(rel, _, _)| *rel == a.rel) else { continue };
            for (c, t) in a.args.iter().enumerate().skip(1) {
                if !matches!(t, Term::Var(x) if x == v) {
                    continue;
                }
                let Some(name) = names.get(c - 1) else { continue };
                let mut out = None;
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(text) {
                    find(&json, &format!("${name}"), false, None, &mut out);
                }
                let (kinds, fields) = out.unwrap_or_default();
                if kinds.is_empty() && fields.is_empty() {
                    continue;
                }
                // Bound by two captures: the narrower is enough, the first is kept.
                got.get_or_insert((kinds.iter().map(|k| bare(k)).collect(), fields));
            }
        }
        got.unwrap_or_default()
    };
    // The fields `v` is a node in (`field(_, "F", v)`).
    let fields_of = |body: &[Literal], v: &str| -> BTreeSet<String> {
        body.iter().filter_map(Literal::atom).filter(|a| !a.neg && a.rel == "field").filter_map(|a| match (a.args.get(1), a.args.get(2)) {
            (Some(Term::Str(f)), Some(Term::Var(x))) if x == v => Some(f.clone()),
            _ => None,
        }).collect()
    };
    let (mut kinds, mut any) = (BTreeSet::new(), Vec::new());
    let mut read = Some(TextNodes::default());
    let mut wide = None;
    for (r, rule) in program.rules.iter().enumerate() {
        for (i, l) in rule.body.iter().enumerate() {
            let Literal::Atom(text) = l else { continue };
            if text.neg || text.rel != "text" {
                continue;
            }
            let Some(Term::Var(v)) = text.args.first() else {
                read = None;
                wide.get_or_insert_with(|| rule.head.rel.to_string());
                continue;
            };
            let fixed = of_var(&columns, &rule.body, v);
            if let Some(nodes) = &mut read {
                match &fixed {
                    Some(k) => nodes.kinds.extend(k.iter().cloned()),
                    None if span(&rule.body, v) => {}
                    None => match (fields_of(&rule.body, v), captured(&rule.body, v)) {
                        (f, _) if !f.is_empty() => nodes.fields.extend(f),
                        (_, (k, _)) if !k.is_empty() => nodes.kinds.extend(k),
                        (_, (_, f)) if !f.is_empty() => nodes.fields.extend(f),
                        _ => {
                            wide = Some(format!("{}, reading text({v}, …)", rule.head.rel).into());
                            read = None
                        }
                    },
                }
            }
            match fixed {
                Some(k) => kinds.extend(k),
                None if leaf_field(&rule.body, v) || short_constant(text) || twinned(&rule.head.rel, v) => {}
                None => any.push((r, i)),
            }
        }
    }
    TextDemand { long: kinds, any, read, wide }
}

/// Plan-time provenance, not a per-file scan. Only copying a span (including variable equality)
/// carries its origin. Regex captures have their own bounded ranges and do not demand the container text.
fn span_text_demand(program: &crate::code_rule_dsl::logic::front::ast::Program) -> BTreeSet<crate::code_rule_dsl::logic::front::ast::SpanIn> {
    use crate::code_rule_dsl::logic::front::ast::{Literal, SpanIn, Term};
    let mut columns: HashMap<(String, usize), u8> = [( ("source".to_owned(), 1), 1), (("src_line".to_owned(), 2), 2)].into();
    let mut requested = 0;
    loop {
        let mut changed = false;
        for rule in &program.rules {
            let mut vars: HashMap<String, u8> = HashMap::new();
            for atom in rule.body.iter().filter_map(Literal::atom).filter(|atom| !atom.neg) {
                for (column, term) in atom.args.iter().enumerate() {
                    if let Term::Var(value) = term {
                        *vars.entry(value.clone()).or_default() |= columns.get(&(atom.rel.to_string(), column)).copied().unwrap_or(0);
                    }
                }
            }
            loop {
                let mut aliases_changed = false;
                for literal in &rule.body {
                    if let Literal::Cmp { l: Term::Var(left), op: "=", r: Term::Var(right) } = literal {
                        let mask = vars.get(left).copied().unwrap_or(0) | vars.get(right).copied().unwrap_or(0);
                        for value in [left, right] {
                            let entry = vars.entry(value.clone()).or_default();
                            aliases_changed |= *entry != (*entry | mask);
                            *entry |= mask;
                        }
                    }
                }
                if !aliases_changed { break; }
            }
            for (column, term) in rule.head.args.iter().enumerate() {
                if let Term::Var(value) = term {
                    let mask = vars.get(value).copied().unwrap_or(0);
                    if mask != 0 {
                        let entry = columns.entry((rule.head.rel.to_string(), column)).or_default();
                        changed |= *entry != (*entry | mask);
                        *entry |= mask;
                    }
                }
            }
            for atom in rule.body.iter().filter_map(Literal::atom).filter(|atom| atom.rel == "text") {
                if let Some(Term::Var(value)) = atom.args.first() {
                    requested |= vars.get(value).copied().unwrap_or(0);
                }
            }
        }
        if !changed { break; }
    }
    [(SpanIn::Source, 1), (SpanIn::Line, 2)].into_iter().filter_map(|(span, bit)| (requested & bit != 0).then_some(span)).collect()
}

/// The relation a program's guest settings are facts of (`.guests`, `.on guest_error`; decision 0055):
/// `guest_policy("library", "on" | "off")`, `guest_policy("errors" | "errors:GRAMMAR", "skip" | "partial")`.
pub const GUEST_POLICY: &str = "guest_policy";

/// The first guest document's key: files take `1..GUEST_KEYS` (`corpus::memo::doc_keys`), guests in the run's
/// language `GUEST_KEYS..HIDDEN_KEYS`, and every other document (a guest in another grammar, a value) `HIDDEN_KEYS..2^31`
/// (decision 0058 §1: what a rule sees is a bound on these keys).
pub const GUEST_KEYS: u32 = 1 << 30;
pub const HIDDEN_KEYS: u32 = (1 << 30) + (1 << 29);

/// A guest document's place (decision 0055): the host document and the node or span it stands at, its grammar and
/// role, its root, and how its bytes map to the host's.
#[derive(Clone, Debug)]
pub struct Guest {
    pub host: u32,
    /// `at`: the host's node or span (`host << 32 | local`).
    pub at: i64,
    pub grammar: String,
    /// "program" or "fragment" (`doc_role`).
    pub role: String,
    /// The core id of the guest's root: a fragment's expression inside its wrapper, else the tree's root. The
    /// document is that node's subtree.
    pub root: u32,
    /// Where its bytes stand in its host's text (`host_node`).
    pub place: Placement,
    /// Where they stand in its file's text: `place` composed with its host's `in_file` when the host is a guest
    /// too (a string inside a parsed annotation). Its positions are these.
    pub in_file: Placement,
    /// What made it: a host's code, or a value a rule computed (decision 0058 §4).
    pub made: Made,
}

/// What made a guest document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Made {
    /// The code at `at` in its host (`embed`, `parsed`, `sub_tape`).
    Host,
    /// `@parse(grammar, text)`: a value document, with no host; its places are its own text's.
    Parse { text: String },
    /// `@splice(tape, node, with)`: tape `tape` with node `node`'s text replaced by tape `with`'s, parsed in its
    /// grammar; a value document.
    Splice { tape: String, node: i64, with: String },
}

/// A document's text as a rule gets it back (`@unparse`): a wrapped fragment's root's text (the expression inside
/// `( … )`), else the whole text.
pub fn unparsed(d: &Document) -> &str {
    match d.guest.as_deref() {
        Some(g) if g.root != 0 => d.facts.text(g.root),
        _ => d.facts.source.as_str(),
    }
}

/// A document's tape: its content key, hex (`tape_of`).
pub fn tape_hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where a guest's bytes stand in its host's source.
#[derive(Clone, Debug)]
pub enum Placement {
    /// A copied stretch: guest byte `b` is host byte `b + delta`, kept within the host bytes `within` (a wrapper's
    /// written `(` and `)` stand at the stretch's ends).
    Copied { delta: i64, within: [u32; 2] },
    /// A normalized string (decision 0058 §3): its stretches (`normalized::Piece`, host bytes absolute, sorted, the
    /// last ending the text) copied byte for byte, an escape's output standing at the escape; kept within `within`.
    Mapped { pieces: Arc<[normalized::Piece]>, within: [u32; 2] },
    /// A decoded value with no map: every node stands at the host bytes `at`.
    At([u32; 2]),
}

impl Placement {
    pub(crate) fn host(&self, [a, b]: [u32; 2]) -> [u32; 2] {
        let clamp = |x: i64, [lo, hi]: [u32; 2]| x.clamp(i64::from(lo), i64::from(hi)) as u32;
        match self {
            Placement::Copied { delta, within } => [clamp(i64::from(a) + delta, *within), clamp(i64::from(b) + delta, *within)],
            Placement::Mapped { pieces, within } => {
                let start = clamp(mapped_start(pieces, a), *within);
                if b <= a { [start, start] } else { [start, clamp(mapped_end(pieces, b), *within).max(start)] }
            }
            Placement::At(at) => *at,
        }
    }

    /// This placement's stretches: a copied stretch is one, unbounded.
    fn pieces(&self) -> Option<Vec<normalized::Piece>> {
        match self {
            Placement::Copied { delta, .. } => Some(vec![normalized::Piece { d: 0, s: *delta, escape: false }]),
            Placement::Mapped { pieces, .. } => Some(pieces.to_vec()),
            Placement::At(_) => None,
        }
    }

    /// `inner`, a placement in this one's text, as a placement in this one's host's.
    pub fn then(&self, inner: Placement) -> Placement {
        match (self, &inner) {
            (Placement::At(at), _) => Placement::At(*at),
            (_, Placement::At(at)) => Placement::At(self.host(*at)),
            (Placement::Copied { delta, .. }, Placement::Copied { delta: d, within }) => Placement::Copied { delta: delta + d, within: self.host(*within) },
            _ => {
                let (Some(outer), Some(pieces), Placement::Copied { within, .. } | Placement::Mapped { within, .. }) = (self.pieces(), inner.pieces(), &inner) else {
                    unreachable!("At is matched above")
                };
                // Cut where either map cuts: the inner's stretches, and the outer's, pulled back through a copied
                // inner stretch.
                let mut cuts: Vec<u32> = std::iter::once(0).chain(pieces.iter().map(|p| p.d)).collect();
                for (i, p) in pieces.iter().enumerate().filter(|(_, p)| !p.escape) {
                    let hi = pieces.get(i + 1).map_or(i64::MAX, |n| i64::from(n.d));
                    for o in &outer {
                        let x = i64::from(o.d) - p.s + i64::from(p.d);
                        if x >= i64::from(p.d) && x < hi {
                            cuts.push(x as u32);
                        }
                    }
                }
                cuts.sort_unstable();
                cuts.dedup();
                let escape_at = |ps: &[normalized::Piece], x: i64| ps.iter().rev().find(|p| i64::from(p.d) <= x).is_some_and(|p| p.escape);
                let composed: Vec<normalized::Piece> = cuts
                    .into_iter()
                    .map(|x| {
                        let mid = mapped_start(&pieces, x);
                        normalized::Piece { d: x, s: mapped_start(&outer, mid.max(0) as u32), escape: escape_at(&pieces, i64::from(x)) || escape_at(&outer, mid) }
                    })
                    .collect();
                Placement::Mapped { pieces: composed.into(), within: self.host(*within) }
            }
        }
    }
}

/// Where decoded byte `x` starts in the host: its stretch's source, plus its offset when the stretch is copied.
fn mapped_start(pieces: &[normalized::Piece], x: u32) -> i64 {
    let i = pieces.partition_point(|p| p.d <= x).saturating_sub(1);
    match pieces.get(i) {
        Some(p) if p.escape && pieces.get(i + 1).is_some_and(|n| n.d > x) => p.s,
        Some(p) => p.s + i64::from(x) - i64::from(p.d),
        None => i64::from(x),
    }
}

/// Where the decoded text ending before byte `x` ends in the host: an escape's whole source.
fn mapped_end(pieces: &[normalized::Piece], x: u32) -> i64 {
    let i = pieces.partition_point(|p| p.d < x).saturating_sub(1);
    match (pieces.get(i), pieces.get(i + 1)) {
        (Some(p), Some(n)) if p.escape => n.s,
        (Some(p), _) => p.s + i64::from(x) - i64::from(p.d),
        (None, _) => i64::from(x),
    }
}

/// One document of a batch.
#[derive(Clone)]
pub struct Document {
    /// `d`: the document's key, its path atom in the push path.
    pub key: u32,
    /// The path relative to its root, from which `module` and `pkg` follow (T's `module`).
    pub path: String,
    /// The `#core:` name the facts were built for, and their digest (`core::digest`).
    pub core: String,
    pub digest: [u8; 32],
    pub facts: Arc<Facts>,
    /// The tape the facts were read off and the file it parsed, which W's values, f-string trees and string annotations
    /// read (`taped`), and the program's matchers run over (decision 0106); kept only for a file of a program that
    /// reads one of them or runs matchers, else `None` (a guest's is `None`).
    pub tape: Option<(crate::code_rule_dsl::tape::TapeDoc, Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>)>,
    /// A guest's place in its host (decision 0055); `None` for a file's own tree. A guest's `path` is its host's.
    pub guest: Option<Arc<Guest>>,
    /// The module a loaded document is (t21 E2: `load(path, module, …)`, the module its resolution gave); `None` for
    /// a file of the corpus, whose module follows from its path.
    pub module: Option<Arc<str>>,
}

/// W's values (`Extra`) and the AST patterns' matches (`ast_match`, matched over the tape), which a batch reads off
/// its documents' tapes.
const VALUES_TAPED: [&str; 5] = ["value", "static_value", "value_type_of", "value_type", "ast_match"];

/// Whether `rel` is read off a document's tape, for which a batch's documents keep their tapes (`Document::tape`):
/// W's values (`Extra`), the AST patterns' matches, and a registered producer's (`Producer::taped`: Python's f-string trees and string
/// annotations).
pub fn taped(rel: &str) -> bool {
    VALUES_TAPED.contains(&rel) || producers().iter().any(|producer| producer.taped().contains(&rel))
}

/// A family's producer of relations over a batch's documents (Python's f-string trees, CPython's syntax, string
/// annotations), registered at load by the crate that knows the family: the batch runs every registered producer
/// over every document, telling it whether the document is of its dialect.
pub trait Producer: Send + Sync {
    /// The dialect whose documents it reads.
    fn dialect(&self) -> crate::code_rule_dsl::language::GrammarDialect;
    /// The relations it reads off a document's tape.
    fn taped(&self) -> &'static [&'static str] {
        &[]
    }
    /// A run over one batch whose program reads `reads` (with `markers`, the program's interpolation markers).
    fn start(&self, reads: &BTreeSet<String>, markers: &[String]) -> Box<dyn ProducerRun>;
}

/// A [`Producer`]'s run over one batch.
pub trait ProducerRun {
    /// Document `p`'s rows (`d` its document, `dialect` its grammar's kinds, `own`: it is of the producer's dialect).
    fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, own: bool, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<()>;
    /// The relations it made, with `atom` mapping a string's index to its atom.
    fn relations(self: Box<Self>, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, Relation)>;
    /// Whether it reads guests' documents too (else only files').
    fn reads_guests(&self) -> bool {
        false
    }
}

static PRODUCERS: std::sync::OnceLock<Vec<&'static dyn Producer>> = std::sync::OnceLock::new();

/// Registers the families' producers (once, at load), in the order their relations are listed.
pub fn install_producers(producers: Vec<&'static dyn Producer>) {
    let _ = PRODUCERS.set(producers);
}

fn producers() -> &'static [&'static dyn Producer] {
    PRODUCERS.get().map_or(&[], Vec::as_slice)
}

/// A comment's text without its marker (core.rr's `comment`): `#`, `--`, `//` (and a doc comment's `/` or `!`),
/// `/*` to `*/` (and a doc comment's `*` or `!`) or `<!--` to `-->` taken off, then the whitespace around the rest.
pub fn comment_body(text: &str) -> &str {
    let body = if let Some(rest) = text.strip_prefix("/*") {
        let rest = rest.strip_suffix("*/").unwrap_or(rest);
        rest.strip_prefix(['*', '!']).unwrap_or(rest)
    } else if let Some(rest) = text.strip_prefix("//") {
        rest.strip_prefix(['/', '!']).unwrap_or(rest)
    } else if let Some(rest) = text.strip_prefix("<!--") {
        rest.strip_suffix("-->").unwrap_or(rest)
    } else {
        text.strip_prefix('#').or_else(|| text.strip_prefix("--")).unwrap_or(text)
    };
    body.trim()
}

/// A file's nodes as `shape` numbers them: each local id's core id and kind name, and the way from a node id's low
/// 32 bits to its local id (they differ when nodes are numbered per definition, `DefIds`). For making guests from
/// requests that name host nodes (decision 0055, `corpus::guests`).
pub struct Numbering {
    pub nodes: Vec<(u32, &'static str)>,
    /// `(low, local)`, ascending, under `DefIds`; `None` when the low bits are the local id.
    back: Option<Vec<(u32, u32)>>,
}

impl Numbering {
    /// The local id of the node whose id's low 32 bits are `low`.
    pub fn local(&self, low: u32) -> Option<u32> {
        match &self.back {
            None => Some(low),
            Some(b) => b.binary_search_by_key(&low, |&(g, _)| g).ok().map(|i| b[i].1),
        }
    }
    /// The node whose id's low 32 bits are `low`: its local id, core id and kind.
    pub fn at(&self, low: u32) -> Option<(u32, u32, &'static str)> {
        let l = self.local(low)?;
        self.nodes.get(l as usize).map(|&(id, k)| (l, id, k))
    }
}

pub fn numbering(doc: &Document, shape: Shape) -> StoreResult<Numbering> {
    // A document read as text only has no nodes.
    if doc.facts.len() == 0 {
        return Ok(Numbering { nodes: Vec::new(), back: None });
    }
    let tag = crate::code_rule_dsl::facts::core::facts_tag(&doc.facts);
    let dialect = Dialect::kept(tag)?;
    // In `core.rr`'s shape a guest's nodes are its root's subtree; in T's, its whole tree (`Batch::build`).
    let guest = doc.guest.as_deref().filter(|_| shape == Shape::Core).map(|g| (g, Arc::new(core_rr::Lines::new("", false))));
    let p = Projected::new(doc.key, Arc::clone(&doc.facts), Some(&*dialect), shape, false, guest);
    let nodes = p.core.iter().map(|&id| (id, dialect.name(&doc.facts, id))).collect();
    // The ids the projection gives (`Projected::id`): per definition where it numbered the document so. A unit's own
    // nodes are numbered before its tokens, so the nodes' ids need no tokens.
    Ok(Numbering { nodes, back: p.defs.map(|d| d.back) })
}

/// A path's module: `a/b.py` is `a.b`, `a/__init__.py` is `a`; a Rust file's is its path from
/// the crate root under `crate` (`src/lib.rs` and `src/main.rs` are `crate`, `src/a/mod.rs` and
/// `src/a.rs` are `crate.a`).
pub fn module(rel: &str) -> String {
    if let Some(rs) = rel.strip_suffix(".rs") {
        let rs = rs.rsplit_once("src/").map_or(rs, |(_, r)| r);
        let mut parts: Vec<&str> = rs.split('/').collect();
        if matches!(parts.last(), Some(&"mod")) || (parts.len() == 1 && matches!(parts[0], "lib" | "main")) {
            parts.pop();
        }
        return std::iter::once("crate").chain(parts).collect::<Vec<_>>().join(".");
    }
    // A stub (`.pyi`) is named as its module is.
    let m = rel.strip_suffix(".pyi").or_else(|| rel.strip_suffix(".py")).unwrap_or(rel).replace('/', ".");
    m.strip_suffix(".__init__").map(str::to_owned).unwrap_or(m)
}

/// The package relative imports resolve against (`__package__`): a package's own name, else its
/// module's parent.
/// The package of module `m`, the file at `path`: `m` itself for a package's `__init__`, else `m`'s parent.
pub(crate) fn package_of(path: &str, m: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.starts_with("__init__.") { m.to_owned() } else { m.rsplit_once('.').map(|(p, _)| p.to_owned()).unwrap_or_default() }
}

pub fn package(rel: &str) -> String {
    let m = module(rel);
    if rel.ends_with("__init__.py") || rel.ends_with("__init__.pyi") { m } else { m.rsplit_once('.').map(|(p, _)| p.to_owned()).unwrap_or_default() }
}

/// Which of T's rules a kind falls under, by name.
#[derive(Clone, Copy, PartialEq)]
enum Class {
    Other,
    Comment,
    Text,
    Integer,
    Operator,
    FunctionDefinition,
    /// `for` and `with` statements, which an `async` keyword marks too.
    AsyncStatement,
    FunctionItem,
    Attribute,
    String,
}
impl Class {
    fn of(name: &str) -> Self {
        match name {
            "comment" => Self::Comment,
            "identifier" | "dotted_name" | "import_prefix" | "type_identifier" | "field_identifier" | "crate" | "self" | "super" => {
                Self::Text
            }
            "integer" => Self::Integer,
            "binary_operator" | "augmented_assignment" | "comparison_operator" | "boolean_operator" | "unary_operator" => {
                Self::Operator
            }
            "function_definition" => Self::FunctionDefinition,
            "for_statement" | "with_statement" => Self::AsyncStatement,
            "function_item" => Self::FunctionItem,
            "attribute" => Self::Attribute,
            "string" => Self::String,
            _ => Self::Other,
        }
    }
}

/// A dialect's kinds as T's exporter reads them. T's kinds were names, and a grammar gives some names several codes
/// (aliases): the `kind` column holds each name's canonical code (`KindCatalog::kind_id`, what a program's constant
/// lowers to), and a lookup reads the postings of every code of that name.
pub struct Dialect {
    tag: u8,
    family: Family,
    pub catalog: KindCatalog,
    classes: Vec<Class>,
    operator: Option<u16>,
    /// The `operators` field: a comparison's operator tokens (`a < b <= c`).
    operators: Option<u16>,
    pub canonical: Vec<u16>,
    codes: HashMap<u16, Vec<u16>>,
    /// Whether W reads values of the dialect's nodes (its family's `ValueRules::SUPPORTED`): `Extra`'s `value` and
    /// `value_type` are W's for its documents, `core_rr`'s literal values for the others (an `.rr` file's).
    values: bool,
    /// The grammar's token automata: the tokens a named-only tape leaves out, in the gaps between a
    /// parent's stored children (`tree::Tree`'s `token`, `prev_token`; t31).
    grammar: Option<Arc<crate::code_rule_dsl::tape::tokens::TokenGrammar>>,
}
impl Dialect {
    /// The dialect of `tag`, made once per thread: making one walks the grammar's kinds (a millisecond in a debug
    /// build), and a chunk's guests number every host they are made in (`numbering`).
    fn kept(tag: u8) -> StoreResult<std::rc::Rc<Self>> {
        thread_local! {
            static KEPT: std::cell::RefCell<HashMap<u8, std::rc::Rc<Dialect>>> = std::cell::RefCell::new(HashMap::new());
        }
        if let Some(d) = KEPT.with(|k| k.borrow().get(&tag).cloned()) {
            return Ok(d);
        }
        let d = std::rc::Rc::new(Self::new(tag)?);
        KEPT.with(|k| k.borrow_mut().insert(tag, std::rc::Rc::clone(&d)));
        Ok(d)
    }

    pub fn new(tag: u8) -> StoreResult<Self> {
        
        let dialect = from_tape_tag(tag).ok_or_else(|| StoreError::Invalid(format!("no dialect has tape tag {tag}")))?;
        let catalog = dialect.kinds();
        let values = crate::code_rule_dsl::language::base(crate::code_rule_dsl::language::Family::of(dialect)).reads_values();
        let classes = (0..catalog.kind_count()).map(|k| catalog.kind_name(k as u16).map_or(Class::Other, Class::of)).collect();
        let canonical: Vec<u16> = (0..catalog.kind_count() as u16)
            .map(|k| match catalog.kind_name(k) {
                Some(name) if catalog.kind_is_named(k) => Some(catalog.kind_id(name)).filter(|&c| c != 0).unwrap_or(k),
                _ => k,
            })
            .collect();
        let mut codes: HashMap<u16, Vec<u16>> = HashMap::new();
        for (k, &c) in canonical.iter().enumerate() {
            codes.entry(c).or_default().push(k as u16);
        }
        let family = Family::of(from_tape_tag(tag).and_then(crate::code_rule_dsl::language::label));
        let grammar = crate::code_rule_dsl::tape::tokens::TokenGrammar::of_dialect(dialect);
        Ok(Self { tag, family, catalog, classes, operator: catalog.field_id("operator"), operators: catalog.field_id("operators"), canonical, codes, values, grammar })
    }
    /// Node `id`'s kind code (ERROR's is `u16::MAX`).
    fn code(&self, f: &Facts, id: u32) -> u16 {
        kind_code(f.kind[id as usize])
    }
    fn class(&self, f: &Facts, id: u32) -> Class {
        self.classes.get(usize::from(self.code(f, id))).copied().unwrap_or(Class::Other)
    }
    pub fn name(&self, f: &Facts, id: u32) -> &'static str {
        self.catalog.kind_name(self.code(f, id)).unwrap_or("")
    }
    /// Node `id`'s kind atom as the `kind` column holds it: its name's canonical code's.
    fn kind(&self, f: &Facts, id: u32) -> i64 {
        let code = self.code(f, id);
        i64::from(kind_atom(self.tag, self.canonical.get(usize::from(code)).copied().unwrap_or(code)))
    }
    /// A kind atom's name (a token's, `Facts::tokens`).
    pub fn atom_name(&self, atom: u32) -> &'static str {
        self.catalog.kind_name(crate::code_rule_dsl::facts::core::kind_code(atom)).unwrap_or("")
    }
    /// The child of `id` in field `field`.
    fn field_child(&self, f: &Facts, id: u32, field: &str) -> Option<u32> {
        let field = self.catalog.field_id(field)?;
        f.children(id).iter().copied().find(|&c| f.field[c as usize] == field)
    }
    /// A name's dotted text: an identifier's, or an attribute's object's and attribute's joined by a dot (`a.b.c`),
    /// whatever spaces the source has between them. `None` where the object has no name (a call, a subscript).
    fn dotted(&self, f: &Facts, id: u32) -> Option<String> {
        match self.class(f, id) {
            Class::Text => Some(f.text(id).to_owned()).filter(|t| !t.contains(['\t', '\n', '\r', '\\'])),
            Class::Attribute => {
                let object = self.dotted(f, self.field_child(f, id, "object")?)?;
                let attribute = self.field_child(f, id, "attribute")?;
                Some(format!("{object}.{}", f.text(attribute)))
            }
            _ => None,
        }
    }
    /// A string literal's prefix (lowercased, `""` for none) and, where it has no interpolation, its content
    /// between the quotes, as written (escapes kept).
    fn string<'f>(&self, f: &'f Facts, id: u32) -> Option<(String, Option<&'f str>)> {
        let children = f.children(id);
        let start = *children.iter().find(|&&c| self.name(f, c) == "string_start")?;
        let end = *children.iter().rev().find(|&&c| self.name(f, c) == "string_end")?;
        let opening = f.text(start);
        let prefix = opening.trim_end_matches(['"', '\'']).to_lowercase();
        let interpolated = children.iter().any(|&c| self.name(f, c) == "interpolation");
        let whole = f.text(id);
        let content = (!interpolated && whole.len() >= opening.len() + f.text(end).len()).then(|| &whole[opening.len()..whole.len() - f.text(end).len()]);
        Some((prefix, content))
    }
}

/// A document's nodes, numbered in preorder: its named nodes other than comments (T's), or every named node
/// (`core.rr`'s).
pub struct Projected {
    pub module: Option<crate::code_rule_dsl::module_path::ModulePath>,
    pub key: u32,
    pub tag: u8,
    pub facts: Arc<Facts>,
    /// The core id of each node.
    pub core: Vec<u32>,
    /// The node of each core id, `NONE` for one left out.
    pub local: Vec<u32>,
    pub idx: OnceLock<Vec<u32>>,
    pub size: OnceLock<Vec<u32>>,
    pub depth: OnceLock<Vec<u32>>,
    /// The nodes hanging from their parent under a field, ascending.
    pub fielded: OnceLock<Vec<u32>>,
    /// `core.rr`'s lines and spans; `None` in T's shape.
    pub extra: Option<Box<Locs>>,
    /// Node ids numbered per definition (`DefIds`), when the run asks for them; else a node's id is its preorder.
    defs: Option<DefIds>,
}

/// Node ids per definition (decisions 0083 §3, 0094 §2; `granules`): a node's id is `document << 32 | slot << 18 |
/// index`, its granule's slot and its index there. An edit renumbers the granules it touches and no other's.
/// Spans that are positions (a document's text, its lines, its tokens) are in the top slots, in order; `Extra`'s spans
/// in the slots below them (`granules::ExtraSpan`), so a node by definition is never read as one of those spans, nor
/// one of them as a node.
struct DefIds {
    /// The low 32 bits of each node's and token's id, by local id.
    low: Vec<u32>,
    /// `(low, local)`, ascending: the way back.
    back: Vec<(u32, u32)>,
    /// Each node's order key, by local id (`granules::order`): `node_order`'s low 32 bits.
    order: Vec<u32>,
}

use granules::INDEX_BITS;

/// Whether this process numbers nodes per definition: the default; `RERAIL_NODE_IDS=preorder` numbers them in
/// preorder, to compare.
pub fn definition_ids() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| !std::env::var("RERAIL_NODE_IDS").is_ok_and(|v| v == "preorder"));
    *ON
}

/// How an output shows a node id's low 32 bits: its preorder index, or by definition `slot.index` (`DefIds`). A
/// memoised document is never parsed in a run, so the way back to preorder is not kept: the form by definition is
/// the stable one, and a finding keyed by it survives edits to other definitions.
pub fn shown_local(low: i64) -> String {
    if definition_ids() {
        format!("{}.{}", low >> INDEX_BITS, low & ((1 << INDEX_BITS) - 1))
    } else {
        low.to_string()
    }
}

impl DefIds {
    /// The ids of `p`'s nodes, and of `tokens` (each token's node, in order), by definition.
    fn new(p: &Projected, dialect: &Dialect, tokens: &[u32]) -> Self {
        let (low, order) = granules::number(p, dialect, tokens);
        let mut back: Vec<(u32, u32)> = low.iter().enumerate().map(|(l, &g)| (g, l as u32)).collect();
        back.sort_unstable();
        DefIds { low, back, order }
    }
}

/// A document's lines, and its spans past its nodes (`Shape::Core`'s): from local id `len()`, its anonymous tokens
/// (`tokens` of them, in preorder), then its whole text, then each line. Not `Extra`'s spans (`SPAN`), which the
/// relations core.rr's programs read beyond T's are made over.
pub struct Locs {
    lines: Arc<core_rr::Lines>,
    pub token_spans: OnceLock<Vec<[u32; 2]>>,
    pub tokens: u32,
    /// The whole source and its lines follow the tokens, implicit (`Projected::span`): a guest has none of its own.
    own_text: bool,
    /// A guest's host lines and placement: its places are the host's (decision 0055). A guest has no source-text
    /// spans of its own (`source`, `src_line`): those are its file's.
    host: Option<(Arc<core_rr::Lines>, Placement)>,
}

impl Locs {
    /// The document's spans past its nodes: its tokens, then (its own text) the whole source and each line.
    fn positions(&self) -> u32 { self.tokens + if self.own_text { 1 + self.lines.count() as u32 } else { 0 } }
    /// A span's first line, last line and start column, in its file.
    fn place(&self, span: [u32; 2]) -> (u32, u32, u32) {
        match &self.host {
            Some((lines, place)) => lines.place(place.host(span)),
            None => self.lines.place(span),
        }
    }
    /// The file line of this document's line `l`.
    fn file_line(&self, l: u32) -> u32 {
        match &self.host {
            Some((lines, place)) => {
                let at = if self.lines.count() == 0 { 0 } else { self.lines.span((l as usize).clamp(1, self.lines.count()))[0] };
                lines.line(place.host([at, at])[0])
            }
            None => l,
        }
    }
}

impl Projected {
    /// Document `key`'s projection in `shape`; `dialect` is its facts' (`None` for a text-only document, which has no
    /// node). A `withheld` document (`core.rr`: one that does not parse) has no nodes and no spans: no facts. A guest's
    /// nodes are its root's subtree, placed in its host's lines (`host`).
    pub fn new(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape, withheld: bool, guest: Option<(&Guest, Arc<core_rr::Lines>)>) -> Self {
        let identity_span = crate::code_rule_dsl::logic::profile::span(|| "identity maps".into());
        let f = &*facts;
        let mut p = Projected {
            module:None,
            key,
            tag: f.tag,
            core: Vec::new(),
            local: vec![NONE; f.len()],
            idx: OnceLock::new(),
            size: OnceLock::new(),
            depth: OnceLock::new(),
            fielded: OnceLock::new(),
            extra: None,
            defs: None,
            facts: Arc::clone(&facts),
        };
        let mut tokens = 0;
        let (first, last) = match guest {
            Some((g, _)) => (g.root, f.subtree_end.get(g.root as usize).copied().unwrap_or(g.root + 1).max(g.root + 1)),
            None => (0, f.len() as u32),
        };
        if !withheld {
            let count = (first..last.min(f.len() as u32)).filter(|&id| f.named(id) && dialect.is_some_and(|d| shape != Shape::T || d.class(f, id) != Class::Comment)).count();
            p.core.reserve_exact(count);
        }
        for id in first..if withheld { first } else { last.min(f.len() as u32) } {
            if !f.named(id) {
                // A token (`Facts::tokens`) of an included node: its span is made on first read (`Projected::span`).
                if shape == Shape::Core && dialect.is_some() && f.end[id as usize] > f.start[id as usize]
                    && f.parent(id).is_some_and(|q| p.local[q as usize] != NONE) { tokens += 1; }
                continue;
            }
            if dialect.is_none_or(|dialect| shape == Shape::T && dialect.class(f, id) == Class::Comment) {
                continue;
            }
            // T walks named children from the root: a node under one left out is not reached.
            let parent = match f.parent(id) {
                _ if id == first => NONE,
                None => NONE,
                Some(q) if p.local[q as usize] != NONE => p.local[q as usize],
                Some(_) => continue,
            };
            if parent == NONE && !p.core.is_empty() {
                continue;
            }
            let local = p.core.len() as u32;
            p.local[id as usize] = local;
            p.core.push(id);
        }
        drop(identity_span);
        if shape == Shape::Core {
            let lines = Arc::new(core_rr::Lines::shared(Arc::clone(&f.source), dialect.is_some_and(|d| d.family == Family::Python)));
            if definition_ids() && let Some(dialect) = dialect {
                let mut owners = Vec::with_capacity(tokens as usize);
                owners.extend(p.core.iter().enumerate().flat_map(|(l, &id)| f.children(id).iter().filter(|&&c| !f.named(c) && f.end[c as usize] > f.start[c as usize]).map(move |_| l as u32)));
                p.defs = Some(DefIds::new(&p, dialect, &owners));
            }
            // Raw file text remains valid when its tree is withheld. Guests have no own text spans.
            let own_text = guest.is_none();
            let host = guest.filter(|(g, _)| g.made == Made::Host).map(|(g, lines)| (lines, g.in_file.clone()));
            p.extra = Some(Box::new(Locs { lines, tokens, token_spans: OnceLock::new(), own_text, host }));
        } else if definition_ids() && let Some(dialect) = dialect {
            // T's shape has no token spans: its nodes alone are numbered per definition.
            p.defs = Some(DefIds::new(&p, dialect, &[]));
        }
        p
    }
    #[inline]
    pub fn parent(&self, local: usize) -> u32 {
        self.facts.parent(self.core[local]).map_or(NONE, |p| self.local[p as usize])
    }
    #[inline]
    pub fn idx(&self, local: usize) -> u32 { self.ordinals()[local] }
    #[inline]
    fn ordinals(&self) -> &[u32] {
        self.idx.get_or_init(|| {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "projection ordinals".into());
            let mut next = vec![0; self.core.len()];
            let mut out = vec![0; self.core.len()];
            for (l, slot) in out.iter_mut().enumerate().skip(1) {
                let p = self.parent(l) as usize;
                *slot = next[p]; next[p] += 1;
            }
            out
        })
    }
    #[inline]
    pub fn size(&self, local: usize) -> u32 { self.sizes()[local] }
    #[inline]
    pub fn sizes(&self) -> &[u32] {
        self.size.get_or_init(|| {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "projection subtree sizes".into());
            let mut out = vec![0; self.core.len()];
            for l in (1..out.len()).rev() { let p = self.parent(l) as usize; out[p] += out[l] + 1; }
            out
        })
    }
    #[inline]
    pub fn depth(&self, local: usize) -> u32 { self.depths()[local] }
    #[inline]
    pub fn depths(&self) -> &[u32] {
        self.depth.get_or_init(|| {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "projection depths".into());
            let mut out = vec![0; self.core.len()];
            for l in 1..out.len() { out[l] = out[self.parent(l) as usize] + 1; }
            out
        })
    }
    fn fielded(&self) -> &[u32] {
        self.fielded.get_or_init(|| {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "projection fields".into());
            let count = self.core.iter().skip(1).filter(|&&id| self.facts.field[id as usize] != 0).count();
            let mut out = Vec::with_capacity(count);
            out.extend(self.core.iter().enumerate().skip(1).filter_map(|(l, &id)| (self.facts.field[id as usize] != 0).then_some(l as u32)));
            out
        })
    }
    /// Nodes and spans: the ids `text`, `line` and `docof` take.
    pub fn locs(&self) -> u32 {
        self.len() + self.extra.as_ref().map_or(0, |x| x.positions())
    }
    /// The byte span of node or span `local`.
    pub fn span(&self, local: u32) -> [u32; 2] {
        match local.checked_sub(self.len()) {
            None => {
                let id = self.core[local as usize] as usize;
                [self.facts.start[id], self.facts.end[id]]
            }
            Some(i) => self.extra.as_ref().map_or([0, 0], |x| {
                if i < x.tokens {
                    let spans = x.token_spans.get_or_init(|| {
                        let _span = crate::code_rule_dsl::logic::profile::span(|| "token spans".into());
                        let f = &self.facts;
                        let mut spans = Vec::with_capacity(x.tokens as usize);
                        for &id in &self.core {
                            for &c in f.children(id) {
                                if !f.named(c) && f.end[c as usize] > f.start[c as usize] {
                                    spans.push([f.start[c as usize], f.end[c as usize]]);
                                }
                            }
                        }
                        debug_assert_eq!(spans.len(), x.tokens as usize);
                        spans
                    });
                    spans[i as usize]
                } else if i == x.tokens {
                    [0, self.facts.source.len() as u32]
                } else {
                    x.lines.span((i - x.tokens) as usize)
                }
            }),
        }
    }
    pub fn len(&self) -> u32 {
        self.core.len() as u32
    }
    pub fn global(&self, local: u32) -> i64 {
        Self::id(self.key, &self.defs, local)
    }
    /// `global`, from the two fields it reads (so a caller may hold the others mutably).
    fn id(key: u32, defs: &Option<DefIds>, local: u32) -> i64 {
        let low = match defs {
            Some(d) => d.low.get(local as usize).copied().unwrap_or_else(|| granules::span_low(local - d.low.len() as u32)),
            None => local,
        };
        i64::from(key) << 32 | i64::from(low)
    }
    /// Node `local`'s order key: `node_order`'s low 32 bits (`granules::order`; its preorder index without `DefIds`).
    pub fn order(&self, local: u32) -> u32 {
        self.defs.as_ref().map_or(local, |d| d.order[local as usize])
    }
    /// The node whose order key (`node_order`'s low 32 bits) is `key`: the keys ascend with the local ids.
    pub fn local_of_order(&self, key: u32) -> Option<u32> {
        match &self.defs {
            Some(d) => d.order.binary_search(&key).ok().map(|l| l as u32),
            None => (key < self.len()).then_some(key),
        }
    }
    /// The node or span whose id's low 32 bits are `low` (`global`'s inverse).
    pub fn local_of(&self, low: u32) -> Option<u32> {
        match &self.defs {
            Some(d) if granules::span_of(low).is_some() => granules::span_of(low).map(|i| d.low.len() as u32 + i),
            Some(d) => d.back.binary_search_by_key(&low, |&(g, _)| g).ok().map(|i| d.back[i].1),
            None => Some(low),
        }
    }
    /// The included children of `local`, in order.
    pub fn children(&self, local: u32) -> impl Iterator<Item = u32> + '_ {
        self.facts.children(self.core[local as usize]).iter().map(|&c| self.local[c as usize]).filter(|&c| c != NONE)
    }
}

/// One wave's documents, sorted by key, and the row offsets of the views.
pub struct Batch {
    pub docs: Vec<Projected>,
    pub dialects: HashMap<u8, Dialect>,
    pub nodes: Vec<usize>,
    children: Vec<usize>,
    fields: Vec<usize>,
    /// Nodes and spans (`Projected::locs`).
    locs: OnceLock<Vec<usize>>,
    shape: Shape,
    /// The registered lazy families (`Families`, Python's `py.*`: list step 25), by registration; `None` for one
    /// whose dialect has no document in the batch.
    families: Vec<Option<Box<dyn BoundFamilies>>>,
    /// The relations made as rows for what the program reads (`Extra` and those beside it), bound in place of a view
    /// of the same name (`line`, `end_line`, `col`, `docof`: their spans have places too).
    made: BTreeSet<&'static str>,
}

/// A span's id (core.rr's `span`: a piece of a document's text): `d << 32 | granules::extra_low(kind, i)`, in slots of
/// its kind's past every node of its document, so no node view has a row of it (`Batch::node`). `ExtraSpan::Line` `0`
/// is the whole source (`source`), `Line` `l` physical line `l` (`src_line`), `Lf` `l` the `\n`-line `l`
/// (`src_line_lf`), `Tape` `l` the tape's line `l` (`tape_line`), `Comment` `k` the document's `k`-th comment from 1,
/// in source order, of those no node is (`comment`: T's nodes leave out a grammar's `comment` kind, as Python's, but
/// keep Rust's `line_comment`, which is then the comment's id) and `Match` `i` the document's `i`th match or group of
/// a `match_all`, `match_group` or `rx_span` regex. A document with more of a kind than its capacity fails, named.
fn extra_span(key: i64, kind: granules::ExtraSpan, i: u32) -> StoreResult<i64> {
    granules::extra_low(kind, i).map(|low| key << 32 | i64::from(low)).ok_or_else(|| {
        StoreError::Invalid(format!("document {key} has more than {} {kind:?} spans (core.rr's span ids)", kind.capacity()))
    })
}

/// The physical lines of `source`, as byte ranges without their terminators: as Python's `str.splitlines()`
/// splits (`splitlines`: `\r\n`, `\n`, `\r`, `\v`, `\f`, `\x1c`-`\x1e`, `\x85`, U+2028, U+2029; W's `lines`
/// regexes), else at `\n` only. A terminator at the end ends the last line: no empty line follows it, and an empty
/// source has none.
pub fn physical_lines(source: &str, splitlines: bool) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = source.char_indices().peekable();
    while let Some((at, ch)) = chars.next() {
        let ends = ch == '\n'
            || splitlines && matches!(ch, '\r' | '\u{b}' | '\u{c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if !ends {
            continue;
        }
        out.push(start..at);
        start = at + ch.len_utf8();
        if ch == '\r' && chars.peek().is_some_and(|&(_, c)| c == '\n') {
            chars.next();
            start += 1;
        }
    }
    if start < source.len() {
        out.push(start..source.len());
    }
    out
}

/// A family's relations over a batch's documents of its dialect, made on their first read (Python's `py.*`),
/// registered at load by the crate that knows the family.
pub trait Families: Send + Sync {
    /// The dialect whose documents they are over.
    fn dialect(&self) -> crate::code_rule_dsl::language::GrammarDialect;
    /// The relations' names, and each one's columns.
    fn names(&self) -> &'static [&'static str];
    fn columns(&self) -> &'static [&'static [&'static str]];
    /// The strings its rows hold as atoms, interned with the batch's.
    fn strings(&self) -> &'static [&'static str];
    /// The families of a batch of `documents` documents, some of the dialect (`tag`, its kinds `catalog`), `atoms`
    /// its strings'.
    fn bind(&self, tag: u8, catalog: &KindCatalog, atoms: &[i64], documents: usize) -> Box<dyn BoundFamilies>;
    /// The infix of a document's derivation name for the families (`{scope}#py:{path}`, decision 0029).
    fn derived(&self) -> &'static str;
    /// The producer of their derivations: the rows' version, over core facts.
    fn producer(&self) -> [u8; 32];
    /// The digest of their rows over a document's core facts.
    fn rows_digest(&self, facts: &Facts) -> [u8; 32];
}

/// A batch's [`Families`].
pub trait BoundFamilies: Send + Sync {
    /// The rows of the `family`th relation over `batch`'s documents.
    fn rows<'b>(&'b self, batch: &'b Batch, family: usize) -> &'b Relation;
    /// The families as their own type, for the family's other readers (Python's candidates and demand).
    fn as_any(&self) -> &dyn std::any::Any;
}

impl Batch {
    /// The batch's bound families of type `T`; `None` without documents of their dialect.
    pub fn bound<T: 'static>(&self) -> Option<&T> {
        self.families.iter().flatten().find_map(|bound| bound.as_any().downcast_ref::<T>())
    }
}

/// A family's semantic facts over a batch's explicit demands (Python's qualified names, origins, assignments and
/// arguments, and its call candidates), which a program's native steps read; registered at load.
pub trait SemanticProducer: Send + Sync {
    /// The `kind` facts of the nodes `requests`, for `targets`.
    fn project(&self, batch: &Batch, kind: &str, requests: &[i64], targets: &BTreeSet<String>) -> Result<Vec<Vec<rusqlite::types::Value>>, String>;
    /// The call candidates in the documents `scope` for `targets`, in calls of `context` reading `member`.
    fn candidates(&self, batch: &Batch, scope: &[i64], targets: &BTreeSet<String>, context: &str, member: &str) -> Result<Vec<Vec<rusqlite::types::Value>>, String>;
}

static SEMANTIC: std::sync::OnceLock<&'static dyn SemanticProducer> = std::sync::OnceLock::new();

/// Registers the producer of semantic facts (once, at load).
pub fn install_semantic_producer(producer: &'static dyn SemanticProducer) {
    let _ = SEMANTIC.set(producer);
}

/// The registered producer of semantic facts.
pub fn semantic_producer() -> Result<&'static dyn SemanticProducer, String> {
    SEMANTIC.get().copied().ok_or_else(|| "no producer of semantic facts is registered".to_owned())
}

/// Whether `rel` is a registered family's lazy relation.
pub fn is_family(rel: &str) -> bool {
    family_columns(rel).is_some()
}

static FAMILIES: std::sync::OnceLock<Vec<&'static dyn Families>> = std::sync::OnceLock::new();

/// Registers the families' lazy relations (once, at load).
pub fn install_families(families: Vec<&'static dyn Families>) {
    let _ = FAMILIES.set(families);
}

pub(crate) fn families() -> &'static [&'static dyn Families] {
    FAMILIES.get().map_or(&[], Vec::as_slice)
}

/// The columns of a registered family's relation `rel`.
pub fn family_columns(rel: &str) -> Option<&'static [&'static str]> {
    families().iter().find_map(|f| f.names().iter().position(|&name| name == rel).map(|i| f.columns()[i]))
}

/// One of a batch's lazy families' relations, served from the rows its first read makes (none without documents of
/// its dialect).
struct FamilySource {
    batch: Arc<Batch>,
    at: usize,
    family: usize,
    empty: Relation,
}

impl FamilySource {
    fn rows(&self) -> &Relation {
        match &self.batch.families[self.at] {
            Some(bound) => bound.rows(&self.batch, self.family),
            None => &self.empty,
        }
    }
}

impl Source for FamilySource {
    fn columns(&self) -> Vec<String> {
        families()[self.at].columns()[self.family].iter().map(|c| (*c).to_owned()).collect()
    }
    fn len(&self) -> usize {
        self.rows().len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.rows().estimated_matches(column, batch)
    }
    fn indexed(&self, column: usize) -> bool {
        self.rows().indexed(column)
    }
    fn range_indexed(&self, column: usize) -> bool {
        self.rows().range_indexed(column)
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        self.rows().range_lookup(column, lower, upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.rows().lookup(column, values)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.rows().value(row, column)
    }
}

/// Which view a source serves.
#[derive(Clone, Copy)]
pub enum View {
    Node,
    /// `line(n, l)`: a node's first line, 1-based (rows laid out as `node`'s).
    Line,
    Child,
    Field,
    Tape,
    /// `core_node(d, t, n)`: document `d`'s tape node `t` (the matcher's preorder id over every
    /// node) is node `n`. The wave's matches are tape nodes; this joins them to the rows above.
    Core,

    /// `ast_depth(n, k)`: the node's depth, the root's 0.
    Depth,
    /// `node_order(n, k)`: the node's place in preorder, `document << 32 | its order key`: by definition, its granule
    /// run's rank and its index in the granule (`granules::order`, which an edit elsewhere leaves alone); under
    /// `RERAIL_NODE_IDS=preorder`, its preorder index. Rules that order nodes read it (`front::order_nodes`).
    Order,
    /// `core.rr`'s place of a node or span (`loc`): `line(n, l)`, `end_line(n, l)`, `col(n, c)`, `docof(n, d)`,
    /// read from the document's lines (`core_rr::Lines`), rows laid out over `Projected::locs`. A batch that makes
    /// one of them as rows (`Extra`, whose spans have places too) binds those rows instead (`Batch::made`).
    Loc(Place),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Line,
    EndLine,
    Col,
    Docof,
}

/// The relations of core.rr a batch makes only for a program that reads them (its `reads`), T's exporter having
/// had none of them, so a program that reads none does no work and interns no text:
///
/// ```text
/// source(d, sp)          src_line(d, l, sp)     src_line_lf(d, l, sp)  tape_line(d, l, sp)
/// line_count(d, k)       node_text(n, s)        end_line(n, l)         col(n, c)
/// docof(n, d)
/// ```
///
/// with `sp` a span (`SPAN`); `src_line` split as `physical_lines` does (Python's `str.splitlines()`; other
/// languages at `\n`), `src_line_lf` at `\n` only (W's `terminators: "lf"`), and `tape_line` the lines `line(n, l)`
/// numbers (the tape's `LineTable`: `\r\n`, `\r` and `\n` for Python; W's `layout`), the empty line after a final
/// terminator included; `line_count` counts `src_line`'s lines. Spans have rows in `text` (their text, terminators
/// left out), `line`, `end_line` and `col` (0): a line span's line is its number; the source's first line is 1 and
/// its end line the line holding its end offset (`src_line`'s numbering). A node's `end_line` is the line holding its
/// end offset, as W's `layout` counts (its last byte's, but where the node ends with a line terminator, as a module
/// may), and its `col` its start's byte column, both on the tape's lines; `node_text` is its source text, exactly.
/// `value(n, v)` is W's value of expression node `n` as a string (`matcher::file_values`: a clause's `value` in its
/// per-file context; Python's `str(v)` of a str, int, float, bool, None or Ellipsis), read off the document's tape
/// (`Document::tape`), no row where W does not know it or it is no scalar; `static_value(n, v)` the same with names
/// followed through their file-local bindings (W's `value.follow`). `value_type_of(n, t)` names the type of W's value
/// of `n` as W's `value.type` does (containers and bytes included, a JavaScript RegExp also `object`), and
/// `value_type(v, t)` pairs `value`'s strings with those types: one row per string and type some node of the batch has
/// (the str `"12"` and the int `12` both give `12`). `comment(c, s)` is each comment's text without its marker
/// (`comment_body`); a comment no node is (`COMMENT`) has rows in `text` and `node_text` (its text, marker included),
/// `line`, `end_line` and `col` as a node's. `docof` has every node and every span (comments and matches included),
/// each with its document (a span's `sp >> 32`, as a node's). Rows hold string indexes, as `Batch::build`'s do.
///
/// In `core.rr`'s shape (`Shape::Core`) a relation made here is the batch's, in place of `core_rr`'s rows or the `Loc`
/// view of that name (`Batch::build`), and its node rows cover that shape's anonymous tokens too (`Locs`); `comment`
/// is `core_rr`'s there (comments are nodes), so it is not made here.
///
///
/// And each `matchall_<k>(sp, m)` read (`logic::strings::as_relations`, core.rr's `match_all`): `m` each match of
/// regex `k` in span `sp`'s text, leftmost-first and non-overlapping (Rust's `find_iter`, as W's `regex` reads a
/// file), over every span above. A match is a span (`MATCH`) with rows in `text` (what it matched), `line`,
/// `end_line` and `col`, as W's finding of that regex is placed: in a line span, that line and the byte column in it
/// (W's `lines` regexes); in the source, the lines and column of the tape's line table (W's content regex).
///
/// And in `core.rr`'s shape, `rx_span(p, c, j, g, m)` where read (`rr::lower`'s `match_all` and `match` over a span):
/// group `g` of match `j` (from 0) of regex `p` in span `c` is span `m` (`MATCH`, numbered with `match_all`'s), placed
/// as `match_all`'s. A span `c` is a document's `source` span, one of its `src_line` spans, a comment node, a node of a
/// kind, or a span an earlier pattern's group captured (`SpanIn`, decision 0081). Group "" is the whole match; a group
/// that takes no part in a match has no row. The spans are those `source` and `src_line` give here, so the two join.
#[derive(Default)]
struct Extra {
    source: bool,
    src_line: bool,
    src_line_lf: bool,
    tape_line: bool,
    line_count: bool,
    text: bool,
    /// `text` of the `source` and `src_line` spans: core.rr's programs make it where a rule reads a copy of such a
    /// span's text (`Projection::wants_span_text`), T's wherever `text` is read.
    source_text: bool,
    line_text: bool,
    line: bool,
    end_line: bool,
    col: bool,
    node_text: bool,
    docof: bool,
    value: bool,
    static_value: bool,
    value_type_of: bool,
    value_type: bool,
    comment: bool,
    sources: Vec<[i64; 2]>,
    src_lines: Vec<[i64; 3]>,
    src_lines_lf: Vec<[i64; 3]>,
    tape_lines: Vec<[i64; 3]>,
    line_counts: Vec<[i64; 2]>,
    lines: Vec<[i64; 2]>,
    end_lines: Vec<[i64; 2]>,
    cols: Vec<[i64; 2]>,
    node_texts: Vec<[i64; 2]>,
    docofs: Vec<[i64; 2]>,
    values: Vec<[i64; 2]>,
    static_values: Vec<[i64; 2]>,
    value_types_of: Vec<[i64; 2]>,
    value_types: BTreeSet<[i64; 2]>,
    comments: Vec<[i64; 2]>,
    /// The regexes of the `match_all` and `match_group` relations read (`Found`).
    matches: Vec<Found>,
    rx_span: bool,
    rx_spans: Vec<[i64; 5]>,
    /// A pattern's matches in a line's, node's or capture's text, by pattern and text, for the batch (decision 0081):
    /// each distinct text is scanned once. Each match's groups as (match, group, start, end), offsets in the text.
    rx_memo: HashMap<(usize, Box<str>), GroupSpans>,
}

/// A text's matches of one pattern: each match's groups as (match, group, start, end), offsets in the text.
type GroupSpans = Arc<[(u32, u32, u32, u32)]>;

/// Regex `k`'s rows: its matches (`matchall_<k>(sp, m)`, when read) and their named groups' spans
/// (`matchgroup_<k>(sp, m, g, gsp)`, when read; `g` a string index), with one numbering of `m` for both.
struct Found {
    k: usize,
    regex: Arc<regex::Regex>,
    all: bool,
    groups: bool,
    rows: Vec<[i64; 2]>,
    group_rows: Vec<[i64; 4]>,
}

/// A span made for a document: its id, its bytes, and its line where it is one line (else the source).
type MadeSpan = (i64, std::ops::Range<usize>, Option<usize>);

impl Extra {
    fn new(reads: &BTreeSet<String>, matches: &[Arc<regex::Regex>], shape: Shape, [source_text, line_text]: [bool; 2]) -> Self {
        let read = |rel: &str| reads.contains(rel);
        Self {
            source: read("source"),
            src_line: read("src_line"),
            src_line_lf: read("src_line_lf"),
            tape_line: read("tape_line"),
            line_count: read("line_count"),
            text: read("text"),
            source_text: read("text") && source_text,
            line_text: read("text") && line_text,
            line: read("line"),
            end_line: read("end_line"),
            col: read("col"),
            node_text: read("node_text"),
            docof: read("docof"),
            value: read("value"),
            static_value: read("static_value"),
            value_type_of: read("value_type_of"),
            value_type: read("value_type"),
            comment: shape == Shape::T && read("comment"),
            rx_span: shape == Shape::Core && read("rx_span"),
            matches: matches
                .iter()
                .enumerate()
                .map(|(k, regex)| Found {
                    k,
                    regex: Arc::clone(regex),
                    all: read(&format!("{}{k}", crate::code_rule_dsl::logic::strings::MATCHES)),
                    groups: read(&format!("{}{k}", crate::code_rule_dsl::logic::strings::GROUPS)),
                    rows: Vec::new(),
                    group_rows: Vec::new(),
                })
                .filter(|found| found.all || found.groups)
                .collect(),
            ..Self::default()
        }
    }

    /// Whether spans are made (their matches' too), with places in `line`, `end_line` and `col`.
    fn spans(&self) -> bool {
        self.source || self.src_line || self.src_line_lf || self.tape_line || self.comment || self.rx_span
    }

    /// Whether `line` is made as rows rather than read off the node view: spans have lines too.
    fn line_rows(&self) -> bool {
        self.line && self.spans()
    }

    /// A span's rows in `text` (into `text`, with `text_too`), `line`, `end_line` and `col` (its start's byte column).
    #[allow(clippy::too_many_arguments)]
    fn span(&mut self, sp: i64, first: usize, last: usize, col: usize, body: &str, text_too: bool, text: &mut Vec<[i64; 2]>, string: &mut dyn FnMut(&str) -> usize) {
        if self.docof {
            self.docofs.push([sp, sp >> 32]);
        }
        if self.text && text_too {
            text.push([sp, string(body) as i64]);
        }
        if self.line_rows() {
            self.lines.push([sp, first as i64]);
        }
        if self.end_line {
            self.end_lines.push([sp, last as i64]);
        }
        if self.col {
            self.cols.push([sp, col as i64]);
        }
    }

    /// Document `p`'s node rows: `line`, `end_line`, `col`, `node_text` and `docof`; and `line`, `end_line`, `col` and
    /// `docof` of `core.rr`'s anonymous tokens (`Locs`), which follow the nodes. A guest's nodes are placed in its file
    /// (`Locs::place`, decision 0055).
    fn nodes(&mut self, p: &Projected, string: &mut dyn FnMut(&str) -> usize) {
        let f = &*p.facts;
        let tokens = p.extra.as_ref().map_or(0, |x| x.tokens);
        let guest = p.extra.as_deref().filter(|x| x.host.is_some());
        if self.line_rows() || self.end_line || self.col || self.node_text {
            for (local, &id) in p.core.iter().enumerate() {
                let n = p.global(local as u32);
                let (start, end) = (f.start[id as usize] as usize, f.end[id as usize] as usize);
                if self.node_text {
                    self.node_texts.push([n, string(f.text(id)) as i64]);
                }
                if let Some(x) = guest {
                    let (first, last, col) = x.place([start as u32, end as u32]);
                    if self.line_rows() {
                        self.lines.push([n, i64::from(first)]);
                    }
                    if self.end_line {
                        self.end_lines.push([n, i64::from(last)]);
                    }
                    if self.col {
                        self.cols.push([n, i64::from(col)]);
                    }
                    continue;
                }
                if self.line_rows() {
                    self.lines.push([n, i64::from(f.start_line(id))]);
                }
                if self.end_line {
                    self.end_lines.push([n, f.lines.line(end) as i64 + 1]);
                }
                if self.col {
                    self.cols.push([n, (start - f.lines.start(f.lines.line(start))) as i64]);
                }
            }
            for local in p.len()..p.len() + tokens {
                let n = p.global(local);
                let [start, end] = p.span(local).map(|at| at as usize);
                if self.line_rows() {
                    self.lines.push([n, f.lines.line(start) as i64 + 1]);
                }
                if self.end_line {
                    self.end_lines.push([n, f.lines.line(end) as i64 + 1]);
                }
                if self.col {
                    self.cols.push([n, (start - f.lines.start(f.lines.line(start))) as i64]);
                }
            }
        }
        if self.docof {
            let key = i64::from(p.key);
            self.docofs.extend((0..p.len() + tokens).map(|local| [p.global(local), key]));
        }
    }

    /// `core_rr`'s literal values (n, v, v's type), of documents W reads no values of (`Dialect::values`), in the
    /// value relations made here: `value` and `static_value` (no names to follow), and `value_type`.
    fn literal_values(&mut self, rows: impl Iterator<Item = [i64; 3]>) {
        for [n, v, t] in rows {
            if self.value {
                self.values.push([n, v]);
            }
            if self.static_value {
                self.static_values.push([n, v]);
            }
            if self.value_type {
                self.value_types.insert([v, t]);
            }
        }
    }

    /// Document `p`'s rows (its `Document` `d`, its grammar's kinds `dialect`, none for a text-only document);
    /// `splitlines`: its lines split as Python's `str.splitlines()` does; `rx`: `rx_span`'s patterns, the spans each
    /// runs over and its block program, and `ends` (by pattern) the bytes of the document's source a match can end
    /// after, where the batch ran the pattern's block program (decision 0081 §3).
    #[allow(clippy::too_many_arguments)]
    fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, splitlines: bool, rx: &[core_rr::SpanPattern<'_>], ends: &[Option<Vec<u64>>], text: &mut Vec<[i64; 2]>, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<()> {
        let f = &*p.facts;
        let plain = self.value || self.value_type_of || self.value_type;
        // A text-only document has no node, so no value; a document of a family W reads no values of has none of W's
        // (`literal_values`).
        if (plain || self.static_value) && dialect.is_some_and(|d| d.values) {
            let Some((tape, file)) = &d.tape else {
                return Err(StoreError::Invalid(format!("{}: W's values need the document's tape", d.path)));
            };
            if tape.transducer_len() != f.len() {
                return Err(StoreError::Invalid(format!("{}: the tape is not the one the facts were read off", d.path)));
            }
            let (value, value_type_of, value_type) = (self.value, self.value_type_of, self.value_type);
            let (values, static_values, types_of, types) = (&mut self.values, &mut self.static_values, &mut self.value_types_of, &mut self.value_types);
            let Some(reader) = crate::code_rule_dsl::facts::core::value_reader() else {
                return Err(StoreError::Invalid(format!("{}: no reader of W's values is registered", d.path)));
            };
            reader.values(tape, file.as_ref(), plain, self.static_value, &mut |followed, ordinal, v, names| {
                let local = p.local[ordinal as usize];
                if local == NONE {
                    return;
                }
                // A value's text is interned only for a relation that holds it.
                let v = v.filter(|_| followed || value || value_type).map(|v| string(&v) as i64);
                let n = p.global(local);
                if followed {
                    static_values.extend(v.map(|v| [n, v]));
                    return;
                }
                if value && let Some(v) = v {
                    values.push([n, v]);
                }
                for name in names {
                    let t = string(name) as i64;
                    if value_type_of {
                        types_of.push([n, t]);
                    }
                    if value_type && let Some(v) = v {
                        types.insert([v, t]);
                    }
                }
            });
        }
        self.nodes(p, string);
        let key = i64::from(p.key);
        use granules::ExtraSpan;
        let span = |kind: ExtraSpan, i: usize| extra_span(key, kind, u32::try_from(i).unwrap_or(u32::MAX));
        let source = f.source.as_str();
        if self.comment && let Some(dialect) = dialect {
            use crate::code_rule_dsl::language::is_comment_kind;
            let mut k = 0;
            for id in 0..f.len() as u32 {
                // A comment kind's node, not inside another (Rust's doc comment parts are not comments).
                if !f.named(id) || !is_comment_kind(dialect.name(f, id)) || f.parent(id).is_some_and(|q| is_comment_kind(dialect.name(f, q))) {
                    continue;
                }
                let whole = f.text(id);
                let c = match p.local[id as usize] {
                    NONE => {
                        k += 1;
                        let c = span(ExtraSpan::Comment, k)?;
                        let (start, end) = (f.start[id as usize] as usize, f.end[id as usize] as usize);
                        let col = start - f.lines.start(f.lines.line(start));
                        self.span(c, f.start_line(id) as usize, f.lines.line(end) + 1, col, whole, true, text, string);
                        if self.node_text {
                            self.node_texts.push([c, string(whole) as i64]);
                        }
                        c
                    }
                    local => p.global(local),
                };
                self.comments.push([c, string(comment_body(whole)) as i64]);
            }
        }
        if !(self.spans() || self.line_count) {
            return Ok(());
        }
        let physical = physical_lines(source, splitlines);
        if self.line_count {
            self.line_counts.push([key, physical.len() as i64]);
        }
        let mut made: Vec<MadeSpan> = Vec::new();
        // `rx_span`'s containers among them: the source's and the lines' (indexes in `made`).
        let (mut source_at, mut lines_at) = (None, 0..0);
        if self.source {
            let sp = span(ExtraSpan::Line, 0)?;
            self.sources.push([key, sp]);
            let terminated = physical.last().map_or(!source.is_empty(), |r| r.end < source.len());
            self.span(sp, 1, (physical.len() + usize::from(terminated)).max(1), 0, source, self.source_text, text, string);
            source_at = Some(made.len());
            made.push((sp, 0..source.len(), None));
        }
        if self.src_line {
            lines_at.start = made.len();
            for (i, r) in physical.iter().enumerate() {
                let (l, sp) = (i + 1, span(ExtraSpan::Line, i + 1)?);
                self.src_lines.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], self.line_text, text, string);
                made.push((sp, r.clone(), Some(l)));
            }
            lines_at.end = made.len();
        }
        if self.src_line_lf {
            for (i, r) in physical_lines(source, false).into_iter().enumerate() {
                let (l, sp) = (i + 1, span(ExtraSpan::Lf, i + 1)?);
                self.src_lines_lf.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], true, text, string);
                made.push((sp, r, Some(l)));
            }
        }
        if self.tape_line {
            for i in 0..f.lines.starts().len() {
                let (l, sp) = (i + 1, span(ExtraSpan::Tape, i + 1)?);
                let r = f.lines.start(i)..f.lines.end(i);
                self.tape_lines.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], true, text, string);
                made.push((sp, r, Some(l)));
            }
        }
        // Each regex's matches in each span, and their named groups, numbered through the document.
        let mut next: usize = 0;
        let mut id = |_: &Projected| -> StoreResult<i64> {
            next += 1;
            span(ExtraSpan::Match, next - 1)
        };
        // A piece of span `r` (`at` within it) placed as W places a regex's finding there: in a line (its number and
        // its start), that line and the byte column in it; else on the tape's lines.
        let place = |r: &std::ops::Range<usize>, line: Option<(usize, usize)>, at: std::ops::Range<usize>| match line {
            Some((l, start)) => (l, l, r.start + at.start - start),
            None => {
                let s = f.lines.span(r.start + at.start, r.start + at.end);
                (s.start_line + 1, s.end_line + 1, s.start_column)
            }
        };
        for k in 0..self.matches.len() {
            let (regex, groups) = (Arc::clone(&self.matches[k].regex), self.matches[k].groups);
            let names: Vec<&str> = regex.capture_names().flatten().collect();
            for (sp, r, line) in &made {
                let body = &source[r.clone()];
                for captures in regex.captures_iter(body) {
                    let found = captures.get(0).expect("group 0 always matches");
                    let m = id(p)?;
                    self.matches[k].rows.push([*sp, m]);
                    let (first, last, col) = place(r, line.map(|l| (l, r.start)), found.range());
                    self.span(m, first, last, col, found.as_str(), true, text, string);
                    if !groups {
                        continue;
                    }
                    // A group that takes no part in the match has no row.
                    for name in &names {
                        let Some(group) = captures.name(name) else { continue };
                        let g = id(p)?;
                        self.matches[k].group_rows.push([*sp, m, string(name) as i64, g]);
                        let (first, last, col) = place(r, line.map(|l| (l, r.start)), group.range());
                        self.span(g, first, last, col, group.as_str(), true, text, string);
                    }
                }
            }
        }
        // `rx_span`: each pattern over its spans, `j` each match in one (decision 0081). A container is a span made
        // here, a node (a comment, a node of a kind) or a span an earlier pattern's group captured: (its id, its bytes,
        // its line and the line's start where it lies in one line span).
        type Container = (i64, std::ops::Range<usize>, Option<(usize, usize)>);
        let rx = if self.rx_span { rx } else { &[] };
        let nodes_of = |test: &dyn Fn(&str) -> bool| -> Vec<Container> {
            match dialect {
                Some(dialect) => (0..p.len())
                    .filter(|&l| test(dialect.name(f, p.core[l as usize])))
                    .map(|l| {
                        let [a, b] = p.span(l);
                        (p.global(l), a as usize..b as usize, None)
                    })
                    .collect(),
                None => Vec::new(),
            }
        };
        // Spans each (pattern, group) captured in this document: the containers of a `SpanIn::Capture`. A capture's
        // container is made by an earlier pattern: patterns run once theirs has; patterns whose containers wait on each
        // other (a cycle of captures) run in order, with what is made by then.
        let mut captured: HashMap<(&str, &str), Vec<Container>> = HashMap::new();
        let mut done = vec![false; rx.len()];
        let mut force = false;
        // Bytes scanned, reported as profile work.
        let mut scanned = 0u64;
        while !done.iter().all(|d| *d) {
            let mut progressed = false;
            for (i, (within, pattern, re, prefilter)) in rx.iter().enumerate() {
                use crate::code_rule_dsl::logic::front::ast::SpanIn;
                if done[i] {
                    continue;
                }
                if !force
                    && let SpanIn::Capture(from, _) = within
                    && rx.iter().enumerate().any(|(k, (_, q, _, _))| !done[k] && k != i && *q == from.as_str())
                {
                    continue;
                }
                done[i] = true;
                progressed = true;
                let line_of = |(sp, r, line): &MadeSpan| -> Container { (*sp, r.clone(), line.map(|l| (l, r.start))) };
                // The source is scanned once per pattern (decision 0042 §3); a line's, node's or capture's text once
                // per distinct text in the batch (0042 §2, `rx_memo`), its matches placed at each span holding it.
                let (containers, memo): (Vec<Container>, bool) = match within {
                    SpanIn::Source => (source_at.map(|i| line_of(&made[i])).into_iter().collect(), false),
                    SpanIn::Line => (made[lines_at.clone()].iter().map(line_of).collect(), true),
                    SpanIn::Comment => (nodes_of(&core_rr::is_comment), true),
                    SpanIn::Kind(k) => (nodes_of(&|name| name == k.as_str()), true),
                    SpanIn::Capture(from, g) => (captured.get(&(from.as_str(), g.as_str())).cloned().unwrap_or_default(), true),
                };
                let pat = string(pattern) as i64;
                let names: Vec<Option<&str>> = re.capture_names().enumerate().map(|(g, n)| if g == 0 { Some("") } else { n }).collect();
                // The bytes a match can end after (decision 0081 §3), from the batch's run of the block programs: a
                // source or line where none can is not scanned.
                let ends = match (ends.get(i).and_then(Option::as_ref), prefilter) {
                    (Some(e), _) => Some(std::borrow::Cow::Borrowed(e.as_slice())),
                    (None, Some(f)) => Some(std::borrow::Cow::Owned(f.ends(source.as_bytes()))),
                    (None, None) => None,
                };
                for (c, r, line) in &containers {
                    if let Some(ends) = &ends
                        && !crate::code_rule_dsl::facts::span_prefilter::any_in(ends, r.start, r.end)
                    {
                        continue;
                    }
                    let body = &source[r.clone()];
                    let found: GroupSpans = if memo && body.len() <= core_rr::RX_MEMO_TEXT {
                        match self.rx_memo.get(&(pat as usize, Box::from(body))) {
                            Some(found) => Arc::clone(found),
                            None => {
                                scanned += body.len() as u64;
                                let found = core_rr::scan(re, &names, body);
                                self.rx_memo.insert((pat as usize, Box::from(body)), Arc::clone(&found));
                                found
                            }
                        }
                    } else {
                        scanned += body.len() as u64;
                        core_rr::scan(re, &names, body)
                    };
                    for &(j, g, s0, e0) in found.iter() {
                        let name = names[g as usize].expect("a named group");
                        let at = s0 as usize..e0 as usize;
                        let m = id(p)?;
                        self.rx_spans.push([pat, *c, i64::from(j), string(name) as i64, m]);
                        let (first, last, col) = place(r, *line, at.clone());
                        self.span(m, first, last, col, &body[at.clone()], true, text, string);
                        captured.entry((*pattern, name)).or_default().push((m, r.start + at.start..r.start + at.end, *line));
                    }
                }
            }
            force = !progressed;
        }
        if !rx.is_empty() {
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, scanned);
        }
        Ok(())
    }

    /// The relations read, their strings as atoms (`atom` of a string index), and the `match_all` relations read.
    #[allow(clippy::type_complexity)]
    fn relations(self, atom: &dyn Fn(i64) -> i64) -> (Vec<(&'static str, Relation)>, Vec<(String, Relation)>) {
        let relation = |columns: &[&str], rows: &mut dyn Iterator<Item = Vec<i64>>| {
            let mut r = Relation::new(columns);
            rows.for_each(|row| r.push(&row));
            r
        };
        let mut out = Vec::new();
        if self.source {
            out.push(("source", relation(&["d", "sp"], &mut self.sources.iter().map(|r| r.to_vec()))));
        }
        if self.src_line {
            out.push(("src_line", relation(&["d", "l", "sp"], &mut self.src_lines.iter().map(|r| r.to_vec()))));
        }
        if self.src_line_lf {
            out.push(("src_line_lf", relation(&["d", "l", "sp"], &mut self.src_lines_lf.iter().map(|r| r.to_vec()))));
        }
        if self.tape_line {
            out.push(("tape_line", relation(&["d", "l", "sp"], &mut self.tape_lines.iter().map(|r| r.to_vec()))));
        }
        if self.line_count {
            out.push(("line_count", relation(&["d", "k"], &mut self.line_counts.iter().map(|r| r.to_vec()))));
        }
        if self.line_rows() {
            out.push(("line", relation(&["n", "l"], &mut self.lines.iter().map(|r| r.to_vec()))));
        }
        if self.end_line {
            out.push(("end_line", relation(&["n", "l"], &mut self.end_lines.iter().map(|r| r.to_vec()))));
        }
        if self.col {
            out.push(("col", relation(&["n", "c"], &mut self.cols.iter().map(|r| r.to_vec()))));
        }
        if self.node_text {
            out.push(("node_text", relation(&["n", "s"], &mut self.node_texts.iter().map(|&[n, s]| vec![n, atom(s)]))));
        }
        if self.docof {
            out.push(("docof", relation(&["n", "d"], &mut self.docofs.iter().map(|r| r.to_vec()))));
        }
        if self.value {
            out.push(("value", relation(&["n", "v"], &mut self.values.iter().map(|&[n, v]| vec![n, atom(v)]))));
        }
        if self.static_value {
            out.push(("static_value", relation(&["n", "v"], &mut self.static_values.iter().map(|&[n, v]| vec![n, atom(v)]))));
        }
        if self.value_type_of {
            out.push(("value_type_of", relation(&["n", "t"], &mut self.value_types_of.iter().map(|&[n, t]| vec![n, atom(t)]))));
        }
        if self.value_type {
            out.push(("value_type", relation(&["v", "t"], &mut self.value_types.iter().map(|&[v, t]| vec![atom(v), atom(t)]))));
        }
        if self.comment {
            out.push(("comment", relation(&["n", "s"], &mut self.comments.iter().map(|&[n, s]| vec![n, atom(s)]))));
        }
        if self.rx_span {
            out.push(("rx_span", relation(&["p", "c", "j", "g", "m"], &mut self.rx_spans.iter().map(|&[p, c, j, g, m]| vec![atom(p), c, j, atom(g), m]))));
        }
        let mut matched = Vec::new();
        for found in self.matches {
            if found.all {
                matched.push((format!("{}{}", crate::code_rule_dsl::logic::strings::MATCHES, found.k), relation(&["sp", "m"], &mut found.rows.iter().map(|r| r.to_vec()))));
            }
            if found.groups {
                let rows = &mut found.group_rows.iter().map(|&[sp, m, g, gsp]| vec![sp, m, atom(g), gsp]);
                matched.push((format!("{}{}", crate::code_rule_dsl::logic::strings::GROUPS, found.k), relation(&["sp", "m", "g", "gsp"], rows)));
            }
        }
        (out, matched)
    }
}

/// The small relations of a document's nodes, with string indexes: `text(n, s)`, `intval(n, v)`, `op(n, s)` (T's,
/// also bound as `op_t`), and the text relations (decision 0037) `name(n, s)` (an identifier's text, an attribute's
/// dotted text), `str(n, s)` (a string literal's content, between its quotes) and `strprefix(n, p)` (its prefix,
/// lowercased). In `core.rr`'s shape only `intval` and T's `op`: `core_rr` makes the others as `core.rr` states them.
#[derive(Default)]
struct NodeRows {
    text: Vec<[i64; 2]>,
    intval: Vec<[i64; 2]>,
    op: Vec<[i64; 2]>,
    name: Vec<[i64; 2]>,
    strs: Vec<[i64; 2]>,
    strprefix: Vec<[i64; 2]>,
}

impl NodeRows {
    /// Document `p`'s rows (`dialect` its grammar's kinds; `core`: in `core.rr`'s shape).
    fn document(&mut self, p: &Projected, dialect: &Dialect, core: bool, string: &mut dyn FnMut(&str) -> usize) {
        let f = &*p.facts;
        for (local, &id) in p.core.iter().enumerate() {
            let n = p.global(local as u32);
            match dialect.class(f, id) {
                Class::Text | Class::Attribute | Class::String if core => {}
                Class::Text => {
                    let t = f.text(id);
                    if !t.contains(['\t', '\n', '\r', '\\']) {
                        self.text.push([n, string(t) as i64]);
                        self.name.push([n, string(t) as i64]);
                    }
                }
                Class::Attribute => {
                    if let Some(dotted) = dialect.dotted(f, id) {
                        self.name.push([n, string(&dotted) as i64]);
                    }
                }
                Class::String => {
                    if let Some((prefix, content)) = dialect.string(f, id) {
                        self.strprefix.push([n, string(&prefix) as i64]);
                        if let Some(content) = content {
                            self.strs.push([n, string(content) as i64]);
                        }
                    }
                }
                Class::Integer => {
                    if let Ok(v) = f.text(id).replace('_', "").parse::<i64>() {
                        self.intval.push([n, v]);
                    }
                }
                Class::Operator => {
                    let operator = dialect.operator.filter(|&o| o != 0);
                    if let Some(&o) = f.children(id).iter().find(|&&c| operator == Some(f.field[c as usize])) {
                        self.op.push([n, string(dialect.name(f, o)) as i64]);
                    }
                }
                // `async def`: the keyword is an anonymous child, recorded as the definition's modifier.
                // `async for`, `async with` likewise.
                Class::FunctionDefinition | Class::AsyncStatement if f.children(id).first().is_some_and(|&k| dialect.name(f, k) == "async") => {
                    self.op.push([n, string("async") as i64]);
                }
                Class::FunctionItem
                    if f.children(id).iter().any(|&m| {
                        dialect.name(f, m) == "function_modifiers" && f.tokens(m).iter().any(|k| dialect.atom_name(k.kind) == "async")
                    }) =>
                {
                    self.op.push([n, string("async") as i64]);
                }
                _ => {}
            }
        }
    }
}

/// Every batch built in a test, by its documents' keys, held weakly: a run releases its batches when it returns
/// (`logic::rr::tests`), so a chunk's facts are not kept by a binding past it.
#[cfg(any(test, feature = "test-hooks"))]
pub static BATCHES: parking_lot::Mutex<Vec<(Vec<u32>, std::sync::Weak<Batch>)>> = parking_lot::Mutex::new(Vec::new());

impl Batch {
    /// Projects each document and interns the batch's strings; the documents' keys are distinct.
    /// Returns the batch and its small relations (`doc`, `pkg`, `syntax_error`, `text`, `intval`, `op`), and those
    /// of core.rr's the program `reads` that T had not (`Extra`), the `match_all` relations among them (of `matches`)
    /// apart. A text-only document (`core::TEXT`) has the document's relations and its text's, and no node.
    /// Also returns the batch's distinct strings and their symbols: its vocabulary, which text tests read
    /// (`logic::strings::Vocabulary`, decision 0037).
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        store: &ContentStore,
        documents: Vec<Document>,
        projection: &Projection,
        matches: &[Arc<regex::Regex>],
        markers: &[String],
        metal: bool,
    ) -> StoreResult<(Arc<Self>, Vec<(&'static str, Relation)>, Vec<(String, Relation)>, (Vec<String>, Vec<u32>))> {
        let shape = projection.shape;
        let core = shape == Shape::Core;
        // What the program reads, for the relations made only for a program that reads them (`Extra` and those beside
        // it); a projection that names none (T's, `Projection::core`'s) reads none of them.
        let none = BTreeSet::new();
        let reads = projection.reads.as_ref().unwrap_or(&none);
        let mut documents = documents;
        documents.sort_by_key(|d| d.key);
        if documents.windows(2).any(|w| w[0].key == w[1].key) {
            return Err(StoreError::Invalid("a batch holds a document key twice".into()));
        }
        let mut dialects: HashMap<u8, Dialect> = HashMap::new();
        let mut strings: Vec<String> = Vec::new();
        // Every string of every node passes here: Fx, not SipHash (a third of the projection's time with SipHash).
        let mut at: rustc_hash::FxHashMap<String, usize> = rustc_hash::FxHashMap::default();
        let mut string = |s: &str| -> usize {
            if let Some(&i) = at.get(s) {
                return i;
            }
            strings.push(s.to_owned());
            at.insert(s.to_owned(), strings.len() - 1);
            strings.len() - 1
        };
        // Rows with string indexes; the indexes become atoms once the batch's strings are interned.
        let (mut doc, mut pkg) = (Vec::new(), Vec::new());
        // `path_part(d, s)`: each directory of a document's path and its file name; `file_name(d, s)`: the last.
        let (mut path_part, mut file_name): (Vec<[i64; 2]>, Vec<[i64; 2]>) = (Vec::new(), Vec::new());
        // `text`, `intval`, T's `op` and the text relations (`NodeRows`).
        let mut rows = NodeRows::default();
        // `syntax_error(d)`: the documents whose parse has an error (`Facts::syntax_errors`); `core.rr`'s
        // `syntax_error(d, l, msg)` (`core_rr`): the first error's line and message.
        let mut errors = Vec::new();
        let span_text = [crate::code_rule_dsl::logic::front::ast::SpanIn::Source, crate::code_rule_dsl::logic::front::ast::SpanIn::Line].map(|within| !core || projection.wants_span_text(within));
        let mut extra = Extra::new(reads, matches, shape, span_text);
        let mut tree = tree::Tree::new(reads);
        let mut produced: Vec<(Option<u8>, Box<dyn ProducerRun>)> =
            producers().iter().map(|producer| (crate::code_rule_dsl::language::tape_tag(producer.dialect()), producer.start(reads, markers))).collect();
        let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        // `core.rr`'s (decision 0046).
        let mut c = core_rr::CoreRows::default();
        // Node identity/ancestry and line views do not require scalar rows or text interning.
        let scalar_nodes=!core || ["text","name","str","strprefix","value","value_type","static_value","comment","spread","op","op_at","op_t","intval","next_sibling","str_part","str_part_start","str_part_word"]
            .into_iter().any(|r|projection.wants(r));
        let wants_any = |rels: &[&str]| rels.iter().any(|r| projection.wants(r));
        c.skip = core_rr::Skip {
            ops: !wants_any(&["op", "op_at"]),
            next_sibling: !projection.wants("next_sibling"),
            comment: !projection.wants("comment"),
            spread: !projection.wants("spread"),
            name: !projection.wants("name"),
            literal: !wants_any(&["str", "strprefix", "value", "static_value", "value_type", "text"]),
            structural: !wants_any(&["structural_before", "structural_after"]),
        };
        let core_nodes = core && (scalar_nodes || !c.skip.structural);
        let wants_long_text_nodes = projection.wants("rr_long_text_node");
        let mut long_text_nodes = Vec::new();
        c.long_text = projection.long_text.clone();
        c.long_text_any = projection.long_text_any;
        c.text_nodes = projection.text_nodes.as_ref().map(|t| (t.kinds.clone(), t.fields.clone()));
        let wants_text = core && projection.wants("text");
        let wants_source_text = core && projection.wants_span_text(crate::code_rule_dsl::logic::front::ast::SpanIn::Source);
        let wants_line_text = core && projection.wants_span_text(crate::code_rule_dsl::logic::front::ast::SpanIn::Line);
        let wants_split_lines = core && projection.wants("split_line");
        // Asked once for the batch, not per node (a lookup in the program's set of read relations).
        let (wants_parts, wants_part_starts, wants_words) = (projection.wants("str_part"), projection.wants("str_part_start"), projection.wants("str_part_word"));
        let wants_lf_lines = core && projection.wants("lf_line");
        let wants_tokens = core && ["first_token", "last_token", "token_before", "token_after"].iter().any(|r| projection.wants(r));
        // Only a program that reads it: every node's hash is a symbol.
        let wants_trees = core && projection.reads.as_ref().is_some_and(|r| r.contains("tree_digest"));
        let wants_units = core && projection.reads.as_ref().is_some_and(|r| r.contains("def_digest"));
        let wants_digests = wants_trees || wants_units;
        // `rx_span`'s regexes, compiled once for the batch.
        // Over a document's source or its lines, each with its block program (decision 0081 §3): the bytes a match can
        // end after, chunk at a time, so the regex runs only where one can.
        let rx_spans: Vec<core_rr::SpanPattern> = if core {
            projection
                .rx_spans
                .iter()
                .map(|(within, re)| {
                    use crate::code_rule_dsl::logic::front::ast::SpanIn;
                    let on = std::env::var("RERAIL_SPAN_PREFILTER").map_or(true, |v| v != "off");
                    let prefilter = match within {
                        SpanIn::Source | SpanIn::Line if on => crate::code_rule_dsl::facts::span_prefilter::compile(re, *within == SpanIn::Line),
                        _ => None,
                    };
                    regex::Regex::new(re).map(|r| (within.clone(), re.as_str(), r, prefilter)).map_err(|e| StoreError::Invalid(format!("rx_span: bad regex {re:?}: {e}")))
                })
                .collect::<StoreResult<_>>()?
        } else {
            Vec::new()
        };
        // The AST patterns, compiled once per grammar; one a grammar does not parse matches nothing there.
        let mut ast_patterns: HashMap<u8, Vec<(&str, ast_grep_core::matcher::Pattern)>> = HashMap::new();
        // Every prefiltered pattern's marks over every file, in one batch of block programs (decision 0081 §3): one
        // launch of the executor (decision 0100 §13), on the cores or, under the device binding, the GPU, as its price
        // places it. `span_ends[doc][pattern]`.
        let span_ends: Vec<Vec<Option<Vec<u64>>>> = {
            use crate::code_rule_dsl::facts::span_prefilter::{RanOn, SpanEnds, launch as ends_launch};
            let filters: Vec<(usize, &Arc<dyn SpanEnds>)> = rx_spans.iter().enumerate().filter_map(|(i, (_, _, _, f))| f.as_ref().map(|f| (i, f))).collect();
            let mut out: Vec<Vec<Option<Vec<u64>>>> = vec![vec![None; rx_spans.len()]; documents.len()];
            if !filters.is_empty() {
                let mut at = Vec::new();
                let mut sources: Vec<Arc<String>> = Vec::new();
                let mut files: Vec<(usize, usize)> = Vec::new();
                for (j, d) in documents.iter().enumerate().filter(|(_, d)| d.guest.is_none()) {
                    sources.push(Arc::clone(&d.facts.source));
                    for (f, _) in filters.iter().enumerate() {
                        at.push((j, filters[f].0));
                        files.push((sources.len() - 1, f));
                    }
                }
                let _span = crate::code_rule_dsl::logic::profile::span(|| "span prefilter".into());
                let programs: Vec<Arc<dyn SpanEnds>> = filters.iter().map(|(_, f)| Arc::clone(f)).collect();
                let (ends, ran) = ends_launch(crate::code_rule_dsl::facts::core::executor_for(metal), &programs, &sources, &files, metal).map_err(StoreError::Invalid)?;
                crate::code_rule_dsl::logic::profile::decision("span prefilter", || match ran {
                    RanOn::Metal => "metal".into(),
                    RanOn::Cpu => "cpu".into(),
                });
                for ((j, i), e) in at.into_iter().zip(ends) {
                    out[j][i] = Some(e);
                }
            }
            out
        };
        let mut docs: Vec<Projected> = Vec::with_capacity(documents.len());
        // Decision 0055: each document's role, root and tape; each guest's place, and its nodes' host nodes. A guest's
        // key is past every file's (`corpus::guests`), so its host is projected first and its lines are known.
        let mut g = GuestRows::default();
        let mut lines_of: HashMap<u32, Arc<core_rr::Lines>> = HashMap::new();
        // Each guest's file: its host's, through guests of guests. Files' keys are below every guest's, so a file
        // is projected (and its lines known) before its guests; a guest's guests may come before it.
        let host: HashMap<u32, u32> = documents.iter().filter_map(|d| d.guest.as_ref().map(|g| (d.key, g.host))).collect();
        let file_of = |mut k: u32| {
            for _ in 0..=host.len() {
                match host.get(&k) {
                    Some(&h) => k = h,
                    None => break,
                }
            }
            k
        };
        let mut withheld_at = Vec::with_capacity(documents.len());
        // The documents W reads no values of (`Dialect::values`).
        let mut unvalued: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for (j, d) in documents.iter().enumerate() {
            // The document's own dialect (`Facts::tag`): a text-only document has none, and no node.
            let tag = crate::code_rule_dsl::facts::core::facts_tag(&d.facts);
            let dialect = if tag == TEXT {
                None
            } else {
                if let std::collections::hash_map::Entry::Vacant(e) = dialects.entry(tag) {
                    e.insert(Dialect::new(tag)?);
                }
                Some(&dialects[&tag])
            };
            // `core.rr`: a document that does not parse is withheld from the policy (no facts) but its `doc`, path
            // and `syntax_error` rows; its outcome is Unknown(parse). T's shape keeps every document.
            // A guest that does not parse is withheld too, unless the policy keeps its partial tree (decision 0055 §6).
            let guest = d.guest.as_deref().filter(|_| core);
            let error = match dialect {
                Some(dialect) if (core && (projection.wants("syntax_error") || guest.is_some())) || (!core && projection.verdict) => {
                    core_rr::verdict(&d.facts, dialect).map_err(|raised| core_rr::failure(&d.path, &raised))?
                }
                _ => None,
            };
            let withheld = core && error.is_some() && !guest.is_some_and(|g| projection.keeps_partial(&g.grammar));
            let python_lines = dialect.is_some_and(|d| d.family == Family::Python);
            let placed = guest.map(|g| (g, lines_of.get(&file_of(g.host)).cloned().unwrap_or_else(|| Arc::new(core_rr::Lines::shared(Arc::clone(&d.facts.source), python_lines)))));
            let new_span = crate::code_rule_dsl::logic::profile::span(|| "projected".into());
            let mut p = Projected::new(d.key, Arc::clone(&d.facts), dialect, shape, withheld, placed);
            drop(new_span);
            p.module = crate::code_rule_dsl::module_path::ModulePath::dotted(d.module.as_deref().unwrap_or(&module(&d.path)));
            // Its positions (tokens, text, lines) in the top slots (`granules::span_low`): past them ids would clash.
            if p.defs.is_some() && u64::from(p.locs() - p.len()) > granules::POSITIONS {
                return Err(StoreError::Invalid(format!("{} has more than {} positions (tokens and lines: core.rr's span ids)", d.path, granules::POSITIONS)));
            }
            if core && !projection.ast_patterns.is_empty() && guest.is_none() && !withheld
                && let Some(dialect) = dialect
                && let Some(lang) = crate::code_rule_dsl::language::from_tape_tag(tag)
            {
                let compiled = ast_patterns.entry(tag).or_insert_with(|| {
                    projection
                        .ast_patterns
                        .iter()
                        .filter_map(|t| crate::code_rule_dsl::language::rule_pattern(crate::code_rule_dsl::language::PatternSource::Plain(t), lang).ok().map(|p| (t.as_str(), p)))
                        .collect()
                });
                c.patterns(&p, d.tape.as_ref().map(|(t, _)| t), dialect, lang, compiled, &mut string);
            }
            if core && guest.is_none() && let Some(x) = &p.extra {
                lines_of.insert(d.key, Arc::clone(&x.lines));
            }
            withheld_at.push(withheld);
            if dialect.is_some_and(|d| !d.values) {
                unvalued.insert(d.key);
            }
            let key = i64::from(d.key);
            // core.rr: a module for Python and Rust files, "" for the others; `pkg` for Python only. T's has both
            // for every file.
            let py_file = d.path.ends_with(".py") || d.path.ends_with(".pyi");
            let dotted = py_file || d.path.ends_with(".rs");
            let named = d.module.as_deref();
            let module = match named {
                Some(m) => m.to_owned(),
                None if core && !dotted => String::new(),
                None => module(&d.path),
            };
            doc.push([key, string(&d.path) as i64, string(&module) as i64]);
            if !core || py_file {
                let package = match named {
                    Some(m) => package_of(&d.path, m),
                    None => package(&d.path),
                };
                pkg.push([key, string(&package) as i64]);
            }
            let parts: Vec<&str> = d.path.split('/').filter(|s| !s.is_empty()).collect();
            if let Some(last) = parts.last() {
                file_name.push([key, string(last) as i64]);
            }
            let mut seen = std::collections::HashSet::new();
            for part in parts {
                if seen.insert(part) {
                    path_part.push([key, string(part) as i64]);
                }
            }
            if d.facts.syntax_errors {
                errors.push(key);
            }
            let nodes_span = crate::code_rule_dsl::logic::profile::span(|| "node rows".into());
            if let Some(dialect) = dialect {
                if scalar_nodes {
                    rows.document(&p, dialect, core, &mut string);
                }
                if core_nodes || wants_long_text_nodes {
                    for local in 0..p.len() {
                        if wants_long_text_nodes {
                            let id = p.core[local as usize] as usize;
                            if (p.facts.end[id] - p.facts.start[id]) as usize > core_rr::TEXT_INLINE {
                                long_text_nodes.push(p.global(local));
                            }
                        }
                        if core_nodes {
                            c.node(&p, dialect, local, &mut string, wants_text, wants_parts, wants_part_starts, wants_words);
                        }
                    }
                }
                if let Some((line, msg)) = &error {
                    let line = p.extra.as_ref().map_or(*line, |x| x.file_line(*line));
                    c.withheld(&p, &(line, msg.clone()), &mut string);
                }
            }
            drop(nodes_span);
            let document_span = crate::code_rule_dsl::logic::profile::span(|| "document rows".into());
            // A text-only document has core.rr's text rows too (its lines, `split_line`), and no tokens.
            if core && (!withheld || guest.is_none()) {
                c.document(&p, dialect, &mut string, wants_text, wants_tokens && !withheld, wants_split_lines, wants_lf_lines, wants_source_text, wants_line_text, guest.is_none(), projection);
                if let Some(dialect) = dialect
                    && wants_digests
                    && !withheld
                {
                    c.digests(&p, dialect, &mut string, wants_trees, wants_units);
                }
            }
            if d.guest.is_none() {
                // A text document's lines split as W's `lines` regexes split any file's. A withheld document has no
                // `rx_span` matches.
                let rx = if withheld { &[][..] } else { &rx_spans[..] };
                extra.document(&p, d, dialect, python == Some(tag) || tag == TEXT, rx, &span_ends[j], &mut rows.text, &mut string)?;
                tree.document(&p, dialect, python == Some(tag), &mut string);
            } else {
                // A guest's nodes have their places in its file (`Locs::place`); W's values, f-string trees and CPython's
                // verdicts are a file's, read off its tape.
                extra.nodes(&p, &mut string);
            }
            // A guest's are read only by a producer of guests' rows (string annotations); the others' are its file's.
            for (own, run) in &mut produced {
                if d.guest.is_none() || run.reads_guests() {
                    run.document(&p, d, dialect, *own == Some(tag), &mut string)?;
                }
            }
            drop(document_span);
            docs.push(p);
        }
        // The guests' rows, once every document is projected: a guest's host may be a guest after it. Either shape:
        // a program that declares its base relations itself (T's) reads guests as core.rr's do.
        for ((p, d), &withheld) in docs.iter().zip(&documents).zip(&withheld_at) {
            g.document(p, d, d.guest.as_deref(), withheld, &docs, [projection.wants("host_node"), projection.wants("unparse_of")], &mut string);
        }
        // Each lazy family's strings, where the batch has documents of its dialect.
        let vocabulary: Vec<Option<(u8, Vec<i64>)>> = families()
            .iter()
            .map(|f| {
                crate::code_rule_dsl::language::tape_tag(f.dialect())
                    .filter(|tag| dialects.contains_key(tag))
                    .map(|tag| (tag, f.strings().iter().map(|name| string(name) as i64).collect()))
            })
            .collect();
        crate::code_rule_dsl::logic::profile::decision("input payload", || {
            let identities: usize = docs.iter().map(|p| (p.core.capacity() + p.local.capacity() + [&p.idx, &p.size, &p.depth, &p.fielded].iter().map(|v| v.get().map_or(0, |v| v.capacity())).sum::<usize>()) * 4).sum();
            let extras: usize = docs.iter().filter_map(|p| p.extra.as_ref()).map(|x| x.token_spans.get().map_or(0, |v| v.capacity()) * 8).sum();
            serde_json::json!({"documents": docs.len(), "coreColumnBytes": docs.iter().map(|p| p.facts.bytes()).sum::<usize>(), "identityCapacityBytes": identities, "extraSpanCapacityBytes": extras}).to_string()
        });
        crate::code_rule_dsl::logic::profile::decision("projection demand",||serde_json::json!({"coreNodes":if core_nodes {docs.iter().map(|p|p.core.len()).sum::<usize>()}else{0},"scalarNodes":if scalar_nodes {docs.iter().map(|p|p.core.len()).sum::<usize>()}else{0},"nodes":docs.iter().map(|p|p.core.len()).sum::<usize>(),"strings":strings.len(),"reads":projection.reads}).to_string());
        let intern = crate::code_rule_dsl::logic::profile::span(|| "intern atoms".into());
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, strings.len() as u64);
        let atoms = store.intern_atoms(&strings.iter().map(String::as_str).collect::<Vec<_>>())?;
        drop(intern);
        let atom = |i: i64| i64::from(atoms[i as usize]);
        let bound: Vec<Option<Box<dyn BoundFamilies>>> = families()
            .iter()
            .zip(vocabulary)
            .map(|(f, v)| v.map(|(tag, strings)| f.bind(tag, &dialects[&tag].catalog, &strings.into_iter().map(atom).collect::<Vec<_>>(), docs.len())))
            .collect();
        let _relations_span = crate::code_rule_dsl::logic::profile::span(|| "relations".into());
        let relation = |columns: &[&str], rows: &mut dyn Iterator<Item = Vec<i64>>| {
            let mut r = Relation::new(columns);
            rows.for_each(|row| r.push(&row));
            r
        };
        let pairs = |columns: &[&str], rows: &[[i64; 2]], sym: [bool; 2]| {
            relation(columns, &mut rows.iter().map(|&[a, b]| vec![if sym[0] { atom(a) } else { a }, if sym[1] { atom(b) } else { b }]))
        };
        let mut relations = vec![
            ("rr_long_text_node", relation(&["n"], &mut long_text_nodes.iter().map(|&n| vec![n]))),
            ("doc", relation(&["d", "path", "module"], &mut doc.iter().map(|&[d, p, m]| vec![d, atom(p), atom(m)]))),
            ("pkg", pairs(&["d", "p"], &pkg, [false, true])),
            ("path_part", pairs(&["d", "s"], &path_part, [false, true])),
            ("file_name", pairs(&["d", "s"], &file_name, [false, true])),
            ("intval", pairs(&["n", "v"], &rows.intval, [false, false])),
            ("op_t", pairs(&["n", "s"], &rows.op, [false, true])),
        ];
        relations.extend(g.relations(&atom));
        if core {
            extra.literal_values(c.values_of(&unvalued));
        }
        let (mut read, matched) = extra.relations(&atom);
        read.extend(tree.relations(&atom));
        for (_, run) in produced {
            read.extend(run.relations(&atom));
        }
        // The relations made for what the program reads are the batch's: in `core.rr`'s shape they stand in for
        // `core_rr`'s rows and the views of the same name (W's values, the spans and places `match_all` reads,
        // `op_at`'s operators, `visit_index`'s CPython order).
        let made: BTreeSet<&'static str> = read.iter().map(|(name, _)| *name).collect();
        if core {
            // `core_rr`'s `text` and the text of `Extra`'s spans.
            c.extend_text(&rows.text);
            relations.extend(c.relations(&atom).into_iter().filter(|(name, _)| !made.contains(name)));
        } else {
            // T's `syntax_error(d)`, or core.rr's `syntax_error(d, l, msg)` (CPython's verdict) where the program
            // declares that one (`Projection::verdict`).
            let syntax_error = if projection.verdict {
                c.relations(&atom).into_iter().find(|(name, _)| *name == "syntax_error").map(|(_, r)| r).expect("core_rr makes syntax_error")
            } else {
                relation(&["d"], &mut errors.iter().map(|&d| vec![d]))
            };
            relations.extend([
                ("text", pairs(&["n", "s"], &rows.text, [false, true])),
                ("op", pairs(&["n", "s"], &rows.op, [false, true])),
                ("name", pairs(&["n", "s"], &rows.name, [false, true])),
                ("str", pairs(&["n", "s"], &rows.strs, [false, true])),
                ("strprefix", pairs(&["n", "s"], &rows.strprefix, [false, true])),
                ("syntax_error", syntax_error),
            ]);
        }
        relations.extend(read);
        let offsets = |count: &dyn Fn(&Projected) -> usize| {
            let mut v = vec![0];
            docs.iter().for_each(|p| v.push(v.last().unwrap() + count(p)));
            v
        };
        let nodes = offsets(&|p| p.core.len());
        let children = offsets(&|p| p.core.len().saturating_sub(1));
        let fields = offsets(&|p| if projection.wants("field") { p.fielded().len() } else { 0 });
        let locs = OnceLock::new();
        let batch = Arc::new(Batch { docs, dialects, nodes, children, fields, locs, shape, families: bound, made });
        hold_texts(&batch);
        #[cfg(any(test, feature = "test-hooks"))]
        BATCHES.lock().push((batch.docs.iter().map(|p| p.key).collect(), Arc::downgrade(&batch)));
        Ok((batch, relations, matched, (strings, atoms)))
    }

    fn offsets(&self, view: View) -> &[usize] {
        match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order => &self.nodes,
            View::Child => &self.children,
            View::Field => &self.fields,
            View::Loc(_) => self.locs.get_or_init(|| {
                let mut offsets = Vec::with_capacity(self.docs.len() + 1);
                offsets.push(0);
                for p in &self.docs { offsets.push(offsets.last().unwrap() + p.locs() as usize); }
                offsets
            }),
        }
    }
    /// The document and its row index of `row`.
    /// Rows are read in runs of one document, so the last document found (per thread, per offsets array) is tried
    /// before the search: its range is checked against the offsets, so a stale hint costs a comparison, never a
    /// wrong document. The search per row was 13 s of the pack's cold run.
    pub fn locate(&self, view: View, row: usize) -> (&Projected, usize) {
        let offsets = self.offsets(view);
        let key = offsets.as_ptr() as usize;
        let doc = LOCATED.with(|h| {
            let (k, d) = h.get();
            if k == key && d + 1 < offsets.len() && offsets[d] <= row && row < offsets[d + 1] {
                return d;
            }
            let d = offsets.partition_point(|&o| o <= row) - 1;
            h.set((key, d));
            d
        });
        (&self.docs[doc], row - offsets[doc])
    }
    /// The document of key `d`, by index (the last one found tried first, as `locate`).
    fn doc(&self, d: i64) -> Option<usize> {
        let d = u32::try_from(d).ok()?;
        let key = self.docs.as_ptr() as usize;
        let hit = DOC.with(|h| {
            let (k, i) = h.get();
            (k == key && self.docs.get(i).is_some_and(|p| p.key == d)).then_some(i)
        });
        if hit.is_some() {
            return hit;
        }
        let i = self.docs.binary_search_by_key(&d, |p| p.key).ok()?;
        DOC.with(|h| h.set((key, i)));
        Some(i)
    }
    /// The document index and node of id `n`.
    fn node(&self, n: i64) -> Option<(usize, u32)> {
        let doc = self.doc(n >> 32)?;
        let local = self.docs[doc].local_of((n & 0xffff_ffff) as u32)?;
        (n >= 0 && local < self.docs[doc].len()).then_some((doc, local))
    }
    /// The document index and node or span of id `n`.
    fn loc(&self, n: i64) -> Option<(usize, u32)> {
        let doc = self.doc(n >> 32)?;
        let local = self.docs[doc].local_of((n & 0xffff_ffff) as u32)?;
        (n >= 0 && (local < self.docs[doc].len() || local < self.docs[doc].locs())).then_some((doc, local))
    }
    /// The view's row of node `local` of document `doc`, if it has one.
    fn row(&self, view: View, doc: usize, local: u32) -> Option<usize> {
        let p = &self.docs[doc];
        let within = match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order | View::Loc(_) => Some(local as usize),
            View::Child => (local > 0).then(|| local as usize - 1),
            View::Field => p.fielded().binary_search(&local).ok(),
        }?;
        Some(self.offsets(view)[doc] + within)
    }
    /// The rows of nodes with ids in `lo..=hi`.
    fn between(&self, view: View, lo: i64, hi: i64) -> Vec<usize> {
        let mut rows = Vec::new();
        let first = self.docs.partition_point(|p| i64::from(p.key) < lo >> 32);
        for (doc, p) in self.docs.iter().enumerate().skip(first) {
            let base = i64::from(p.key) << 32;
            if base > hi {
                break;
            }
            // Ids by definition are not in local order: the locals whose ids fall in the range.
            if p.defs.is_some() {
                let count = if matches!(view, View::Loc(_)) { p.locs() } else { p.len() };
                let mut locals: Vec<u32> = (0..count).filter(|&l| (lo..=hi).contains(&p.global(l))).collect();
                locals.sort_unstable();
                rows.extend(locals.into_iter().filter_map(|local| self.row(view, doc, local)));
                continue;
            }
            let start = (lo.max(base) - base) as u32;
            let count = if matches!(view, View::Loc(_)) { p.locs() } else { p.len() };
            let end = ((hi - base).min(i64::from(count) - 1) + 1).max(0) as u32;
            rows.extend((start..end.max(start)).filter_map(|local| self.row(view, doc, local)));
        }
        rows
    }
}

struct Viewed {
    batch: Arc<Batch>,
    view: View,
}
impl Viewed {
    fn columns(view: View) -> &'static [&'static str] {
        match view {
            View::Node => &["d", "n", "kind", "start", "end_"],
            View::Line => &["n", "l"],
            View::Child => &["p", "c", "idx"],
            View::Field => &["p", "f", "c"],
            View::Tape => &["n", "par", "size", "depth"],
            View::Core => &["d", "t", "n"],
            View::Depth => &["n", "k"],
            View::Order => &["n", "k"],
            View::Loc(Place::Line | Place::EndLine) => &["n", "l"],
            View::Loc(Place::Col) => &["n", "c"],
            View::Loc(Place::Docof) => &["n", "d"],
        }
    }
    /// The column holding the row's own node, which ranges and direct lookups address.
    fn own(&self) -> usize {
        match self.view {
            View::Node | View::Child => 1,
            View::Line | View::Depth | View::Order | View::Loc(_) => 0,
            View::Field | View::Core => 2,
            View::Tape => 0,
        }
    }
    /// The column holding the row's parent, looked up through the children lists.
    fn parent(&self) -> Option<usize> {
        match self.view {
            View::Child | View::Field => Some(0),
            View::Tape => Some(1),
            View::Node | View::Line | View::Core | View::Depth | View::Order | View::Loc(_) => None,
        }
    }
}
impl Source for Viewed {
    fn python_batch(&self)->Option<Arc<Batch>> {Some(Arc::clone(&self.batch))}
    fn columns(&self) -> Vec<String> {
        Self::columns(self.view).iter().map(|c| (*c).to_owned()).collect()
    }
    fn len(&self) -> usize {
        *self.batch.offsets(self.view).last().unwrap()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        let per = if column == self.own() {
            1
        } else if Some(column) == self.parent() {
            4
        } else if matches!(self.view, View::Node | View::Core) && column == 0 {
            self.len() / self.batch.docs.len().max(1)
        } else if matches!(self.view, View::Order) && column == 1 {
            1
        } else if matches!(self.view, View::Core) && column == 1 {
            // One probe per document of the batch.
            self.batch.docs.len()
        } else {
            (self.len() / 64).max(1)
        };
        if batch { per * 8 } else { per }
    }
    fn indexed(&self, column: usize) -> bool {
        column == self.own()
            || Some(column) == self.parent()
            || matches!(self.view, View::Node) && matches!(column, 0 | 2)
            || matches!(self.view, View::Core) && matches!(column, 0 | 1)
            || matches!(self.view, View::Order) && column == 1
    }
    fn range_indexed(&self, column: usize) -> bool {
        column == self.own()
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        if column != self.own() {
            return None;
        }
        let lo = match lower {
            None => 0,
            Some((v, inclusive)) => v.saturating_add(i64::from(!inclusive)),
        };
        let hi = match upper {
            None => i64::MAX,
            Some((v, inclusive)) => v.saturating_sub(i64::from(!inclusive)),
        };
        Some(if lo > hi { Vec::new() } else { self.batch.between(self.view, lo, hi) })
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.lookup_docs(column, values, None)
    }
    fn doc_column(&self) -> Option<usize> {
        matches!(self.view, View::Node | View::Core).then_some(0)
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        matches!((self.view, column), (View::Node, 2) | (View::Core, 1))
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        if !self.kept_to_doc(column) {
            return None;
        }
        // A document not in the batch has no rows.
        Some(match self.batch.doc(doc) {
            Some(doc) => self.lookup_docs(column, values, Some(doc))?,
            None => Vec::new(),
        })
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.row_value(row, column)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let (p, i) = self.batch.locate(self.view, row);
        for (o, &c) in out.iter_mut().zip(columns) {
            *o = self.value_at(p, i, c);
        }
    }
}
impl Viewed {
    /// The rows whose `column` is one of `values`, in document `only` (by index) where the column spans the
    /// documents (a kind, a tape node), else in every document.
    fn lookup_docs(&self, column: usize, values: &[i64], only: Option<usize>) -> Option<Vec<usize>> {
        let b = &*self.batch;
        let kept = |doc: &usize| only.is_none_or(|o| o == *doc);
        let mut rows = Vec::new();
        for &v in values {
            if column == self.own() {
                let found = if matches!(self.view, View::Loc(_)) { b.loc(v) } else { b.node(v) };
                rows.extend(found.and_then(|(doc, local)| b.row(self.view, doc, local)));
            } else if Some(column) == self.parent() {
                if v == -1 && matches!(self.view, View::Tape) {
                    rows.extend((0..b.docs.len()).filter(|&doc| b.docs[doc].len() > 0).map(|doc| b.nodes[doc]));
                } else if let Some((doc, local)) = b.node(v) {
                    rows.extend(b.docs[doc].children(local).filter_map(|c| b.row(self.view, doc, c)));
                }
            } else if matches!(self.view, View::Order) && column == 1 {
                // An order key: its document, and the node with that key (`Projected::local_of_order`).
                let key = (v & 0xffff_ffff) as u32;
                rows.extend(b.doc(v >> 32).filter(|_| v >= 0).and_then(|doc| b.docs[doc].local_of_order(key).and_then(|local| b.row(self.view, doc, local))));
            } else if matches!(self.view, View::Node | View::Core) && column == 0 {
                rows.extend(b.doc(v).map_or(0..0, |doc| b.nodes[doc]..b.nodes[doc + 1]));
            } else if matches!(self.view, View::Core) && column == 1 {
                // A tape node of each document that has it as a node; the statement filters `d`.
                let Ok(t) = usize::try_from(v) else { continue };
                for (doc, p) in b.docs.iter().enumerate().filter(|(doc, _)| kept(doc)) {
                    if let Some(&local) = p.local.get(t).filter(|&&local| local != NONE) {
                        rows.push(b.nodes[doc] + local as usize);
                    }
                }
            } else if matches!(self.view, View::Node) && column == 2 {
                let (Ok(tag), Ok(code)) = (u8::try_from(v / i64::from(KIND_STRIDE)), u16::try_from(v % i64::from(KIND_STRIDE))) else {
                    continue;
                };
                let Some(dialect) = b.dialects.get(&tag).filter(|_| v >= 0) else { continue };
                let atoms: Vec<u32> = dialect.codes.get(&code).map_or(&[][..], Vec::as_slice).iter().map(|&c| kind_atom(tag, c)).collect();
                for (doc, p) in b.docs.iter().enumerate().filter(|(doc, p)| p.tag == tag && kept(doc)) {
                    for &kind in &atoms {
                        rows.extend(p.facts.of_kind(kind).iter().map(|&id| p.local[id as usize]).filter(|&l| l != NONE).map(|l| b.nodes[doc] + l as usize));
                    }
                }
            } else {
                return None;
            }
        }
        Some(rows)
    }
    fn row_value(&self, row: usize, column: usize) -> i64 {
        let (p, i) = self.batch.locate(self.view, row);
        self.value_at(p, i, column)
    }
    /// Document by document, with no search for each row's document.
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        let offsets = self.batch.offsets(self.view);
        let mut out = Vec::new();
        for (doc, p) in self.batch.docs.iter().enumerate() {
            let base = offsets[doc];
            for i in 0..offsets[doc + 1] - base {
                if eqs.iter().all(|&(c, v)| self.value_at(p, i, c) == v) {
                    out.push((base + i) as u32);
                }
            }
        }
        out
    }
}

impl Viewed {
    /// Column `column` of row `i` of document `p` in this view.
    fn value_at(&self, p: &Projected, i: usize, column: usize) -> i64 {
        if let View::Loc(place) = self.view {
            let local = i as u32;
            if column == 0 {
                return p.global(local);
            }
            if matches!(place, Place::Docof) { return i64::from(p.key); }
            let (line, last, col) = p.extra.as_ref().expect("core.rr's lines").place(p.span(local));
            return match place {
                Place::Line => i64::from(line),
                Place::EndLine => i64::from(last),
                Place::Col => i64::from(col),
                Place::Docof => i64::from(p.key),
            };
        }
        let local = match self.view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order | View::Loc(_) => i as u32,
            View::Child => i as u32 + 1,
            View::Field => p.fielded()[i],
        };
        let l = local as usize;
        let id = p.core[l] as usize;
        let parent = || if p.parent(l) == NONE { -1 } else { p.global(p.parent(l)) };
        match (self.view, column) {
            (View::Node | View::Core, 0) => i64::from(p.key),
            (View::Core, 1) => id as i64,
            (View::Node, 2) => self.batch.dialects[&p.tag].kind(&p.facts, id as u32),
            (View::Node, 3) => i64::from(p.facts.start[id]),
            (View::Node, 4) => i64::from(p.facts.end[id]),
            (View::Line, 1) => i64::from(p.facts.start_line(id as u32)),
            (View::Child | View::Field, 0) | (View::Tape, 1) => parent(),
            (View::Child, 2) => i64::from(p.idx(l)),
            (View::Field, 1) => i64::from(field_atom(p.tag, p.facts.field[id])),
            (View::Tape, 2) => i64::from(p.size(l)),
            (View::Tape, 3) => i64::from(p.depth(l)),
            (View::Depth, 1) => i64::from(p.depth(l)),
            (View::Order, 1) => i64::from(p.key) << 32 | i64::from(p.order(local)),
            _ => p.global(local),
        }
    }
}

/// The field atom of field `code` in the dialect whose tape tag is `tag`.
pub fn field_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) << 16 | u32::from(code)
}

/// Document `key`'s `function_definition` and `class_definition` nodes by byte range (start, end), as ids of
/// `node`: what W's call summaries name a definition by (`calls::summary::Def`).
pub fn definitions(key: u32, facts: &Arc<Facts>) -> StoreResult<HashMap<(u32, u32), i64>> {
    let dialect = Dialect::new(crate::code_rule_dsl::facts::core::facts_tag(facts))?;
    let p = Projected::new(key, Arc::clone(facts), Some(&dialect), Shape::T, false, None);
    Ok(p.core
        .iter()
        .enumerate()
        .filter(|(_, id)| matches!(dialect.name(facts, **id), "function_definition" | "class_definition"))
        .map(|(local, &id)| ((facts.start[id as usize], facts.end[id as usize]), p.global(local as u32)))
        .collect())
}

/// T's nine base relations over `documents`, and the `py.*` families, by table name.
pub fn sources(store: &ContentStore, documents: Vec<Document>) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    shaped(store, documents, &Projection::default())
}

/// The base relations over `documents` in `projection`'s shape, and the `py.*` families, by table name.
pub fn shaped(store: &ContentStore, documents: Vec<Document>, projection: &Projection) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    let (batch, relations, _, _) = Batch::build(store, documents, projection, &[], &[], false)?;
    let mut out: Vec<(&'static str, Arc<dyn Source>)> =
        relations.into_iter().map(|(name, r)| (name, Arc::new(r) as Arc<dyn Source>)).collect();
    out.extend(views(&batch));
    Ok(out)
}

/// The batch's views: `node`, `child`, `field`, `tape` and the `py.*` families.
fn views(batch: &Arc<Batch>) -> Vec<(&'static str, Arc<dyn Source>)> {
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = Vec::new();
    let mut named = vec![("node", View::Node), ("child", View::Child), ("field", View::Field), ("tape", View::Tape), ("core_node", View::Core), ("node_order", View::Order)];
    match batch.shape {
        Shape::T => named.push(("line", View::Line)),
        Shape::Core => named.extend([
            ("line", View::Loc(Place::Line)),
            ("end_line", View::Loc(Place::EndLine)),
            ("col", View::Loc(Place::Col)),
            // `core.rr`'s `docof(x: loc, d)` is an input (`rr::lower`): nodes' and spans' documents.
            ("docof", View::Loc(Place::Docof)),
            ("ast_depth", View::Depth),
        ]),
    }
    // A relation the batch made as rows (`Batch::made`: its spans have places too) is bound in place of the view.
    for (name, view) in named.into_iter().filter(|(name, _)| !batch.made.contains(name)) {
        out.push((name, Arc::new(Viewed { batch: Arc::clone(batch), view })));
    }
    for (at, families) in families().iter().enumerate() {
        for (family, &name) in families.names().iter().enumerate() {
            let empty = Relation::new(families.columns()[family]);
            out.push((name, Arc::new(FamilySource { batch: Arc::clone(batch), at, family, empty })));
        }
    }
    out
}


impl Batch {

    /// The node id of row `row` of the node view.
    pub fn id(&self, row: usize) -> i64 {
        let (p, local) = self.locate(View::Node, row);
        p.global(local as u32)
    }
}

/// T's base relations over `documents` (`sources`) in `projection`'s shape and those of core.rr's the program reads
/// that T had not (`Extra`), each of `tests`' relations, and each of `sets` (a relation and its set's kind atoms):
/// `rr_nearest_<set>(n, a)`, the nearest ancestor in the set, `rr_inside_<set>(n, a)`, every one (decision 0036), or
/// `rr_has_<set>(n, a)`, n in the set and a each of its strict ancestors. Each is made per looked-up node on the host
/// (`within::Axis`), or for the whole batch by the transducer's file-local FIX (`typing::file_local::nearest`, bound
/// `binding`) where the planner expects more lookups than that costs (decision 0105).
pub fn with_nearest(
    store: &ContentStore,
    documents: Vec<Document>,
    sets: &[(String, Vec<i64>)],
    tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
    binding: crate::code_rule_dsl::facts::nearest::Binding,
    projection: &Projection,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    with_matchers(store, documents, sets, &Matchers::default(), tests, binding, projection)
}

/// A program's `ast` matchers (decision 0106) as one wave over a batch's tapes (decision 0107), which the engine
/// compiles from the matchers' text (`AstCompiler`): facts runs the wave where `with_matchers` binds, and keeps no
/// matcher of its own.
pub trait AstWave: Send + Sync {
    /// How many matchers the wave runs.
    fn len(&self) -> usize;
    /// Each matcher's relation over `tapes` (each a batch document's facts, tape and path): its matched nodes, and
    /// the captures it binds.
    fn matched(
        &self,
        batch: &Batch,
        tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        atoms: &crate::content_store::atoms::Atoms,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>>;
}

/// A program's matchers, as the engine compiled them; none by default.
#[derive(Clone, Default)]
pub struct Matchers(Option<Arc<dyn AstWave>>);

impl Matchers {
    pub fn new(wave: Arc<dyn AstWave>) -> Self {
        Self(Some(wave))
    }
    pub fn len(&self) -> usize {
        self.0.as_ref().map_or(0, |wave| wave.len())
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// What compiles a program's `ast` matchers (`rel`, the matcher's text, its captures) over its first `.policy`: the
/// engine's (`waves::policies`), registered at load.
pub trait AstCompiler: Send + Sync {
    fn compile(&self, matchers: &[(String, String, Vec<String>)], policies: &[serde_json::Value]) -> Result<Matchers, String>;
}

static AST_COMPILER: std::sync::OnceLock<&'static dyn AstCompiler> = std::sync::OnceLock::new();

/// Registers the compiler of `ast` matchers (once, at load).
pub fn install_ast_compiler(compiler: &'static dyn AstCompiler) {
    let _ = AST_COMPILER.set(compiler);
}

/// `matchers` compiled; a program with matchers and no registered compiler fails, naming the first.
pub fn compile_matchers(matchers: &[(String, String, Vec<String>)], policies: &[serde_json::Value]) -> Result<Matchers, String> {
    if matchers.is_empty() {
        return Ok(Matchers::default());
    }
    match AST_COMPILER.get() {
        Some(compiler) => compiler.compile(matchers, policies),
        None => Err(format!("{}: no compiler of `ast` matchers is registered", matchers[0].0)),
    }
}

/// [`with_nearest`], with each of `matchers`' relations over the documents' tapes (decision 0106).
pub fn with_matchers(
    store: &ContentStore,
    documents: Vec<Document>,
    sets: &[(String, Vec<i64>)],
    matchers: &Matchers,
    tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
    binding: crate::code_rule_dsl::facts::nearest::Binding,
    projection: &Projection,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    let built = crate::code_rule_dsl::logic::profile::span(|| "project and intern".into());
    let metal = matches!(binding, crate::code_rule_dsl::facts::nearest::Binding::Metal);
    let tapes: Vec<(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)> = if matchers.is_empty() {
        Vec::new()
    } else {
        documents.iter().filter(|d| d.guest.is_none()).filter_map(|d| Some((Arc::clone(&d.facts), d.tape.as_ref()?.0.clone(), d.path.clone()))).collect()
    };
    let (batch, relations, match_all, (strings, symbols)) = Batch::build(store, documents, projection, &tests.matches, &tests.markers, metal)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, batch.nodes.last().copied().unwrap_or(0) as u64);
    drop(built);
    // The batch's relations and views, but none the program derives (`Projection::derived`).
    let mut out: Vec<(String, Arc<dyn Source>)> =
        relations.into_iter().filter(|(name, _)| !projection.derived.contains(*name)).map(|(name, r)| (name.to_owned(), Arc::new(r) as Arc<dyn Source>)).collect();
    // The `match_all` relations (`logic::strings::as_relations`): each regex's matches in the batch's spans.
    out.extend(match_all.into_iter().map(|(name, r)| (name, Arc::new(r) as Arc<dyn Source>)));
    if !matchers.is_empty() {
        if let Some(wave) = &matchers.0 {
            out.extend(wave.matched(&batch, &tapes, binding, &store.atoms_handle())?);
        }
    }
    let rows_named: BTreeSet<String> = out.iter().map(|(n, _)| n.clone()).collect();
    // The text tests' relations (decision 0037): the batch's strings that pass each, each string tested once a run.
    let text = crate::code_rule_dsl::logic::profile::span(|| "text tests".into());
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, strings.len() as u64);
    for ((name, _), rows) in tests.tests.iter().zip(tests.rows(&strings, &symbols)) {
        let mut r = Relation::new(&["s"]);
        rows.into_iter().for_each(|s| r.push(&[s]));
        out.push((name.clone(), Arc::new(r)));
    }
    drop(text);
    let viewing = crate::code_rule_dsl::logic::profile::span(|| "views".into());
    out.extend(views(&batch).into_iter().filter(|(name, _)| !projection.derived.contains(*name)).map(|(name, source)| (name.to_owned(), source)));
    drop(viewing);
    // The kind-set axes, made per looked-up node like containment (decision 0050): a set no join reads, and the nodes
    // no join binds, cost nothing. Their statistics come from the tree.
    let _nearest = crate::code_rule_dsl::logic::profile::span(|| "nearest".into());
    use crate::code_rule_dsl::logic::front::ast::{AXIS, DESCENDANTS};
    for (name, members) in sets {
        let axis = if name.starts_with(DESCENDANTS) {
            within::Axis::Has
        } else if name.starts_with(AXIS) {
            within::Axis::Inside
        } else {
            within::Axis::Nearest
        };
        out.push((name.clone(), Arc::new(within::Within::axis(Arc::clone(&batch), axis, members, binding))));
    }
    // The stored-projection spike (measurement only): the relations the program reads, but the per-node axes.
    if let Some(db) = std::env::var_os("RERAIL_SPIKE_PROJECTION") {
        let viewed: BTreeSet<&str> = views(&batch).iter().map(|(n, _)| *n).collect();
        let read: Vec<(String, Arc<dyn Source>, stored_spike::Kind)> = out
            .iter()
            .filter(|(n, _)| projection.wants(n))
            .filter_map(|(n, s)| {
                let kind = if rows_named.contains(n) { stored_spike::Kind::Rows } else if viewed.contains(n.as_str()) { stored_spike::Kind::View } else { return None };
                Some((n.clone(), Arc::clone(s), kind))
            })
            .collect();
        let keys: Vec<u32> = batch.docs.iter().map(|p| p.key).collect();
        drop(_nearest);
        stored_spike::run(&db, &read, &keys);
        return Ok(out_with_within(out, &batch));
    }
    // Containment (decision 0050): made per looked-up node, so a program that never reads it pays nothing.
    Ok(out_with_within(out, &batch))
}

/// `out` with containment's two relations (decision 0050).
fn out_with_within(mut out: Vec<(String, Arc<dyn Source>)>, batch: &Arc<Batch>) -> Vec<(String, Arc<dyn Source>)> {
    out.push((WITHIN.to_owned(), Arc::new(within::Within::new(Arc::clone(batch), within::Mode::Tree))));
    out.push((SPAN_WITHIN.to_owned(), Arc::new(within::Within::new(Arc::clone(batch), within::Mode::Span))));
    out
}

/// The planner's statistics of `source` (`stat_col`'s rows, ndv, sq and sqd per column, `sqd` by
/// document `d = id >> 32`), for a relation made at bind time rather than at ingestion.
pub fn statistics(source: &dyn Source) -> Vec<(String, i64, i64, i64, i64)> {
    if let Some(known) = source.statistics() {
        return known;
    }
    measured(source)
}

/// `statistics`, measured from the rows (a source that keeps its own calls this once).
pub fn measured(source: &dyn Source) -> Vec<(String, i64, i64, i64, i64)> {
    let rows = source.len();
    // Each row's document (the first column's high half), read once for every column.
    let docs: Vec<i64> = (0..rows).map(|row| source.value(row, 0) >> 32).collect();
    // One sort per column by (value, document): a run of one value is its postings (ndv, and the sum of squares of
    // their lengths), and within it a run of one document is that document's (the sum of squares per document).
    source
        .columns()
        .into_iter()
        .enumerate()
        .map(|(c, name)| {
            // A key column (strictly increasing, as a node column of a relation in node order): every value once,
            // and so every (document, value) once. No sort.
            if (1..rows).all(|row| source.value(row - 1, c) < source.value(row, c)) {
                return (name, rows as i64, rows as i64, rows as i64, rows as i64);
            }
            let mut values: Vec<(i64, i64)> = (0..rows).map(|row| (source.value(row, c), docs[row])).collect();
            values.sort_unstable();
            let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
            let shadow = crate::code_rule_dsl::logic::plan::sketch::shadowing();
            let mut sketch = crate::code_rule_dsl::logic::plan::sketch::Sketch::default();
            let mut at = 0;
            while at < values.len() {
                let v = values[at].0;
                let end = at + values[at..].partition_point(|x| x.0 == v);
                if shadow {
                    sketch.add(v, (end - at) as i64);
                }
                ndv += 1;
                sq += ((end - at) * (end - at)) as i64;
                let mut d = at;
                while d < end {
                    let doc = values[d].1;
                    let de = d + values[d..end].partition_point(|x| x.1 == doc);
                    sqd += ((de - d) * (de - d)) as i64;
                    d = de;
                }
                at = end;
            }
            if shadow {
                crate::code_rule_dsl::logic::plan::sketch::shadow("measured", &name, rows as i64, ndv, sq, &sketch);
            }
            (name, rows as i64, ndv, sq, sqd)
        })
        .collect()
}

/// Fails if a document's facts are not those its `#core:` name binds now (§4).
pub fn fresh(graph: &(impl Graph + ?Sized), documents: &[Document]) -> StoreResult<()> {
    for d in documents {
        if graph.binding(&d.core)? != Some(d.digest) {
            return Err(StoreError::Invalid(format!("stale core facts for {}", d.core)));
        }
    }
    Ok(())
}

/// Binds T's base relations over `documents` on `db`, after checking that each document's facts are
/// those its `#core:` name binds now (§4): a stale document fails the binding.
pub fn bind(modules: &Modules, db: &Connection, graph: &(impl Graph + ?Sized), documents: Vec<Document>) -> StoreResult<()> {
    fresh(graph, &documents)?;
    for (name, source) in sources(graph.store(), documents)? {
        modules.bind(db, name, source).map_err(StoreError::Database)?;
    }
    Ok(())
}

// cratify: `tests` is rerail-engine's (it mounts it)

thread_local! {
    /// `Batch::locate`'s hint: the offsets array (its address) and the document last found in it.
    static LOCATED: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// `Batch::doc`'s hint: the documents (their address) and the index last found.
    static DOC: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

// cratify: `candidate_spike` is rerail-lang-python's (it mounts it)
// cratify: `annotation` is rerail-lang-python's (it mounts it)
// cratify: `cpython` is rerail-lang-python's (it mounts it)
// cratify: `fstring` is rerail-lang-python's (it mounts it)
pub mod tree;
// cratify: `ast_wave` is rerail-engine's (it mounts it)
// cratify: `py_families` is rerail-lang-python's (it mounts it)
// cratify: `python_core` is rerail-lang-python's (it mounts it)
// cratify: `ecma_core` is rerail-lang-ecma's (it mounts it)
// cratify: `rr_core` is rerail-logic's (it mounts it)
