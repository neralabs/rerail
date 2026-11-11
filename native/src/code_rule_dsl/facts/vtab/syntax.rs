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
use crate::code_rule_dsl::facts::core::{Facts, KIND_STRIDE, NONE, kind_atom};
use crate::code_rule_dsl::facts::producers::python::families as py;
use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::Connection;
use crate::code_rule_dsl::logic::front::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

mod within;

/// The language's `within(n, a)`, `n` strictly below `a` (`within::Within`, tree mode).
pub(crate) const WITHIN: &str = "within";
/// `span_within(inner, outer)`, `outer`'s span holding `inner`'s (`within::Within`, span mode).
pub(crate) const SPAN_WITHIN: &str = "span_within";
mod core_rr;
mod stored_spike;
pub(crate) mod normalized;
mod python_order;
mod python_syntax;
mod python_demand;
mod candidates;
mod python_parts;
pub(in crate::code_rule_dsl) mod python_print;
use core_rr::Family;
mod guests;
mod granules;
use guests::GuestRows;

/// Which projection of the tree a batch serves.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Shape {
    /// T's: named nodes without comments, `text` of identifiers, `op` of operator fields.
    #[default]
    T,
    /// `core.rr`'s (decision 0046).
    Core,
}

/// The projection a run binds: its shape, and the relations its program reads (`None`: all). A relation no rule
/// reads is not made where making it costs (`text` of every node, the tokens).
#[derive(Clone, Debug, Default)]
pub(crate) struct Projection {
    pub(crate) shape: Shape,
    pub(crate) reads: Option<BTreeSet<String>>,
    /// The regexes the program runs over `source` or `src_line` spans (`front::Program::rx_spans`): their matches'
    /// groups are spans, rows of `rx_span`.
    pub(crate) rx_spans: Vec<(crate::code_rule_dsl::logic::front::SpanIn, String)>,
    /// The AST patterns the program matches (`front::Program::ast_patterns`): rows of `ast_match`.
    pub(crate) ast_patterns: Vec<String>,
    /// Source/line spans whose complete text a rule reads, traced through copied relation columns.
    /// None is the unprojected test/export interface; AST subtree text keeps its independent bound.
    text_spans: Option<BTreeSet<crate::code_rule_dsl::logic::front::SpanIn>>,
    /// The node kinds whose text a rule reads (`text_kind_demand`): their nodes' `text` is made at any length. Any
    /// other node's is made up to `core_rr::TEXT_INLINE` bytes, unless it is token-like, or `long_text_any`: some rule
    /// reads the text of a node of any kind.
    long_text: BTreeSet<String>,
    long_text_any: bool,
    /// The grammars whose guests keep their partial tree when they do not parse (`.on guest_error partial`,
    /// decision 0055 §6); `"*"` is every grammar. Any other broken guest is withheld.
    pub(crate) partial: BTreeSet<String>,
}

impl Projection {
    pub(crate) fn core() -> Self {
        Projection { shape: Shape::Core, reads: None, rx_spans: Vec::new(), ast_patterns: Vec::new(), text_spans: None, long_text: BTreeSet::new(), long_text_any: false, partial: BTreeSet::new() }
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
    pub(crate) fn of(program: &crate::code_rule_dsl::logic::front::Program, reads: BTreeSet<String>) -> Self {
        crate::code_rule_dsl::logic::profile::projection_plan(program, &reads);
        let core = crate::code_rule_dsl::logic::rr::core_names();
        let shape = if !core.is_empty() && core.iter().all(|r| program.decls.contains_key(r)) { Shape::Core } else { Shape::T };
        let partial = program.facts.get(GUEST_POLICY).into_iter().flatten().filter_map(|row| match row.as_slice() {
            [Value::Str(k), Value::Str(v)] if v == "partial" => k.strip_prefix("errors").map(|g| g.strip_prefix(':').unwrap_or("*").to_owned()),
            _ => None,
        });
        Projection { shape, reads: Some(reads), rx_spans: program.rx_spans.iter().cloned().collect(), ast_patterns: program.ast_patterns.iter().cloned().collect(), text_spans: Some(span_text_demand(program)), long_text: BTreeSet::new(), long_text_any: false, partial: partial.collect() }
    }

    /// This projection, with `text` made at any length for the node kinds whose text the program's rules read
    /// ([`text_kind_demand`]; `matchers` are the program's `ast` relations, by relation, as `matchers::lower` gives them).
    pub(crate) fn with_long_text(mut self, program: &crate::code_rule_dsl::logic::front::Program, matchers: &[(String, String, Vec<String>)]) -> Self {
        let any;
        (self.long_text, any) = text_kind_demand(program, matchers);
        self.long_text_any = !any.is_empty();
        crate::code_rule_dsl::logic::profile::decision("text past the inline bound", || {
            if self.long_text_any { format!("every node ({} rules read the text of a node of any kind)", any.len()) } else { format!("{} kinds: {}", self.long_text.len(), self.long_text.iter().take(12).cloned().collect::<Vec<_>>().join(", ")) }
        });
        self
    }

    fn wants_span_text(&self, within: crate::code_rule_dsl::logic::front::SpanIn) -> bool {
        use crate::code_rule_dsl::logic::front::SpanIn;
        self.wants("text") && self.text_spans.as_ref().map_or_else(
            || self.wants(match within { SpanIn::Source => "source", SpanIn::Line => "src_line", _ => "text" }),
            |spans| spans.contains(&within),
        )
    }
}

/// Whether `v` is a node in a field whose every kind is a leaf (`field(x, "attribute", v)`): a token's text, which
/// `text` holds at any length (`core_rr::TEXT_INLINE`).
fn leaf_field(body: &[crate::code_rule_dsl::logic::front::Literal], v: &str) -> bool {
    use crate::code_rule_dsl::logic::front::Term;
    use crate::code_rule_dsl::logic::plan::corpus::matchers::{field_kinds, leaf_kind};
    body.iter().filter_map(crate::code_rule_dsl::logic::front::Literal::atom).filter(|a| !a.neg && a.rel == "field").any(|a| match (a.args.get(1), a.args.get(2)) {
        (Some(Term::Str(f)), Some(Term::Var(x))) if x == v => field_kinds(f).is_some_and(|k| !k.is_empty() && k.iter().all(|k| leaf_kind(k))),
        _ => false,
    })
}

/// Whether `text(v, "s")` names a text within the bound: no long text equals it.
fn short_constant(text: &crate::code_rule_dsl::logic::front::Atom) -> bool {
    matches!(text.args.get(1), Some(crate::code_rule_dsl::logic::front::Term::Str(s)) if s.len() <= core_rr::TEXT_INLINE)
}

/// Rules reading `text(v, t)` of a node `v` whose kind no body fixes see `text` within its bound only
/// (`core_rr::TEXT_INLINE`): making every node's text at any length costs a run 40% more work (decision 0109). Each
/// such rule gets twins, one per non-empty set of those atoms, reading `t = @rr_node_text(v)` in their place: the
/// text of the nodes the rule binds, when past the bound (`long_node_text`). The rule and its twins together read every
/// text, each once. More than `MAX` such atoms in one rule keep the any-kind projection instead. Returns the twins made.
pub(crate) fn long_text_twins(program: &mut crate::code_rule_dsl::logic::front::Program, matchers: &[(String, String, Vec<String>)]) -> usize {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    const MAX: usize = 3;
    let (_, any) = text_kind_demand(program, matchers);
    let mut by_rule: std::collections::BTreeMap<usize, Vec<usize>> = Default::default();
    for (r, i) in any {
        by_rule.entry(r).or_default().push(i);
    }
    let mut twins = Vec::new();
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
                twin.body[i] = Literal::Cmp { l: t, op: "=", r: Term::Call("rr_node_text".into(), vec![v]) };
            }
            twins.push(twin);
        }
    }
    let made = twins.len();
    program.rules.extend(twins);
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
fn text_kind_demand(program: &crate::code_rule_dsl::logic::front::Program, matchers: &[(String, String, Vec<String>)]) -> (BTreeSet<String>, Vec<(usize, usize)>) {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
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
                let entry = columns.entry((rule.head.rel.clone(), c)).or_insert(Some(BTreeSet::new()));
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
    let (mut kinds, mut any) = (BTreeSet::new(), Vec::new());
    for (r, rule) in program.rules.iter().enumerate() {
        for (i, l) in rule.body.iter().enumerate() {
            let Literal::Atom(text) = l else { continue };
            if text.neg || text.rel != "text" {
                continue;
            }
            let Some(Term::Var(v)) = text.args.first() else { continue };
            match of_var(&columns, &rule.body, v) {
                Some(k) => kinds.extend(k),
                None if leaf_field(&rule.body, v) || short_constant(text) || twinned(&rule.head.rel, v) => {}
                None => any.push((r, i)),
            }
        }
    }
    (kinds, any)
}

/// Plan-time provenance, not a per-file scan. Only copying a span (including variable equality)
/// carries its origin. Regex captures have their own bounded ranges and do not demand the container text.
fn span_text_demand(program: &crate::code_rule_dsl::logic::front::Program) -> BTreeSet<crate::code_rule_dsl::logic::front::SpanIn> {
    use crate::code_rule_dsl::logic::front::{Literal, SpanIn, Term};
    let mut columns: HashMap<(String, usize), u8> = [( ("source".to_owned(), 1), 1), (("src_line".to_owned(), 2), 2)].into();
    let mut requested = 0;
    loop {
        let mut changed = false;
        for rule in &program.rules {
            let mut vars: HashMap<String, u8> = HashMap::new();
            for atom in rule.body.iter().filter_map(Literal::atom).filter(|atom| !atom.neg) {
                for (column, term) in atom.args.iter().enumerate() {
                    if let Term::Var(value) = term {
                        *vars.entry(value.clone()).or_default() |= columns.get(&(atom.rel.clone(), column)).copied().unwrap_or(0);
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
                        let entry = columns.entry((rule.head.rel.clone(), column)).or_default();
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
pub(crate) const GUEST_POLICY: &str = "guest_policy";

/// The first guest document's key: files take `1..GUEST_KEYS` (`corpus::memo::doc_keys`), guests in the run's
/// language `GUEST_KEYS..HIDDEN_KEYS`, and every other document (a guest in another grammar, a value) `HIDDEN_KEYS..2^31`
/// (decision 0058 §1: what a rule sees is a bound on these keys).
pub(crate) const GUEST_KEYS: u32 = 1 << 30;
pub(crate) const HIDDEN_KEYS: u32 = (1 << 30) + (1 << 29);

/// A guest document's place (decision 0055): the host document and the node or span it stands at, its grammar and
/// role, its root, and how its bytes map to the host's.
#[derive(Clone, Debug)]
pub(crate) struct Guest {
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
pub(crate) enum Made {
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
pub(crate) fn unparsed(d: &Document) -> &str {
    match d.guest.as_deref() {
        Some(g) if g.root != 0 => d.facts.text(g.root),
        _ => d.facts.source.as_str(),
    }
}

/// A document's tape: its content key, hex (`tape_of`).
pub(crate) fn tape_hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where a guest's bytes stand in its host's source.
#[derive(Clone, Debug)]
pub(crate) enum Placement {
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
pub(crate) struct Document {
    /// `d`: the document's key, its path atom in the push path.
    pub key: u32,
    /// The path relative to its root, from which `module` and `pkg` follow (T's `module`).
    pub path: String,
    /// The `#core:` name the facts were built for, and their digest (`core::digest`).
    pub core: String,
    pub digest: [u8; 32],
    pub facts: Arc<Facts>,
    /// A guest's place in its host (decision 0055); `None` for a file's own tree. A guest's `path` is its host's.
    pub guest: Option<Arc<Guest>>,
    /// The module a loaded document is (t21 E2: `load(path, module, …)`, the module its resolution gave); `None` for
    /// a file of the corpus, whose module follows from its path.
    pub module: Option<Arc<str>>,
    /// The tape the facts were read off, kept where the program runs matchers over it (decision 0106).
    pub(in crate::code_rule_dsl) tape: Option<crate::code_rule_dsl::tape::TapeDoc>,
}

/// A file's nodes as `core.rr`'s shape numbers them: each local id's core id and kind name, and the way from a node
/// id's low 32 bits to its local id (they differ when nodes are numbered per definition, `DefIds`). For making
/// guests from requests that name host nodes (decision 0055, `corpus::guests`).
pub(crate) struct Numbering {
    pub(crate) nodes: Vec<(u32, &'static str)>,
    /// `(low, local)`, ascending, under `DefIds`; `None` when the low bits are the local id.
    back: Option<Vec<(u32, u32)>>,
}

impl Numbering {
    /// The local id of the node whose id's low 32 bits are `low`.
    pub(crate) fn local(&self, low: u32) -> Option<u32> {
        match &self.back {
            None => Some(low),
            Some(b) => b.binary_search_by_key(&low, |&(g, _)| g).ok().map(|i| b[i].1),
        }
    }
    /// The node whose id's low 32 bits are `low`: its local id, core id and kind.
    pub(crate) fn at(&self, low: u32) -> Option<(u32, u32, &'static str)> {
        let l = self.local(low)?;
        self.nodes.get(l as usize).map(|&(id, k)| (l, id, k))
    }
}

pub(crate) fn numbering(doc: &Document) -> StoreResult<Numbering> {
    // A document read as text only has no nodes.
    if doc.facts.len() == 0 {
        return Ok(Numbering { nodes: Vec::new(), back: None });
    }
    let tag = crate::code_rule_dsl::facts::core::facts_tag(&doc.facts);
    let dialect = Dialect::kept(tag)?;
    // A guest's nodes are its root's subtree.
    let guest = doc.guest.as_deref().map(|g| (g, Arc::new(core_rr::Lines::new("", false))));
    let p = Projected::new(doc.key, Arc::clone(&doc.facts), &dialect, Shape::Core, false, guest);
    // A unit's own nodes are numbered before its tokens, so the nodes' ids need no tokens.
    let back = p.defs.map(|d| d.back);
    Ok(Numbering { nodes: p.core.iter().map(|&id| (id, dialect.name(&doc.facts, id))).collect(), back })
}

/// A path's module: `a/b.py` is `a.b`, `a/__init__.py` is `a`; a Rust file's is its path from
/// the crate root under `crate` (`src/lib.rs` and `src/main.rs` are `crate`, `src/a/mod.rs` and
/// `src/a.rs` are `crate.a`).
pub(crate) fn module(rel: &str) -> String {
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

pub(crate) fn package(rel: &str) -> String {
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
struct Dialect {
    tag: u8,
    family: Family,
    catalog: KindCatalog,
    classes: Vec<Class>,
    operator: Option<u16>,
    /// The `operators` field: a comparison's operator tokens (`a < b <= c`).
    operators: Option<u16>,
    canonical: Vec<u16>,
    codes: HashMap<u16, Vec<u16>>,
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

    fn new(tag: u8) -> StoreResult<Self> {
        let catalog = from_tape_tag(tag).ok_or_else(|| StoreError::Invalid(format!("no dialect has tape tag {tag}")))?.kinds();
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
        Ok(Self { tag, family, catalog, classes, operator: catalog.field_id("operator"), operators: catalog.field_id("operators"), canonical, codes })
    }
    fn code(f: &Facts, id: u32) -> u16 {
        crate::code_rule_dsl::facts::core::kind_code(f.kind[id as usize])
    }
    fn class(&self, f: &Facts, id: u32) -> Class {
        self.classes.get(usize::from(Self::code(f, id))).copied().unwrap_or(Class::Other)
    }
    fn name(&self, f: &Facts, id: u32) -> &'static str {
        self.catalog.kind_name(Self::code(f, id)).unwrap_or("")
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
struct Projected {
    module: Option<crate::code_rule_dsl::module_path::ModulePath>,
    python_imports: std::sync::OnceLock<crate::code_rule_dsl::facts::producers::python::import_screen::Imports>,
    python_semantics: std::sync::Mutex<Option<python_demand::Semantics>>,
    key: u32,
    tag: u8,
    facts: Arc<Facts>,
    /// The core id of each node.
    core: Vec<u32>,
    /// The node of each core id, `NONE` for one left out.
    local: Vec<u32>,
    idx: OnceLock<Vec<u32>>,
    size: OnceLock<Vec<u32>>,
    depth: OnceLock<Vec<u32>>,
    /// The nodes hanging from their parent under a field, ascending.
    fielded: OnceLock<Vec<u32>>,
    /// `core.rr`'s lines and spans; `None` in T's shape.
    extra: Option<Box<Extra>>,
    /// A demanded semantic visit column, shared by every consumer in this document batch.
    python_visit: std::sync::OnceLock<Vec<u32>>,
    /// Node ids numbered per definition (`DefIds`), when the run asks for them; else a node's id is its preorder.
    defs: Option<DefIds>,
}

/// Node ids per definition (decisions 0083 §3, 0094 §2; `granules`): a node's id is `document << 32 | slot << 18 |
/// index`, its granule's slot and its index there. An edit renumbers the granules it touches and no other's.
/// Spans that are positions (a document's text, its lines, matches) are in the top slots, in order.
struct DefIds {
    /// The low 32 bits of each node's and token's id, by local id.
    low: Vec<u32>,
    /// `(low, local)`, ascending: the way back.
    back: Vec<(u32, u32)>,
}

use granules::INDEX_BITS;

/// Whether this process numbers nodes per definition: the default; `RERAIL_NODE_IDS=preorder` numbers them in
/// preorder, to compare.
pub(crate) fn definition_ids() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| !std::env::var("RERAIL_NODE_IDS").is_ok_and(|v| v == "preorder"));
    *ON
}

/// How an output shows a node id's low 32 bits: its preorder index, or by definition `slot.index` (`DefIds`). A
/// memoised document is never parsed in a run, so the way back to preorder is not kept: the form by definition is
/// the stable one, and a finding keyed by it survives edits to other definitions.
pub(crate) fn shown_local(low: i64) -> String {
    if definition_ids() {
        format!("{}.{}", low >> INDEX_BITS, low & ((1 << INDEX_BITS) - 1))
    } else {
        low.to_string()
    }
}

impl DefIds {
    /// The ids of `p`'s nodes, and of `tokens` (each token's node, in order), by definition.
    fn new(p: &Projected, dialect: &Dialect, tokens: &[u32]) -> Self {
        let low = granules::number(p, dialect, tokens);
        let mut back: Vec<(u32, u32)> = low.iter().enumerate().map(|(l, &g)| (g, l as u32)).collect();
        back.sort_unstable();
        DefIds { low, back }
    }
}

/// A document's lines, and its spans past its nodes: from local id `len()`, its anonymous tokens (`tokens` of them,
/// in preorder), then its whole text, then each line.
struct Extra {
    lines: Arc<core_rr::Lines>,
    token_spans: OnceLock<Vec<[u32; 2]>>,
    tokens: u32,
    /// Source and line spans are implicit. Only regex captures need owned ranges.
    own_text: bool,
    matches: Vec<[u32; 2]>,
    /// A guest's host lines and placement: its places are the host's (decision 0055). A guest has no source-text
    /// spans of its own (`source`, `src_line`): those are its file's.
    host: Option<(Arc<core_rr::Lines>, Placement)>,
}

impl Extra {
    fn matched(&self) -> u32 { self.tokens + if self.own_text { 1 + self.lines.count() as u32 } else { 0 } }
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
    /// A `withheld` document (`core.rr`: one that does not parse) has no nodes and no spans: no facts. A guest's
    /// nodes are its root's subtree, placed in its host's lines (`host`).
    fn new(key: u32, facts: Arc<Facts>, dialect: &Dialect, shape: Shape, withheld: bool, guest: Option<(&Guest, Arc<core_rr::Lines>)>) -> Self {
        let identity_span = crate::code_rule_dsl::logic::profile::span(|| "identity maps".into());
        let f = &*facts;
        let mut p = Projected {
            module:None, python_imports:Default::default(), python_semantics:Default::default(),
            key,
            tag: dialect.tag,
            core: Vec::new(),
            local: vec![NONE; f.len()],
            idx: OnceLock::new(),
            size: OnceLock::new(),
            depth: OnceLock::new(),
            fielded: OnceLock::new(),
            extra: None,
            python_visit: Default::default(),
            defs: None,
            facts: Arc::clone(&facts),
        };
        let mut tokens = 0;
        let (first, last) = match guest {
            Some((g, _)) => (g.root, f.subtree_end.get(g.root as usize).copied().unwrap_or(g.root + 1).max(g.root + 1)),
            None => (0, f.len() as u32),
        };
        if !withheld {
            let count = (first..last.min(f.len() as u32)).filter(|&id| f.named(id) && (shape != Shape::T || dialect.class(f, id) != Class::Comment)).count();
            p.core.reserve_exact(count);
        }
        for id in first..if withheld { first } else { last.min(f.len() as u32) } {
            if !f.named(id) {
                if shape == Shape::Core && f.end[id as usize] > f.start[id as usize]
                    && f.parent(id).is_some_and(|q| p.local[q as usize] != NONE) { tokens += 1; }
                continue;
            }
            if shape == Shape::T && dialect.class(f, id) == Class::Comment { continue; }
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
            let lines = Arc::new(core_rr::Lines::shared(Arc::clone(&f.source), dialect.family == Family::Python));
            if definition_ids() {
                let mut owners = Vec::with_capacity(tokens as usize);
                owners.extend(p.core.iter().enumerate().flat_map(|(l, &id)| f.children(id).iter().filter(|&&c| !f.named(c) && f.end[c as usize] > f.start[c as usize]).map(move |_| l as u32)));
                p.defs = Some(DefIds::new(&p, dialect, &owners));
            }
            let own_text = guest.is_none();
            let host = guest.filter(|(g, _)| g.made == Made::Host).map(|(g, lines)| (lines, g.in_file.clone()));
            p.extra = Some(Box::new(Extra { lines, tokens, token_spans: OnceLock::new(), matches: Vec::new(), own_text, host }));
        } else if definition_ids() {
            // T's shape has no token spans: its nodes alone are numbered per definition.
            p.defs = Some(DefIds::new(&p, dialect, &[]));
        }
        p
    }
    #[inline]
    fn parent(&self, local: usize) -> u32 {
        self.facts.parent(self.core[local]).map_or(NONE, |p| self.local[p as usize])
    }
    #[inline]
    fn idx(&self, local: usize) -> u32 { self.ordinals()[local] }
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
    fn size(&self, local: usize) -> u32 { self.sizes()[local] }
    #[inline]
    fn sizes(&self) -> &[u32] {
        self.size.get_or_init(|| {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "projection subtree sizes".into());
            let mut out = vec![0; self.core.len()];
            for l in (1..out.len()).rev() { let p = self.parent(l) as usize; out[p] += out[l] + 1; }
            out
        })
    }
    #[inline]
    fn depth(&self, local: usize) -> u32 { self.depths()[local] }
    #[inline]
    fn depths(&self) -> &[u32] {
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
    fn locs(&self) -> u32 {
        self.len() + self.extra.as_ref().map_or(0, |x| x.matched() + x.matches.len() as u32)
    }
    /// The byte span of node or span `local`.
    fn span(&self, local: u32) -> [u32; 2] {
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
                } else if i < x.matched() {
                    if i == x.tokens { [0, self.facts.source.len() as u32] }
                    else { x.lines.span((i - x.tokens) as usize) }
                } else { x.matches[(i - x.matched()) as usize] }
            }),
        }
    }
    fn len(&self) -> u32 {
        self.core.len() as u32
    }
    fn global(&self, local: u32) -> i64 {
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
    /// The node or span whose id's low 32 bits are `low` (`global`'s inverse).
    fn local_of(&self, low: u32) -> Option<u32> {
        match &self.defs {
            Some(d) if granules::span_of(low).is_some() => granules::span_of(low).map(|i| d.low.len() as u32 + i),
            Some(d) => d.back.binary_search_by_key(&low, |&(g, _)| g).ok().map(|i| d.back[i].1),
            None => Some(low),
        }
    }
    /// The included children of `local`, in order.
    fn children(&self, local: u32) -> impl Iterator<Item = u32> + '_ {
        self.facts.children(self.core[local as usize]).iter().map(|&c| self.local[c as usize]).filter(|&c| c != NONE)
    }
}

/// One wave's documents, sorted by key, and the row offsets of the views.
pub(crate) struct Batch {
    docs: Vec<Projected>,
    dialects: HashMap<u8, Dialect>,
    nodes: Vec<usize>,
    children: Vec<usize>,
    fields: Vec<usize>,
    /// Nodes and spans (`Projected::locs`).
    locs: OnceLock<Vec<usize>>,
    shape: Shape,
    /// The `py.*` families of the batch's Python documents, made on their first read (list step 25).
    py: Option<Py>,
}

/// The `py.*` families (`facts::producers::python::families`): the producer's grammar, the atoms of its
/// names, and the rows, made on the first read of any of them: a program that reads none does no work.
struct Py {
    grammar: py::Grammar,
    contexts: [i64; 5],
    spreads: [i64; 3],
    rows: std::sync::OnceLock<Vec<Relation>>,
    argument_rows: std::sync::OnceLock<Relation>,
}

/// The `py.*` families' tables and columns (all integers; `-1` is none), as `PY_COLUMNS[i]` is `PY[i]`'s.
pub(crate) const PY: [&str; 5] = ["py_call", "py_call_site", "py_exec_context", "py_call_arg", "py_attribute"];
pub(crate) const PY_COLUMNS: [&[&str]; 5] = [
    &["d", "call", "callee"],
    &["d", "report", "call", "ref"],
    &["d", "call", "owner", "kind"],
    &["d", "call", "arg", "ordinal", "keyword", "spread", "value"],
    &["d", "node", "object", "name"],
];

/// Which view a source serves.
#[derive(Clone, Copy)]
enum View {
    Node,
    /// `line(n, l)`: a node's first line, 1-based (rows laid out as `node`'s).
    Line,
    Child,
    Field,
    Tape,
    /// `core_node(d, t, n)`: document `d`'s tape node `t` (the matcher's preorder id over every
    /// node) is node `n`. The wave's matches are tape nodes; this joins them to the rows above.
    Core,
    /// `visit_index(n, i)`: semantic field order in core Python, preorder elsewhere.
    Visit,
    /// `ast_depth(n, k)`: the node's depth, the root's 0.
    Depth,
    /// `node_order(n, k)`: the node's preorder key, `document << 32 | its preorder index`: the order node ids had
    /// before they were numbered per definition (`granules`). Rules that order nodes read it (`rr::lower`).
    Order,
    /// `core.rr`'s place of a node or span (`loc`): `line(n, l)`, `end_line(n, l)`, `col(n, c)`, `docof(n, d)`,
    /// read from the document's lines (`core_rr::Lines`), rows laid out over `Projected::locs`.
    Loc(Place),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Line,
    EndLine,
    Col,
    #[allow(dead_code, reason = "bound once the lowering reads `docof` as an input, not as `node`'s projection")]
    Docof,
}

/// Every batch built in a test, by its documents' keys, held weakly: a run releases its batches when it returns
/// (`logic::rr::tests`), so a chunk's facts are not kept by a binding past it.
#[cfg(test)]
pub(crate) static BATCHES: parking_lot::Mutex<Vec<(Vec<u32>, std::sync::Weak<Batch>)>> = parking_lot::Mutex::new(Vec::new());

impl Batch {
    /// Projects each document and interns the batch's strings; the documents' keys are distinct.
    /// Returns the batch and its small relations (`doc`, `pkg`, `syntax_error`, `text`, `intval`, `op`).
    /// Also returns the batch's distinct strings and their symbols: its vocabulary, which text tests read
    /// (`logic::strings::Vocabulary`, decision 0037).
    #[allow(clippy::type_complexity)]
    fn build(store: &ContentStore, documents: Vec<Document>, projection: &Projection, metal: bool) -> StoreResult<(Arc<Self>, Vec<(&'static str, Relation)>, (Vec<String>, Vec<u32>))> {
        let shape = projection.shape;
        let core = shape == Shape::Core;
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
        let (mut doc, mut pkg, mut text, mut intval, mut op_t) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        // `path_part(d, s)`: each directory of a document's path and its file name; `file_name(d, s)`: the last.
        let (mut path_part, mut file_name): (Vec<[i64; 2]>, Vec<[i64; 2]>) = (Vec::new(), Vec::new());
        // Text relations (decision 0037): `name(n, s)` (an identifier's text, an attribute's dotted text),
        // `str(n, s)` (a string literal's content, between its quotes) and `strprefix(n, p)` (its prefix, lowercased).
        let (mut name, mut strs, mut strprefix) = (Vec::new(), Vec::new(), Vec::new());
        // `value(n, v)`: a literal's static value (`core.rr`): a Python string decoded (`value::python::str_value`),
        // an integer in decimal.
        let mut value: Vec<[i64; 2]> = Vec::new();
        // `op_at(n, i, s)`: n's operator tokens in order, from 0 (`core.rr`): in `a < b <= c`, (c, 0, "<") and
        // (c, 1, "<="). T's: the tokens of the `operator` and `operators` fields; `core.rr`'s: every anonymous child
        // but punctuation.
        let mut op_at: Vec<[i64; 3]> = Vec::new();
        let python_tag = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        // `syntax_error(d)`: the documents whose parse has an error (`Facts::syntax_errors`); `core.rr`'s
        // `syntax_error(d, l, msg)`: the first error's line and message.
        let mut errors = Vec::new();
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
        };
        c.long_text = projection.long_text.clone();
        c.long_text_any = projection.long_text_any;
        let wants_text = core && projection.wants("text");
        let wants_source_text = core && projection.wants_span_text(crate::code_rule_dsl::logic::front::SpanIn::Source);
        let wants_line_text = core && projection.wants_span_text(crate::code_rule_dsl::logic::front::SpanIn::Line);
        let wants_split_lines = core && projection.wants("split_line");
        // Asked once for the batch, not per node (a lookup in the program's set of read relations).
        let (wants_parts, wants_part_starts, wants_part_offsets, wants_words) = (projection.wants("str_part"), projection.wants("str_part_start"), projection.wants("str_part_offset"), projection.wants("str_part_word"));
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
                    use crate::code_rule_dsl::logic::front::SpanIn;
                    let on = std::env::var("RERAIL_SPAN_PREFILTER").map_or(true, |v| v != "off");
                    let prefilter = match within {
                        SpanIn::Source | SpanIn::Line if on => crate::code_rule_dsl::transducer::packed::regex_block::SpanPrefilter::compile(re, *within == SpanIn::Line).map(Arc::new),
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
            use crate::code_rule_dsl::transducer::packed::regex_block::{RanOn, SpanPrefilter, ends_launch};
            let filters: Vec<(usize, &Arc<SpanPrefilter>)> = rx_spans.iter().enumerate().filter_map(|(i, (_, _, _, f))| f.as_ref().map(|f| (i, f))).collect();
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
                let programs: Vec<Arc<SpanPrefilter>> = filters.iter().map(|(_, f)| Arc::clone(f)).collect();
                let (ends, ran) = ends_launch(crate::code_rule_dsl::logic::lanes::executor_for(metal), &programs, &sources, &files, metal).map_err(StoreError::Invalid)?;
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
        // A document of no nodes (one read as text only, `core::text_only`) has no tag of its own: it is read in the
        // batch's dialect, its rows (`doc`, `source`, `src_line`) the same in any.
        let batch_tag = documents.iter().map(|d| crate::code_rule_dsl::facts::core::facts_tag(&d.facts)).find(|&t| t != 0)
            .or_else(|| crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python))
            .unwrap_or(0);
        for (j, d) in documents.iter().enumerate() {
            let tag = match crate::code_rule_dsl::facts::core::facts_tag(&d.facts) {
                0 if d.facts.len() == 0 => batch_tag,
                t => t,
            };
            if !dialects.contains_key(&tag) {
                dialects.insert(tag, Dialect::new(tag)?);
            }
            let dialect = &dialects[&tag];
            // `core.rr`: a document that does not parse is withheld from the policy (no facts) but its `doc`, path
            // and `syntax_error` rows; its outcome is Unknown(parse). T's shape keeps every document.
            // A guest that does not parse is withheld too, unless the policy keeps its partial tree (decision 0055 §6).
            let guest = d.guest.as_deref().filter(|_| core);
            let error = if core && (projection.wants("syntax_error") || projection.wants("syntax_exception") || guest.is_some()) { core_rr::verdict(&d.facts, dialect) } else { None };
            let withheld = error.is_some() && !guest.is_some_and(|g| projection.keeps_partial(&g.grammar));
            let placed = guest.map(|g| (g, lines_of.get(&file_of(g.host)).cloned().unwrap_or_else(|| Arc::new(core_rr::Lines::shared(Arc::clone(&d.facts.source), dialect.family == Family::Python)))));
            let new_span = crate::code_rule_dsl::logic::profile::span(|| "projected".into());
            let mut p = Projected::new(d.key, Arc::clone(&d.facts), dialect, shape, withheld, placed);
            drop(new_span);
            p.module=crate::code_rule_dsl::module_path::ModulePath::dotted(d.module.as_deref().unwrap_or(&module(&d.path)));
            if !rx_spans.is_empty() && guest.is_none() && !withheld {
                c.matches(&mut p, dialect, &rx_spans, &span_ends[j], &mut string);
            }
            if core && !projection.ast_patterns.is_empty() && guest.is_none() && !withheld
                && let Some(lang) = crate::code_rule_dsl::language::from_tape_tag(tag)
            {
                let compiled = ast_patterns.entry(tag).or_insert_with(|| {
                    projection
                        .ast_patterns
                        .iter()
                        .filter_map(|t| crate::code_rule_dsl::language::rule_pattern(crate::code_rule_dsl::language::PatternSource::Plain(t), lang).ok().map(|p| (t.as_str(), p)))
                        .collect()
                });
                c.patterns(&p, dialect, lang, compiled, &mut string);
            }
            if core && guest.is_none() && let Some(x) = &p.extra {
                lines_of.insert(d.key, Arc::clone(&x.lines));
            }
            withheld_at.push(withheld);
            let key = i64::from(d.key);
            // core.rr: a module for Python and Rust files, "" for the others; `pkg` for Python only. T's has both
            // for every file.
            let python = d.path.ends_with(".py") || d.path.ends_with(".pyi");
            let dotted = python || d.path.ends_with(".rs");
            let named = d.module.as_deref();
            let module = match named {
                Some(m) => m.to_owned(),
                None if shape == Shape::Core && !dotted => String::new(),
                None => module(&d.path),
            };
            doc.push([key, string(&d.path) as i64, string(&module) as i64]);
            if shape != Shape::Core || python {
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
            let f = &*d.facts;
            let nodes_span = crate::code_rule_dsl::logic::profile::span(|| "node rows".into());
            for (local, &id) in p.core.iter().take(if scalar_nodes {p.core.len()}else{0}).enumerate() {
                let n = p.global(local as u32);
                if !core {
                    let is_operator = |c: &&u32| {
                        let field = f.field[**c as usize];
                        field != 0 && (dialect.operator == Some(field) || dialect.operators == Some(field))
                    };
                    for (i, &c) in f.children(id).iter().filter(is_operator).enumerate() {
                        op_at.push([n, i as i64, string(dialect.name(f, c)) as i64]);
                    }
                }
                match dialect.class(f, id) {
                    Class::Text if !core => {
                        let t = f.text(id);
                        if !t.contains(['\t', '\n', '\r', '\\']) {
                            text.push([n, string(t) as i64]);
                            name.push([n, string(t) as i64]);
                        }
                    }
                    Class::Attribute if !core => {
                        if let Some(dotted) = dialect.dotted(f, id) {
                            name.push([n, string(&dotted) as i64]);
                        }
                    }
                    Class::String if !core => {
                        if let Some((prefix, content)) = dialect.string(f, id) {
                            strprefix.push([n, string(&prefix) as i64]);
                            if let Some(content) = content {
                                strs.push([n, string(content) as i64]);
                            }
                            if Some(tag) == python_tag
                                && let Some(v) = crate::code_rule_dsl::value::python::str_value(f.text(id))
                            {
                                value.push([n, string(&v) as i64]);
                            }
                        }
                    }
                    Class::Integer => {
                        if let Ok(v) = f.text(id).replace('_', "").parse::<i64>() {
                            intval.push([n, v]);
                            if !core {
                                value.push([n, string(&v.to_string()) as i64]);
                            }
                        }
                    }
                    Class::Operator => {
                        let operator = dialect.operator.filter(|&o| o != 0);
                        if let Some(&o) = f.children(id).iter().find(|&&c| operator == Some(f.field[c as usize])) {
                            op_t.push([n, string(dialect.name(f, o)) as i64]);
                        }
                    }
                    // `async def`: the keyword is an anonymous child, recorded as the definition's modifier.
                    // `async for`, `async with` likewise.
                    Class::FunctionDefinition | Class::AsyncStatement if f.children(id).first().is_some_and(|&k| dialect.name(f, k) == "async") => {
                        op_t.push([n, string("async") as i64]);
                    }
                    Class::FunctionItem
                        if f.children(id).iter().any(|&m| {
                            dialect.name(f, m) == "function_modifiers" && f.children(m).iter().any(|&k| dialect.name(f, k) == "async")
                        }) =>
                    {
                        op_t.push([n, string("async") as i64]);
                    }
                    _ => {}
                }
                if core {
                    c.node(&p, dialect, local as u32, &mut string, wants_text, wants_parts, wants_part_starts, wants_part_offsets, wants_words);
                }
            }
            drop(nodes_span);
            let document_span = crate::code_rule_dsl::logic::profile::span(|| "document rows".into());
            if let Some((line, msg)) = &error {
                let line = p.extra.as_ref().map_or(*line, |x| x.file_line(*line));
                c.withheld(&p, &(line, msg.clone()), &mut string);
                if projection.wants("syntax_exception") && guest.is_none() && dialect.family==Family::Python {
                    c.exception(&p,&mut string);
                }
            }
            if core && (!withheld || guest.is_none()) {
                c.document(&p, dialect, &mut string, wants_text, wants_tokens && !withheld, wants_split_lines, wants_lf_lines, wants_source_text, wants_line_text, guest.is_none(), projection);
                if wants_digests && !withheld {
                    c.digests(&p, dialect, &mut string, wants_trees, wants_units);
                }
                if !withheld && dialect.family == Family::Python
                    && projection.reads.as_ref().is_some_and(|r| ["annotation_text", "annotation_constant_repr", "call_receiver_text", "call_text", "call_function_text", "decorator_text", "class_base_text"].iter().any(|name| r.contains(*name)))
                {
                    let requested = python_print::Requested {
                        annotations: projection.wants("annotation_text"), receivers: projection.wants("call_receiver_text"),
                        annotation_constants: projection.wants("annotation_constant_repr"),
                        calls: projection.wants("call_text"), functions: projection.wants("call_function_text"),
                        decorators: projection.wants("decorator_text"), bases: projection.wants("class_base_text"),
                    };
                    c.printed_expressions(&p, dialect, &mut string, requested).map_err(StoreError::Invalid)?;
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
        let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        let vocabulary = python.filter(|tag| dialects.contains_key(tag)).map(|_| {
            (py::CONTEXTS.map(|name| string(name) as i64), py::SPREADS.map(|name| string(name) as i64))
        });
        crate::code_rule_dsl::logic::profile::decision("input payload", || {
            let identities: usize = docs.iter().map(|p| (p.core.capacity() + p.local.capacity() + [&p.idx, &p.size, &p.depth, &p.fielded].iter().map(|v| v.get().map_or(0, |v| v.capacity())).sum::<usize>()) * 4).sum();
            let extras: usize = docs.iter().filter_map(|p| p.extra.as_ref()).map(|x| (x.matches.capacity() + x.token_spans.get().map_or(0, |v| v.capacity())) * 8).sum();
            serde_json::json!({"documents": docs.len(), "coreColumnBytes": docs.iter().map(|p| p.facts.bytes()).sum::<usize>(), "identityCapacityBytes": identities, "extraSpanCapacityBytes": extras}).to_string()
        });
        crate::code_rule_dsl::logic::profile::decision("projection demand",||serde_json::json!({"scalarNodes":if scalar_nodes {docs.iter().map(|p|p.core.len()).sum::<usize>()}else{0},"nodes":docs.iter().map(|p|p.core.len()).sum::<usize>(),"strings":strings.len(),"reads":projection.reads}).to_string());
        let intern = crate::code_rule_dsl::logic::profile::span(|| "intern atoms".into());
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, strings.len() as u64);
        let atoms = store.intern_atoms(&strings.iter().map(String::as_str).collect::<Vec<_>>())?;
        drop(intern);
        let atom = |i: i64| i64::from(atoms[i as usize]);
        let py = vocabulary.zip(python).map(|((contexts, spreads), tag)| Py {
            grammar: py::Grammar::new(tag, &dialects[&tag].catalog),
            contexts: contexts.map(atom),
            spreads: spreads.map(atom),
            argument_rows: Default::default(),
            rows: Default::default(),
        });
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
            ("doc", relation(&["d", "path", "module"], &mut doc.iter().map(|&[d, p, m]| vec![d, atom(p), atom(m)]))),
            ("pkg", pairs(&["d", "p"], &pkg, [false, true])),
            ("path_part", pairs(&["d", "s"], &path_part, [false, true])),
            ("file_name", pairs(&["d", "s"], &file_name, [false, true])),
            ("intval", pairs(&["n", "v"], &intval, [false, false])),
            ("op_t", pairs(&["n", "s"], &op_t, [false, true])),
        ];
        relations.extend(g.relations(&atom));
        if core {
            relations.extend(c.relations(&atom));
        } else {
            relations.extend([
                ("value", pairs(&["n", "s"], &value, [false, true])),
                ("text", pairs(&["n", "s"], &text, [false, true])),
                ("op", pairs(&["n", "s"], &op_t, [false, true])),
                ("op_at", relation(&["n", "i", "s"], &mut op_at.iter().map(|&[n, i, s]| vec![n, i, atom(s)]))),
                ("name", pairs(&["n", "s"], &name, [false, true])),
                ("str", pairs(&["n", "s"], &strs, [false, true])),
                ("strprefix", pairs(&["n", "s"], &strprefix, [false, true])),
                ("syntax_error", relation(&["d"], &mut errors.iter().map(|&d| vec![d]))),
            ]);
        }
        let offsets = |count: &dyn Fn(&Projected) -> usize| {
            let mut v = vec![0];
            docs.iter().for_each(|p| v.push(v.last().unwrap() + count(p)));
            v
        };
        let nodes = offsets(&|p| p.core.len());
        let children = offsets(&|p| p.core.len().saturating_sub(1));
        let fields = offsets(&|p| if projection.wants("field") { p.fielded().len() } else { 0 });
        let locs = OnceLock::new();
        let batch = Arc::new(Batch { docs, dialects, nodes, children, fields, locs, shape, py });
        hold_texts(&batch);
        #[cfg(test)]
        BATCHES.lock().push((batch.docs.iter().map(|p| p.key).collect(), Arc::downgrade(&batch)));
        Ok((batch, relations, (strings, atoms)))
    }

    fn offsets(&self, view: View) -> &[usize] {
        match view {
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth | View::Order => &self.nodes,
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
    fn locate(&self, view: View, row: usize) -> (&Projected, usize) {
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
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth | View::Order | View::Loc(_) => Some(local as usize),
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
            View::Visit => &["n", "i"],
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
            View::Line | View::Visit | View::Depth | View::Order | View::Loc(_) => 0,
            View::Field | View::Core => 2,
            View::Tape => 0,
        }
    }
    /// The column holding the row's parent, looked up through the children lists.
    fn parent(&self) -> Option<usize> {
        match self.view {
            View::Child | View::Field => Some(0),
            View::Tape => Some(1),
            View::Node | View::Line | View::Core | View::Visit | View::Depth | View::Order | View::Loc(_) => None,
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
                // A preorder key: its document and its local id.
                let local = (v & 0xffff_ffff) as u32;
                rows.extend(b.doc(v >> 32).filter(|&doc| v >= 0 && local < b.docs[doc].len()).and_then(|doc| b.row(self.view, doc, local)));
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
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth | View::Order | View::Loc(_) => i as u32,
            View::Child => i as u32 + 1,
            View::Field => p.fielded()[i],
        };
        let l = local as usize;
        let id = p.core[l] as usize;
        let parent = || if p.parent(l) == NONE { -1 } else { p.global(p.parent(l)) };
        match (self.view, column) {
            (View::Node | View::Core, 0) => i64::from(p.key),
            (View::Core, 1) => id as i64,
            (View::Node, 2) => {
                let code = crate::code_rule_dsl::facts::core::kind_code(p.facts.kind[id]);
                i64::from(kind_atom(p.tag, self.batch.dialects[&p.tag].canonical.get(usize::from(code)).copied().unwrap_or(code)))
            }
            (View::Node, 3) => i64::from(p.facts.start[id]),
            (View::Node, 4) => i64::from(p.facts.end[id]),
            (View::Line, 1) => i64::from(p.facts.start_line(id as u32)),
            (View::Child | View::Field, 0) | (View::Tape, 1) => parent(),
            (View::Child, 2) => i64::from(p.idx(l)),
            (View::Field, 1) => i64::from(field_atom(p.tag, p.facts.field[id])),
            (View::Tape, 2) => i64::from(p.size(l)),
            (View::Tape, 3) => i64::from(p.depth(l)),
            (View::Visit, 1) => {
                let dialect = &self.batch.dialects[&p.tag];
                if self.batch.shape == Shape::Core && dialect.family == Family::Python {
                    i64::from(p.python_visit.get_or_init(|| python_order::ranks(p, dialect))[local as usize])
                } else {
                    i64::from(local)
                }
            },
            (View::Depth, 1) => i64::from(p.depth(l)),
            (View::Order, 1) => i64::from(p.key) << 32 | i64::from(local),
            _ => p.global(local),
        }
    }
}

/// The field atom of field `code` in the dialect whose tape tag is `tag`.
pub(crate) fn field_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) << 16 | u32::from(code)
}

/// T's nine base relations over `documents`, and the `py.*` families, by table name.
pub(crate) fn sources(store: &ContentStore, documents: Vec<Document>) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    shaped(store, documents, &Projection::default())
}

/// The base relations over `documents` in `projection`'s shape, and the `py.*` families, by table name.
pub(crate) fn shaped(store: &ContentStore, documents: Vec<Document>, projection: &Projection) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    let (batch, relations, _) = Batch::build(store, documents, projection, false)?;
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
            ("visit_index", View::Visit),
            ("ast_depth", View::Depth),
        ]),
    }
    for (name, view) in named {
        out.push((name, Arc::new(Viewed { batch: Arc::clone(batch), view })));
    }
    for (family, name) in PY.into_iter().enumerate() {
        out.push((name, Arc::new(PyFamily { batch: Arc::clone(batch), family })));
    }
    out
}

impl Batch {
    /// The `py.*` families' relations (empty without Python documents), made on the first call.
    fn py(&self) -> &[Relation] {
        static EMPTY: std::sync::OnceLock<Vec<Relation>> = std::sync::OnceLock::new();
        let Some(py) = &self.py else {
            return EMPTY.get_or_init(|| PY_COLUMNS.iter().map(|columns| Relation::new(columns)).collect());
        };
        py.rows.get_or_init(|| {
            let started = std::time::Instant::now();
            let mut out: Vec<Relation> = PY_COLUMNS.iter().map(|columns| Relation::new(columns)).collect();
            let mut skipped = 0;
            for p in self.docs.iter().filter(|p| p.tag == py.grammar.tag()) {
                let rows = py::structure(&py.grammar, &p.facts);
                let d = i64::from(p.key);
                // A node id, `-1` for none; `None` for a node the projection leaves out.
                let id = |core: u32| -> Option<i64> {
                    if core == NONE {
                        return Some(-1);
                    }
                    let local = p.local[core as usize];
                    (local != NONE).then(|| p.global(local))
                };
                let mut push = |family: usize, row: Option<Vec<i64>>| match row {
                    Some(row) => out[family].push(&row),
                    None => skipped += 1,
                };
                for &[call, callee] in &rows.call {
                    push(0, (|| Some(vec![d, id(call)?, id(callee)?]))());
                }
                for &[report, call, reference] in &rows.site {
                    push(1, (|| Some(vec![d, id(report)?, id(call)?, id(reference)?]))());
                }
                for &[call, owner, kind] in &rows.context {
                    push(2, (|| Some(vec![d, id(call)?, id(owner)?, py.contexts[kind as usize]]))());
                }
                for &[call, arg, ordinal, keyword, spread, value] in &rows.arg {
                    push(3, (|| Some(vec![d, id(call)?, id(arg)?, i64::from(ordinal), id(keyword)?, py.spreads[spread as usize], id(value)?]))());
                }
                for &[node, object, name] in &rows.attribute {
                    push(4, (|| Some(vec![d, id(node)?, id(object)?, id(name)?]))());
                }
            }
            crate::code_rule_dsl::prof::count("py.rows", out.iter().map(|r| r.len()).sum());
            crate::code_rule_dsl::prof::count("py.skipped", skipped);
            crate::code_rule_dsl::prof::count("py.us", started.elapsed().as_micros() as usize);
            out
        })
    }
}

/// One `py.*` family of a batch (`PY[family]`), served from the rows its first read makes.
struct PyFamily {
    batch: Arc<Batch>,
    family: usize,
}
impl PyFamily {
    fn rows(&self) -> &Relation {
        if self.family!=3 {return &self.batch.py()[self.family];}
        let Some(py)=&self.batch.py else {return &self.batch.py()[3];};
        py.argument_rows.get_or_init(|| {
            let mut out=Relation::new(PY_COLUMNS[3]);
            for p in self.batch.docs.iter().filter(|p|p.tag==py.grammar.tag()) {
                let rows=py::rows(&py.grammar,&p.facts);
                let id=|n:u32| if n==NONE {Some(-1)}else{p.local.get(n as usize).copied().filter(|&l|l!=NONE).map(|l|p.global(l))};
                for [c,a,i,k,s,v] in rows.arg {
                    if let (Some(c),Some(a),Some(k),Some(v))=(id(c),id(a),id(k),id(v)) {out.push(&[p.key as i64,c,a,i as i64,k,py.spreads[s as usize],v]);}
                }
            }
            out
        })
    }
}
impl Source for PyFamily {
    fn columns(&self) -> Vec<String> {
        PY_COLUMNS[self.family].iter().map(|c| (*c).to_owned()).collect()
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

impl Batch {
    /// The node id of row `row` of the node view.
    fn id(&self, row: usize) -> i64 {
        let (p, local) = self.locate(View::Node, row);
        p.global(local as u32)
    }
}

/// T's base relations over `documents` (`sources`), each of `tests`' relations, and each of `sets` (a relation and its set's kind atoms):
/// `rr_nearest_<set>(n, a)`, the nearest ancestor in the set, `rr_inside_<set>(n, a)`, every one (decision 0036), or
/// `rr_has_<set>(n, a)`, n in the set and a each of its strict ancestors. Each is made per looked-up node on the host
/// (`within::Axis`), or for the whole batch by the transducer's file-local FIX (`typing::file_local::nearest`, bound
/// `binding`) where the planner expects more lookups than that costs (decision 0105).
pub(crate) fn with_nearest(
    store: &ContentStore,
    documents: Vec<Document>,
    sets: &[(String, Vec<i64>)],
    tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    projection: &Projection,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    with_matchers(store, documents, sets, &Matchers::default(), tests, binding, projection)
}

/// A program's `ast` matcher (decision 0106): the relation of its matched nodes, `rr_ast_<k>(n)`, its matcher lowered
/// for the transducer, and the conditions on a document's path and text it runs under.
pub(crate) struct Matcher {
    pub(crate) rel: String,
    pub(in crate::code_rule_dsl) lowered: Arc<crate::code_rule_dsl::transducer::packed::LoweredMatcher>,
    pub(in crate::code_rule_dsl) gates: Vec<crate::code_rule_dsl::matcher::DocumentGate>,
    /// The metavariables the atom captures, in its relation's column order after `n` (decision 0107).
    pub(crate) captures: Vec<String>,
    pub(in crate::code_rule_dsl) captor: Arc<crate::code_rule_dsl::waves::policies::Captor>,
}

/// A program's matchers (decision 0106) and the wave they run as: every matcher's labels and roots in one run per
/// chunk (decision 0107).
#[derive(Default)]
pub(crate) struct Matchers {
    pub(crate) list: Vec<Matcher>,
    pub(in crate::code_rule_dsl) wave: Option<Arc<crate::code_rule_dsl::transducer::packed::LoweredWave>>,
}

impl Matchers {
    /// `list` as one wave (labels shared by matchers are one pass).
    pub(in crate::code_rule_dsl) fn new(list: Vec<Matcher>) -> Result<Self, String> {
        let wave = if list.is_empty() {
            None
        } else {
            let clauses: Vec<&crate::code_rule_dsl::transducer::packed::LoweredMatcher> = list.iter().map(|m| m.lowered.as_ref()).collect();
            Some(Arc::new(crate::code_rule_dsl::transducer::packed::LoweredWave::new(&clauses).map_err(|e| format!("the ast matchers as one wave: {e:?}"))?))
        };
        Ok(Self { list, wave })
    }
    pub(crate) fn len(&self) -> usize {
        self.list.len()
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
}

/// A chunk's matcher wave as one launch of the executor (decision 0107), as every other dispatch: priced by its tape
/// nodes and passes, placed on the cores or the GPU, run on that lane's worker and scored. Its GPU form is the wave's
/// passes and roots in one command buffer; its cores' form the interpreter, which the GPU's runs instead where the
/// device refuses the batch.
struct MatcherWave {
    wave: Arc<crate::code_rule_dsl::transducer::packed::LoweredWave>,
    tapes: Vec<crate::code_rule_dsl::tape::TapeDoc>,
    /// Per matcher, per tape: whether the matcher runs there (its gates hold).
    active: Vec<Vec<bool>>,
    gpu: bool,
}

/// What a wave's launch returns: per matcher, per tape, the matched roots; how it ran; its wall seconds on the lane.
type WaveFound = Result<(Vec<Vec<BTreeSet<u32>>>, crate::code_rule_dsl::transducer::packed::WaveRun, f64), String>;

/// The cores' interpreter: seconds a tape node and pass (one thread), and the GPU's. Priors, replaced by the
/// executor's learned rates once it has measured the form.
const WAVE_CPU_PER_NODE_PASS: f64 = 2e-8;
const WAVE_GPU_PER_NODE_PASS: f64 = 1e-9;

impl MatcherWave {
    fn on(&self, backend: crate::code_rule_dsl::transducer::packed::MatcherBackend) -> WaveFound {
        let started = std::time::Instant::now();
        let (found, run) = self.wave.roots_batch(backend, &self.tapes, &self.active).map_err(|e| format!("{e:?}"))?;
        Ok((found, run, started.elapsed().as_secs_f64()))
    }
}

impl rerail_accel::kernel::Program for MatcherWave {
    fn name(&self) -> &'static str {
        "ast matcher wave"
    }

    fn units(&self) -> f64 {
        let passes = self.wave.passes().max(1) as f64;
        self.tapes.iter().map(|t| t.transducer_len() as f64).sum::<f64>() * passes
    }

    fn per_unit(&self, lane: rerail_accel::Lane) -> Option<f64> {
        match lane {
            rerail_accel::Lane::Cpu => Some(WAVE_CPU_PER_NODE_PASS),
            rerail_accel::Lane::Gpu if self.gpu => Some(WAVE_GPU_PER_NODE_PASS),
            _ => None,
        }
    }

    fn run(&self, lane: rerail_accel::Lane) -> Result<rerail_accel::kernel::ProgramOutput, String> {
        use crate::code_rule_dsl::transducer::packed::MatcherBackend;
        #[cfg(target_os = "macos")]
        if lane == rerail_accel::Lane::Gpu
            && let Ok(found) = self.on(MatcherBackend::MetalInterpreter)
        {
            return Ok(Arc::new(Ok(found) as WaveFound));
        }
        let _ = lane;
        Ok(Arc::new(self.on(MatcherBackend::Cpu)))
    }
}

/// Each matcher's relation over the documents' tapes: the chunk's matcher wave as one launch ([`MatcherWave`]), its
/// roots mapped from tape preorder (a core id) to the batch's node ids. A document with no tape (a guest, a text-only
/// file) or one a matcher's gates exclude has no rows of that matcher.
fn matched(
    batch: &Batch,
    tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
    matchers: &Matchers,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    atoms: &crate::content_store::atoms::Atoms,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    use crate::code_rule_dsl::logic::profile;
    let _span = profile::span(|| "ast matchers on the transducer".into());
    let Some(wave) = &matchers.wave else { return Ok(Vec::new()) };
    let metal = matches!(binding, crate::code_rule_dsl::typing::file_local::Binding::Metal);
    // Each tape's document in the batch; the tapes outside it take no part.
    let at: Vec<Option<&Projected>> = tapes.iter().map(|(facts, _, _)| batch.docs.iter().find(|p| Arc::ptr_eq(&p.facts, facts))).collect();
    let held: Vec<usize> = (0..tapes.len()).filter(|&i| at[i].is_some()).collect();
    // A matcher runs where its gates hold and W admits the file: its `syntax` (valid or invalid) and text literals.
    let errors: Vec<crate::code_rule_dsl::soundness::SyntaxErrors> = held.iter().map(|&i| crate::code_rule_dsl::soundness::SyntaxErrors::of_doc(&tapes[i].1)).collect();
    let active: Vec<Vec<bool>> = matchers
        .list
        .iter()
        .map(|m| {
            held.iter()
                .zip(&errors)
                .map(|(&i, errors)| {
                    let source = tapes[i].0.source.as_str();
                    m.gates.iter().all(|g| g.holds(&tapes[i].2, source)) && m.captor.grammar.admits_source(source, errors, &mut rustc_hash::FxHashMap::default())
                })
                .collect()
        })
        .collect();
    let docs: Vec<crate::code_rule_dsl::tape::TapeDoc> = held.iter().map(|&i| tapes[i].1.clone()).collect();
    let nodes: u64 = docs.iter().map(|d| d.transducer_len() as u64).sum();
    let (found, limited) = if docs.is_empty() || active.iter().all(|a| !a.iter().any(|&on| on)) {
        (vec![vec![BTreeSet::new(); docs.len()]; matchers.len()], Vec::new())
    } else {
        let program = MatcherWave { wave: Arc::clone(wave), tapes: docs, active, gpu: cfg!(target_os = "macos") && metal };
        let kernel = rerail_accel::Kernel::Program(rerail_accel::kernel::ProgramKernel(Arc::new(program)));
        let ex = crate::code_rule_dsl::logic::lanes::executor_for(metal);
        let (mut done, _) = crate::code_rule_dsl::logic::lanes::prof::run_on(ex, "ast matcher wave", rerail_accel::Batch(vec![rerail_accel::Launch::new(kernel)])).map_err(StoreError::Invalid)?;
        let done = done.remove(0);
        let rerail_accel::kernel::Output::Program(r) = done.out else {
            return Err(StoreError::Invalid("ast matcher wave: not a program's result".into()));
        };
        let r = r.0.downcast::<WaveFound>().map_err(|_| StoreError::Invalid("ast matcher wave: a result of another type".into()))?;
        let (found, run, wall) = r.as_ref().clone().map_err(|e| StoreError::Invalid(format!("ast matcher wave: {e}")))?;
        if run.metal {
            profile::device("ast matcher wave", run.command_buffers as u64, run.device, wall, nodes);
        }
        profile::decision("ast matcher wave lane", || (if run.metal { "metal" } else { "cpu" }).to_owned());
        if !run.limited.is_empty() {
            profile::decision("ast matcher roots over the work limit (confirmed by W)", || format!("{} matcher-files", run.limited.len()));
        }
        (found, run.limited)
    };
    profile::work(profile::Unit::Node, nodes);
    let mut out = Vec::new();
    for (c, (m, found)) in matchers.list.iter().zip(found).enumerate() {
        let columns = crate::code_rule_dsl::logic::plan::corpus::ast_columns(&m.rel);
        let mut r = Relation::new(columns);
        let mut rows = 0u64;
        // A tape where this matcher's program outran the work limit gave every candidate root: W confirms them there.
        let over: Vec<usize> = limited.iter().filter(|&&(at, _)| at == c).map(|&(_, t)| t).collect();
        if m.captures.is_empty() && !m.captor.verify && !over.is_empty() {
            let _confirm = profile::span(|| "ast roots over the work limit, confirmed (W's matcher at each root)".into());
            let held_over: Vec<usize> = over.iter().map(|&t| held[t]).collect();
            let found_over: Vec<BTreeSet<u32>> = over.iter().map(|&t| found[t].clone()).collect();
            let (found_rows, budget) = captured_rows(m, &held_over, &found_over, &at, tapes, atoms)?;
            if budget > 0 {
                profile::decision("ast captures over a file's visit budget (no rows)", || format!("{}: {budget} roots", m.rel));
            }
            for row in found_rows {
                r.push(&row);
                rows += 1;
            }
        }
        if m.captures.is_empty() && !m.captor.verify {
            for (t, (&i, found)) in held.iter().zip(found).enumerate() {
                if over.contains(&t) {
                    continue;
                }
                let p = at[i].expect("held");
                for core in found {
                    let local = p.local.get(core as usize).copied().unwrap_or(NONE);
                    if local != NONE {
                        r.push(&[p.global(local)]);
                        rows += 1;
                    }
                }
            }
        } else {
            // Captures, or a relaxed matcher's roots to confirm (decision 0107): W's matcher at each root.
            let _captures = profile::span(|| "ast captures and confirmations (W's matcher at each root)".into());
            let roots: usize = found.iter().map(BTreeSet::len).sum();
            let (found_rows, limited) = captured_rows(m, &held, &found, &at, tapes, atoms)?;
            if limited > 0 {
                profile::decision("ast captures over a file's visit budget (no rows)", || format!("{}: {limited} roots", m.rel));
            }
            // A relaxed matcher's cost (decision 0107 §3): the roots W confirmed on the host, and the rows it kept.
            if m.captor.verify {
                crate::code_rule_dsl::prof::count("ast.relaxed_roots", roots);
                crate::code_rule_dsl::prof::count("ast.relaxed_kept", found_rows.len());
                profile::decision("relaxed matchers: roots W confirmed, rows kept", || format!("{}: {roots} roots, {} kept", m.rel, found_rows.len()));
            }
            for row in found_rows {
                r.push(&row);
                rows += 1;
            }
        }
        crate::code_rule_dsl::prof::count("ast.matched", rows as usize);
        out.push((m.rel.clone(), Arc::new(r) as Arc<dyn Source>));
    }
    Ok(out)
}

/// A captured matcher's rows (decision 0107): at each root the transducer matched, W's matcher of the atom's rule
/// matched again on the host, and each capture's nodes read from its bindings (a multi-capture's every node, so its
/// rows are the product of the captures' nodes). A node the projection leaves out has no row. Tapes in parallel; also
/// how many roots hit their file's visit budget (no rows).
fn captured_rows(
    m: &Matcher,
    held: &[usize],
    found: &[BTreeSet<u32>],
    at: &[Option<&Projected>],
    tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
    atoms: &crate::content_store::atoms::Atoms,
) -> StoreResult<(Vec<Vec<i64>>, usize)> {
    use crate::code_rule_dsl::waves::findings::Capture;
    use rayon::prelude::*;
    let per: Vec<StoreResult<(Vec<Vec<i64>>, usize)>> = held
        .par_iter()
        .zip(found.par_iter())
        .map(|(&i, roots)| {
            let mut rows = Vec::new();
            let mut limited = 0;
            if roots.is_empty() {
                return Ok((rows, limited));
            }
            let p = at[i].expect("held");
            let tape = &tapes[i].1;
            // W's matcher reads the file's path (`filePath`, which a relaxed matcher leaves to it).
            let _file = crate::code_rule_dsl::matcher::FileScopeSlot::install(Vec::new(), None, !tape.syntax_error_ranges().is_empty()).path(&tapes[i].2);
            let root = ast_grep_core::AstGrep::doc(tape.clone());
            let work = m.captor.limits.visits_for(tape.transducer_len());
            let global = |core: u32| -> Option<i64> {
                let local = p.local.get(core as usize).copied().unwrap_or(NONE);
                (local != NONE).then(|| p.global(local))
            };
            for &n in roots {
                let Some(n_global) = global(n) else { continue };
                let names = match crate::code_rule_dsl::waves::findings::captured(&m.captor.grammar, &root, n, &m.captures, work) {
                    Ok(Some(names)) => names,
                    Ok(None) => continue,
                    Err(_) => {
                        limited += 1;
                        continue;
                    }
                };
                let mut product: Vec<Vec<i64>> = vec![vec![n_global]];
                for capture in names {
                    // A transform's text is a symbol, the store's atom, as the program's constants are.
                    let nodes: Vec<i64> = match capture {
                        Capture::Nodes(nodes) => nodes.into_iter().filter_map(global).collect(),
                        Capture::Text(text) => vec![i64::from(atoms.intern(&text)?)],
                    };
                    product = product.into_iter().flat_map(|row| nodes.iter().map(move |&x| {
                        let mut row = row.clone();
                        row.push(x);
                        row
                    })).collect();
                }
                rows.extend(product);
            }
            Ok((rows, limited))
        })
        .collect();
    let per = per.into_iter().collect::<StoreResult<Vec<_>>>()?;
    let limited = per.iter().map(|(_, l)| l).sum();
    Ok((per.into_iter().flat_map(|(rows, _)| rows).collect(), limited))
}

/// [`with_nearest`], with each of `matchers`' relations over the documents' tapes (decision 0106).
pub(crate) fn with_matchers(
    store: &ContentStore,
    documents: Vec<Document>,
    sets: &[(String, Vec<i64>)],
    matchers: &Matchers,
    tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    projection: &Projection,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    let built = crate::code_rule_dsl::logic::profile::span(|| "project and intern".into());
    let metal = matches!(binding, crate::code_rule_dsl::typing::file_local::Binding::Metal);
    let tapes: Vec<(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)> = if matchers.is_empty() {
        Vec::new()
    } else {
        documents.iter().filter(|d| d.guest.is_none()).filter_map(|d| Some((Arc::clone(&d.facts), d.tape.clone()?, d.path.clone()))).collect()
    };
    let (batch, relations, (strings, symbols)) = Batch::build(store, documents, projection, metal)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, batch.nodes.last().copied().unwrap_or(0) as u64);
    drop(built);
    let mut out: Vec<(String, Arc<dyn Source>)> = relations.into_iter().map(|(name, r)| (name.to_owned(), Arc::new(r) as Arc<dyn Source>)).collect();
    if !matchers.is_empty() {
        out.extend(matched(&batch, &tapes, matchers, binding, &store.atoms_handle())?);
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
    out.extend(views(&batch).into_iter().map(|(name, source)| (name.to_owned(), source)));
    drop(viewing);
    // The kind-set axes, made per looked-up node like containment (decision 0050): a set no join reads, and the nodes
    // no join binds, cost nothing. Their statistics come from the tree.
    let _nearest = crate::code_rule_dsl::logic::profile::span(|| "nearest".into());
    use crate::code_rule_dsl::logic::front::{AXIS, DESCENDANTS};
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
pub(crate) fn statistics(source: &dyn Source) -> Vec<(String, i64, i64, i64, i64)> {
    if let Some(known) = source.statistics() {
        return known;
    }
    measured(source)
}

/// `statistics`, measured from the rows (a source that keeps its own calls this once).
pub(crate) fn measured(source: &dyn Source) -> Vec<(String, i64, i64, i64, i64)> {
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
pub(crate) fn fresh(graph: &(impl Graph + ?Sized), documents: &[Document]) -> StoreResult<()> {
    for d in documents {
        if graph.binding(&d.core)? != Some(d.digest) {
            return Err(StoreError::Invalid(format!("stale core facts for {}", d.core)));
        }
    }
    Ok(())
}

/// Binds T's base relations over `documents` on `db`, after checking that each document's facts are
/// those its `#core:` name binds now (§4): a stale document fails the binding.
pub(crate) fn bind(modules: &Modules, db: &Connection, graph: &(impl Graph + ?Sized), documents: Vec<Document>) -> StoreResult<()> {
    fresh(graph, &documents)?;
    for (name, source) in sources(graph.store(), documents)? {
        modules.bind(db, name, source).map_err(StoreError::Database)?;
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::code_rule_dsl) mod tests;

thread_local! {
    /// `Batch::locate`'s hint: the offsets array (its address) and the document last found in it.
    static LOCATED: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
    /// `Batch::doc`'s hint: the documents (their address) and the index last found.
    static DOC: std::cell::Cell<(usize, usize)> = const { std::cell::Cell::new((0, 0)) };
}

#[cfg(test)]
mod candidate_spike;
