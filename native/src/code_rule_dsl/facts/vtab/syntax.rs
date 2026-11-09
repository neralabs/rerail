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
//! among them. Every value is an integer: a node is `d << 32 | its preorder index`, so a document is
//! an interval of ids and a subtree the ids `n + 1 ..= n + size`; `d` is the caller's key for the
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
use crate::code_rule_dsl::facts::producers::python::families as py;
use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::Connection;
use crate::code_rule_dsl::logic::front::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

mod within;

/// The language's `within(n, a)`, `n` strictly below `a` (`within::Within`, tree mode).
pub(crate) const WITHIN: &str = "within";
/// `span_within(inner, outer)`, `outer`'s span holding `inner`'s (`within::Within`, span mode).
pub(crate) const SPAN_WITHIN: &str = "span_within";
mod core_rr;
pub(crate) mod normalized;
mod python_syntax;
mod python_parts;
use core_rr::Family;
mod guests;
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
/// reads is not made where making it costs (`text` of every node, the tokens); one the program derives is not bound
/// (`derived`).
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
    /// The grammars whose guests keep their partial tree when they do not parse (`.on guest_error partial`,
    /// decision 0055 §6); `"*"` is every grammar. Any other broken guest is withheld.
    pub(crate) partial: BTreeSet<String>,
    /// In T's shape, `syntax_error` is core.rr's `syntax_error(d, l, msg)`, CPython's verdict, not T's
    /// `syntax_error(d)`: the program declares that one (a program of core.rr's base relations, not T's).
    verdict: bool,
    /// The relations the program's rules derive: the batch binds none of them, made or viewed (T's programs derive
    /// `docof` from `node`, `rr::lower::projections`; a program may name an output as a base relation is named). A
    /// bound table of that name would be the planner's to drop.
    derived: BTreeSet<String>,
}

impl Projection {
    pub(crate) fn core() -> Self {
        Projection { shape: Shape::Core, reads: None, rx_spans: Vec::new(), ast_patterns: Vec::new(), text_spans: None, partial: BTreeSet::new(), verdict: false, derived: BTreeSet::new() }
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
        let core = crate::code_rule_dsl::logic::rr::core_names();
        let shape = if !core.is_empty() && core.iter().all(|r| program.decls.contains_key(r)) { Shape::Core } else { Shape::T };
        let partial = program.facts.get(GUEST_POLICY).into_iter().flatten().filter_map(|row| match row.as_slice() {
            [Value::Str(k), Value::Str(v)] if v == "partial" => k.strip_prefix("errors").map(|g| g.strip_prefix(':').unwrap_or("*").to_owned()),
            _ => None,
        });
        let verdict = shape == Shape::T && program.decls.get("syntax_error").is_some_and(|cols| cols.len() == 3);
        let derived: BTreeSet<String> = program.rules.iter().map(|r| r.head.rel.clone()).collect();
        let reads = reads.into_iter().filter(|r| !derived.contains(r)).collect();
        Projection { shape, reads: Some(reads), rx_spans: program.rx_spans.iter().cloned().collect(), ast_patterns: program.ast_patterns.iter().cloned().collect(), text_spans: Some(span_text_demand(program)), partial: partial.collect(), verdict, derived }
    }

    /// The columns the batch binds base relation `rel` with (`select::input_columns`'), but T's `syntax_error(d)`
    /// where the program declares that one and not core.rr's verdict (`verdict`): the cross-file part binds a base
    /// relation with the chunks' columns, on the same connection.
    pub(crate) fn columns(&self, rel: &str) -> Option<&'static [&'static str]> {
        if rel == "syntax_error" && self.shape == Shape::T && !self.verdict {
            return Some(&["d"]);
        }
        crate::code_rule_dsl::logic::sql::select::input_columns(rel)
    }

    fn wants_span_text(&self, within: crate::code_rule_dsl::logic::front::SpanIn) -> bool {
        use crate::code_rule_dsl::logic::front::SpanIn;
        self.wants("text") && self.text_spans.as_ref().map_or_else(
            || self.wants(match within { SpanIn::Source => "source", SpanIn::Line => "src_line", _ => "text" }),
            |spans| spans.contains(&within),
        )
    }
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
    /// The tape the facts were read off and the file it parsed, which W's values, f-string trees and string annotations
    /// read (`TAPED`); kept only for a file of a program that reads one of them, else `None` (a guest's is `None`).
    pub(in crate::code_rule_dsl) tape: Option<(crate::code_rule_dsl::tape::TapeDoc, crate::code_rule_dsl::model::FileInput)>,
    /// A guest's place in its host (decision 0055); `None` for a file's own tree. A guest's `path` is its host's.
    pub guest: Option<Arc<Guest>>,
    /// The module a loaded document is (t21 E2: `load(path, module, …)`, the module its resolution gave); `None` for
    /// a file of the corpus, whose module follows from its path.
    pub module: Option<Arc<str>>,
}

/// The relations read off a document's tape, for which a batch's documents keep their tapes (`Document::tape`): W's
/// values (`Extra`), its f-string trees (`fstring`) and the string annotations (`annotation`).
pub(crate) const TAPED: [&str; 7] =
    ["value", "static_value", "value_type_of", "value_type", fstring::RELATIONS[0], fstring::RELATIONS[1], annotation::RELATION];

/// A comment's text without its marker (core.rr's `comment`): `#`, `--`, `//` (and a doc comment's `/` or `!`),
/// `/*` to `*/` (and a doc comment's `*` or `!`) or `<!--` to `-->` taken off, then the whitespace around the rest.
pub(crate) fn comment_body(text: &str) -> &str {
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

pub(crate) fn numbering(doc: &Document, shape: Shape) -> StoreResult<Numbering> {
    // A document read as text only has no nodes.
    if doc.facts.len() == 0 {
        return Ok(Numbering { nodes: Vec::new(), back: None });
    }
    let tag = crate::code_rule_dsl::facts::core::facts_tag(&doc.facts);
    let dialect = Dialect::new(tag)?;
    // In `core.rr`'s shape a guest's nodes are its root's subtree; in T's, its whole tree (`Batch::build`).
    let guest = doc.guest.as_deref().filter(|_| shape == Shape::Core).map(|g| (g, Arc::new(core_rr::Lines::new("", false))));
    let p = Projected::new(doc.key, Arc::clone(&doc.facts), Some(&dialect), shape, false, guest);
    let nodes = p.core.iter().map(|&id| (id, dialect.name(&doc.facts, id))).collect();
    // The ids the projection gives (`Projected::id`): per definition where it numbered the document so.
    Ok(Numbering { nodes, back: p.defs.map(|d| d.back) })
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
    /// Whether W reads values of the dialect's nodes (its family's `ValueRules::SUPPORTED`): `Extra`'s `value` and
    /// `value_type` are W's for its documents, `core_rr`'s literal values for the others (an `.rr` file's).
    values: bool,
}
impl Dialect {
    fn new(tag: u8) -> StoreResult<Self> {
        use crate::code_rule_dsl::language::adapter::{Adapter, ValueRules, with_adapter};
        let dialect = from_tape_tag(tag).ok_or_else(|| StoreError::Invalid(format!("no dialect has tape tag {tag}")))?;
        let catalog = dialect.kinds();
        let values = with_adapter!(crate::code_rule_dsl::language::Family::of(dialect), A => <<A as Adapter>::Value as ValueRules>::SUPPORTED);
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
        Ok(Self { tag, family, catalog, classes, operator: catalog.field_id("operator"), operators: catalog.field_id("operators"), canonical, codes, values })
    }
    /// Node `id`'s kind code (ERROR's is `u16::MAX`).
    fn code(&self, f: &Facts, id: u32) -> u16 {
        kind_code(f.kind[id as usize])
    }
    fn class(&self, f: &Facts, id: u32) -> Class {
        self.classes.get(usize::from(self.code(f, id))).copied().unwrap_or(Class::Other)
    }
    fn name(&self, f: &Facts, id: u32) -> &'static str {
        self.catalog.kind_name(self.code(f, id)).unwrap_or("")
    }
    /// Node `id`'s kind atom as the `kind` column holds it: its name's canonical code's.
    fn kind(&self, f: &Facts, id: u32) -> i64 {
        let code = self.code(f, id);
        i64::from(kind_atom(self.tag, self.canonical.get(usize::from(code)).copied().unwrap_or(code)))
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
    key: u32,
    tag: u8,
    facts: Arc<Facts>,
    /// The core id of each node.
    core: Vec<u32>,
    /// The node of each core id, `NONE` for one left out.
    local: Vec<u32>,
    parent: Vec<u32>,
    idx: Vec<u32>,
    size: Vec<u32>,
    depth: Vec<u32>,
    /// The nodes hanging from their parent under a field, ascending.
    fielded: Vec<u32>,
    /// `core.rr`'s lines and spans; `None` in T's shape.
    extra: Option<Box<Locs>>,
    /// Node ids numbered per definition (`DefIds`), when the run asks for them; else a node's id is its preorder.
    defs: Option<DefIds>,
}

/// Node ids numbered per definition (decision 0083 §3, `RERAIL_NODE_IDS=definition`): a node's id is
/// `document << 32 | slot << 18 | index`. Its definition unit (the document's root, each class and function:
/// `core_rr::is_unit`) has a slot, by a hash of its qualified name and its ordinal among same-named units in its
/// parent; the index counts the unit's own nodes in preorder, then its tokens. An edit inside one function then
/// renumbers that function's nodes and no other's. Spans that are positions (a document's text, its lines, matches)
/// are in slot `SPANS`, in order. Slots take 13 bits, so every such id stays below `SPAN` (bit 31), the mark of
/// `Extra`'s span ids: a node by definition is never read as one of those spans, nor one of them as a node.
struct DefIds {
    /// The low 32 bits of each node's and token's id, by local id.
    low: Vec<u32>,
    /// `(low, local)`, ascending: the way back.
    back: Vec<(u32, u32)>,
}

const INDEX_BITS: u32 = 18;
const SLOT_BITS: u32 = 31 - INDEX_BITS;
const SPANS: u32 = (1 << SLOT_BITS) - 1;
const _: () = assert!(SPANS << INDEX_BITS | ((1 << INDEX_BITS) - 1) < SPAN);

/// Why a document keeps preorder ids when the run numbers nodes per definition.
#[derive(Debug, PartialEq, Eq)]
enum Unnumbered {
    /// More definition units than the slots below `SPANS`.
    Units(usize),
    /// A unit with more nodes and tokens than a slot has indexes.
    Indexes,
}

/// Whether this process numbers nodes per definition (`RERAIL_NODE_IDS=definition`).
pub(crate) fn definition_ids() -> bool {
    static ON: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| std::env::var("RERAIL_NODE_IDS").is_ok_and(|v| v == "definition"));
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

/// The slot of each unit, by its key, in order: the root's is 0, every other's in `1..SPANS` by the key's hash, the
/// next free one on a collision.
fn unit_slots(keys: &[String]) -> Result<Vec<u32>, Unnumbered> {
    if keys.len() >= SPANS as usize {
        return Err(Unnumbered::Units(keys.len()));
    }
    let mut taken = vec![false; SPANS as usize];
    taken[0] = true;
    let mut slots = Vec::with_capacity(keys.len());
    slots.push(0);
    for k in keys.iter().skip(1) {
        let mut slot = (u64::from_le_bytes(<sha2::Sha256 as sha2::Digest>::digest(k.as_bytes())[..8].try_into().unwrap()) % u64::from(SPANS - 1)) as u32 + 1;
        while taken[slot as usize] {
            slot = slot % (SPANS - 1) + 1;
        }
        taken[slot as usize] = true;
        slots.push(slot);
    }
    Ok(slots)
}

impl DefIds {
    /// The ids of `p`'s nodes, and of `tokens` (each token's node, in order), by definition, with `positions` spans
    /// (its text and lines) in slot `SPANS`; `Unnumbered` when a unit or the positions would overflow a slot's
    /// indexes or there are more units than slots (the document keeps preorder ids).
    fn new(p: &Projected, dialect: &Dialect, tokens: &[u32], positions: usize) -> Result<Self, Unnumbered> {
        if positions > 1 << INDEX_BITS {
            return Err(Unnumbered::Indexes);
        }
        let f = &*p.facts;
        let n = p.core.len();
        // Each node's unit: itself when it is one, else its parent's.
        let mut unit = vec![0u32; n];
        let mut units: Vec<u32> = Vec::new();
        for l in 0..n {
            unit[l] = if l == 0 || core_rr::is_unit(dialect.family, dialect.name(f, p.core[l])) { units.push(l as u32); l as u32 } else { unit[p.parent[l] as usize] };
        }
        // A unit's key: its parent unit's, its name, and its ordinal among same-named units of that parent.
        let mut key: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
        let mut seen: std::collections::HashMap<(u32, String), u32> = std::collections::HashMap::new();
        let mut keys: Vec<String> = Vec::with_capacity(units.len());
        key.insert(0, String::new());
        keys.push(String::new());
        for &u in units.iter().skip(1) {
            let parent = unit[p.parent[u as usize] as usize];
            let name = core_rr::unit_name(f, dialect, p.core[u as usize]).unwrap_or_default().to_owned();
            let ord = seen.entry((parent, name.clone())).or_insert(0);
            let k = format!("{}.{}#{}", key[&parent], name, *ord);
            *ord += 1;
            key.insert(u, k.clone());
            keys.push(k);
        }
        let slot_of: std::collections::HashMap<u32, u32> = units.iter().copied().zip(unit_slots(&keys)?).collect();
        let mut next: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut low = Vec::with_capacity(n + tokens.len());
        for owner in (0..n as u32).map(|l| unit[l as usize]).chain(tokens.iter().map(|&t| unit[t as usize])) {
            let i = next.entry(owner).or_insert(0);
            if *i >= 1 << INDEX_BITS {
                return Err(Unnumbered::Indexes);
            }
            low.push(slot_of[&owner] << INDEX_BITS | *i);
            *i += 1;
        }
        let mut back: Vec<(u32, u32)> = low.iter().enumerate().map(|(l, &g)| (g, l as u32)).collect();
        back.sort_unstable();
        Ok(DefIds { low, back })
    }
}

/// A document's lines, and its spans past its nodes (`Shape::Core`'s): from local id `len()`, its anonymous tokens
/// (`tokens` of them, in preorder), then its whole text, then each line. Not `Extra`'s spans (`SPAN`), which the
/// relations core.rr's programs read beyond T's are made over.
struct Locs {
    lines: Arc<core_rr::Lines>,
    spans: Vec<[u32; 2]>,
    tokens: u32,
    /// A guest's host lines and placement: its places are the host's (decision 0055). A guest has no source-text
    /// spans of its own (`source`, `src_line`): those are its file's.
    host: Option<(Arc<core_rr::Lines>, Placement)>,
}

impl Locs {
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
    fn new(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape, withheld: bool, guest: Option<(&Guest, Arc<core_rr::Lines>)>) -> Self {
        let f = &*facts;
        let mut p = Projected {
            key,
            tag: f.tag,
            core: Vec::new(),
            local: vec![NONE; f.len()],
            parent: Vec::new(),
            idx: Vec::new(),
            size: Vec::new(),
            depth: Vec::new(),
            fielded: Vec::new(),
            extra: None,
            defs: None,
            facts: Arc::clone(&facts),
        };
        let mut next_idx: Vec<u32> = Vec::new();
        let (first, last) = match guest {
            Some((g, _)) => (g.root, f.subtree_end.get(g.root as usize).copied().unwrap_or(g.root + 1).max(g.root + 1)),
            None => (0, f.len() as u32),
        };
        for id in first..if withheld { first } else { last.min(f.len() as u32) } {
            if !f.named(id) || dialect.is_none_or(|dialect| shape == Shape::T && dialect.class(f, id) == Class::Comment) {
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
            p.parent.push(parent);
            next_idx.push(0);
            if parent == NONE {
                p.idx.push(0);
                p.depth.push(0);
            } else {
                p.idx.push(next_idx[parent as usize]);
                next_idx[parent as usize] += 1;
                p.depth.push(p.depth[parent as usize] + 1);
                if f.field[id as usize] != 0 {
                    p.fielded.push(local);
                }
            }
        }
        p.size = vec![0; p.core.len()];
        for i in (1..p.core.len()).rev() {
            let q = p.parent[i] as usize;
            p.size[q] += p.size[i] + 1;
        }
        if shape == Shape::Core {
            let lines = Arc::new(core_rr::Lines::new(&f.source, dialect.is_some_and(|d| d.family == Family::Python)));
            let mut spans: Vec<[u32; 2]> = Vec::new();
            let mut token_nodes: Vec<u32> = Vec::new();
            for (l, &id) in p.core.iter().enumerate() {
                for &c in f.children(id) {
                    if !f.named(c) && f.end[c as usize] > f.start[c as usize] {
                        spans.push([f.start[c as usize], f.end[c as usize]]);
                        token_nodes.push(l as u32);
                    }
                }
            }
            if definition_ids() && guest.is_none() && let Some(dialect) = dialect {
                p.defs = DefIds::new(&p, dialect, &token_nodes, 1 + lines.count()).ok();
            }
            let tokens = spans.len() as u32;
            // Raw file text remains valid when its tree is withheld. Guests have no own text spans.
            if guest.is_none() {
                spans.push([0, f.source.len() as u32]);
                spans.extend((1..=lines.count()).map(|l| lines.span(l)));
            }
            let host = guest.filter(|(g, _)| g.made == Made::Host).map(|(g, lines)| (lines, g.in_file.clone()));
            p.extra = Some(Box::new(Locs { lines, spans, tokens, host }));
        } else if definition_ids() && guest.is_none() && let Some(dialect) = dialect {
            // T's shape has no token spans: its nodes alone are numbered per definition.
            p.defs = DefIds::new(&p, dialect, &[], 0).ok();
        }
        p
    }
    /// Nodes and spans: the ids `text`, `line` and `docof` take.
    fn locs(&self) -> u32 {
        self.len() + self.extra.as_ref().map_or(0, |x| x.spans.len() as u32)
    }
    /// The byte span of node or span `local`.
    fn span(&self, local: u32) -> [u32; 2] {
        match local.checked_sub(self.len()) {
            None => {
                let id = self.core[local as usize] as usize;
                [self.facts.start[id], self.facts.end[id]]
            }
            Some(i) => self.extra.as_ref().map_or([0, 0], |x| x.spans[i as usize]),
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
            Some(d) => d.low.get(local as usize).copied().unwrap_or_else(|| SPANS << INDEX_BITS | (local - d.low.len() as u32)),
            None => local,
        };
        i64::from(key) << 32 | i64::from(low)
    }
    /// The node or span whose id's low 32 bits are `low` (`global`'s inverse).
    fn local_of(&self, low: u32) -> Option<u32> {
        match &self.defs {
            Some(d) if low >> INDEX_BITS == SPANS => Some(d.low.len() as u32 + (low & ((1 << INDEX_BITS) - 1))),
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
    locs: Vec<usize>,
    shape: Shape,
    /// The `py.*` families of the batch's Python documents, made on their first read (list step 25).
    py: Option<Py>,
    /// The relations made as rows for what the program reads (`Extra` and those beside it), bound in place of a view
    /// of the same name (`line`, `end_line`, `col`, `docof`: their spans have places too).
    made: BTreeSet<&'static str>,
}

/// A span's id (core.rr's `span`: a piece of a document's text): `d << 32 | SPAN | k`, past every node of its
/// document, so no node view has a row of it (`Batch::node`). `k` is 0 for the whole source (`source`), `l` for
/// physical line `l` (`src_line`), `LF | l` for `\n`-line `l` (`src_line_lf`), `TAPE | l` for the tape's line `l`
/// (`tape_line`), `COMMENT | k` for the document's `k`-th comment from 1, in source order, of those no node is
/// (`comment`: T's nodes leave out a grammar's `comment` kind, as Python's, but keep Rust's `line_comment`, which is
/// then the comment's id) and `MATCH | i` for the document's `i`th match or group of a `match_all` or
/// `match_group` regex.
pub(crate) const SPAN: u32 = 1 << 31;
pub(crate) const LF: u32 = 1 << 30;
pub(crate) const TAPE: u32 = 1 << 29;
pub(crate) const COMMENT: u32 = 1 << 28;
pub(crate) const MATCH: u32 = 1 << 27;

/// The physical lines of `source`, as byte ranges without their terminators: as Python's `str.splitlines()`
/// splits (`splitlines`: `\r\n`, `\n`, `\r`, `\v`, `\f`, `\x1c`-`\x1e`, `\x85`, U+2028, U+2029; W's `lines`
/// regexes), else at `\n` only. A terminator at the end ends the last line: no empty line follows it, and an empty
/// source has none.
pub(crate) fn physical_lines(source: &str, splitlines: bool) -> Vec<std::ops::Range<usize>> {
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

/// The `py.*` families (`facts::producers::python::families`): the producer's grammar, the atoms of its
/// names, and the rows, made on the first read of any of them: a program that reads none does no work.
struct Py {
    grammar: py::Grammar,
    contexts: [i64; 5],
    spreads: [i64; 3],
    rows: std::sync::OnceLock<Vec<Relation>>,
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

    /// `ast_depth(n, k)`: the node's depth, the root's 0.
    Depth,
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
                    self.lines.push([n, i64::from(f.start_line[id as usize])]);
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
            crate::code_rule_dsl::matcher::file_values(tape, file, plain, self.static_value, &mut |followed, ordinal, v, names| {
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
        let span = |k: u32| key << 32 | i64::from(SPAN | k);
        let source = f.source.as_str();
        if self.comment && let Some(dialect) = dialect {
            use crate::code_rule_dsl::matcher::is_comment_kind;
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
                        let c = span(COMMENT | k);
                        let (start, end) = (f.start[id as usize] as usize, f.end[id as usize] as usize);
                        let col = start - f.lines.start(f.lines.line(start));
                        self.span(c, f.start_line[id as usize] as usize, f.lines.line(end) + 1, col, whole, true, text, string);
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
            let sp = span(0);
            self.sources.push([key, sp]);
            let terminated = physical.last().map_or(!source.is_empty(), |r| r.end < source.len());
            self.span(sp, 1, (physical.len() + usize::from(terminated)).max(1), 0, source, self.source_text, text, string);
            source_at = Some(made.len());
            made.push((sp, 0..source.len(), None));
        }
        if self.src_line {
            lines_at.start = made.len();
            for (i, r) in physical.iter().enumerate() {
                let (l, sp) = (i + 1, span(i as u32 + 1));
                self.src_lines.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], self.line_text, text, string);
                made.push((sp, r.clone(), Some(l)));
            }
            lines_at.end = made.len();
        }
        if self.src_line_lf {
            for (i, r) in physical_lines(source, false).into_iter().enumerate() {
                let (l, sp) = (i + 1, span(LF | (i as u32 + 1)));
                self.src_lines_lf.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], true, text, string);
                made.push((sp, r, Some(l)));
            }
        }
        if self.tape_line {
            for i in 0..f.lines.starts().len() {
                let (l, sp) = (i + 1, span(TAPE | (i as u32 + 1)));
                let r = f.lines.start(i)..f.lines.end(i);
                self.tape_lines.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[r.clone()], true, text, string);
                made.push((sp, r, Some(l)));
            }
        }
        // Each regex's matches in each span, and their named groups, numbered through the document.
        let mut next: u32 = 0;
        let mut id = |p: &Projected| -> StoreResult<i64> {
            if next >= MATCH {
                return Err(StoreError::Invalid(format!("document {} has more than {MATCH} match_all matches and groups", p.key)));
            }
            next += 1;
            Ok(span(MATCH | (next - 1)))
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
                use crate::code_rule_dsl::logic::front::SpanIn;
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
                        && !crate::code_rule_dsl::transducer::packed::regex_block::SpanPrefilter::any_in(ends, r.start, r.end)
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
                        dialect.name(f, m) == "function_modifiers" && f.children(m).iter().any(|&k| dialect.name(f, k) == "async")
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
#[cfg(test)]
pub(crate) static BATCHES: parking_lot::Mutex<Vec<(Vec<u32>, std::sync::Weak<Batch>)>> = parking_lot::Mutex::new(Vec::new());

impl Batch {
    /// Projects each document and interns the batch's strings; the documents' keys are distinct.
    /// Returns the batch and its small relations (`doc`, `pkg`, `syntax_error`, `text`, `intval`, `op`), and those
    /// of core.rr's the program `reads` that T had not (`Extra`), the `match_all` relations among them (of `matches`)
    /// apart. A text-only document (`core::TEXT`) has the document's relations and its text's, and no node.
    /// Also returns the batch's distinct strings and their symbols: its vocabulary, which text tests read
    /// (`logic::strings::Vocabulary`, decision 0037).
    #[allow(clippy::type_complexity)]
    #[allow(clippy::too_many_arguments)]
    fn build(
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
        let mut at: HashMap<String, usize> = HashMap::new();
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
        let span_text = [crate::code_rule_dsl::logic::front::SpanIn::Source, crate::code_rule_dsl::logic::front::SpanIn::Line].map(|within| !core || projection.wants_span_text(within));
        let mut extra = Extra::new(reads, matches, shape, span_text);
        let mut tree = tree::Tree::new(reads);
        let mut fstrings = fstring::FStrings::new(reads, markers);
        let mut cpython = cpython::CPython::new(reads);
        let mut annotations = annotation::Annotations::new(reads);
        let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        // `core.rr`'s (decision 0046).
        let mut c = core_rr::CoreRows::default();
        let wants_text = core && projection.wants("text");
        let wants_source_text = core && projection.wants_span_text(crate::code_rule_dsl::logic::front::SpanIn::Source);
        let wants_line_text = core && projection.wants_span_text(crate::code_rule_dsl::logic::front::SpanIn::Line);
        let wants_split_lines = core && projection.wants("split_line");
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
                        SpanIn::Source | SpanIn::Line if on => crate::code_rule_dsl::transducer::packed::regex_block::SpanPrefilter::compile(re, *within == SpanIn::Line),
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
        // Every prefiltered pattern's marks over every file, in one batch of block programs (decision 0081 §3): on Metal
        // under the device binding, the CPU interpreter otherwise. `span_ends[doc][pattern]`.
        let span_ends: Vec<Vec<Option<Vec<u64>>>> = {
            use crate::code_rule_dsl::transducer::packed::regex_block::{RanOn, SpanPrefilter, ends_batch};
            let filters: Vec<(usize, &SpanPrefilter)> = rx_spans.iter().enumerate().filter_map(|(i, (_, _, _, f))| f.as_ref().map(|f| (i, f))).collect();
            let mut out: Vec<Vec<Option<Vec<u64>>>> = vec![vec![None; rx_spans.len()]; documents.len()];
            if !filters.is_empty() {
                let mut at = Vec::new();
                let mut files: Vec<(&[u8], usize)> = Vec::new();
                for (j, d) in documents.iter().enumerate().filter(|(_, d)| d.guest.is_none()) {
                    for (f, _) in filters.iter().enumerate() {
                        at.push((j, filters[f].0));
                        files.push((d.facts.source.as_bytes(), f));
                    }
                }
                let _span = crate::code_rule_dsl::logic::profile::span(|| "span prefilter".into());
                let programs: Vec<&SpanPrefilter> = filters.iter().map(|(_, f)| *f).collect();
                let (ends, ran) = ends_batch(&programs, &files, metal);
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
                    core_rr::verdict(&d.facts, dialect).map_err(|raised| python_syntax::failure(&d.path, &raised))?
                }
                _ => None,
            };
            let withheld = core && error.is_some() && !guest.is_some_and(|g| projection.keeps_partial(&g.grammar));
            let python_lines = dialect.is_some_and(|d| d.family == Family::Python);
            let placed = guest.map(|g| (g, lines_of.get(&file_of(g.host)).cloned().unwrap_or_else(|| Arc::new(core_rr::Lines::new(&d.facts.source, python_lines)))));
            let p = Projected::new(d.key, Arc::clone(&d.facts), dialect, shape, withheld, placed);
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
                c.patterns(&p, dialect, lang, compiled, &mut string);
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
            if let Some(dialect) = dialect {
                rows.document(&p, dialect, core, &mut string);
                if core {
                    for local in 0..p.len() {
                        c.node(&p, dialect, local, &mut string, wants_text, projection.wants("str_part"), projection.wants("str_part_start"), projection.wants("str_part_word"));
                    }
                }
                if let Some((line, msg)) = &error {
                    let line = p.extra.as_ref().map_or(*line, |x| x.file_line(*line));
                    c.withheld(&p, &(line, msg.clone()), &mut string);
                }
            }
            // A text-only document has core.rr's text rows too (its lines, `split_line`), and no tokens.
            if core && (!withheld || guest.is_none()) {
                c.document(&p, dialect, &mut string, wants_text, wants_tokens && !withheld, wants_split_lines, wants_lf_lines, wants_source_text, wants_line_text, guest.is_none());
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
                fstrings.document(&p, d, dialect, python == Some(tag), &mut string)?;
                cpython.document(&p, &d.path, dialect, python == Some(tag), &mut string)?;
            } else {
                // A guest's nodes have their places in its file (`Locs::place`); W's values, f-string trees and CPython's
                // verdicts are a file's, read off its tape.
                extra.nodes(&p, &mut string);
            }
            annotations.document(&p, d, dialect, python == Some(tag))?;
            docs.push(p);
        }
        // The guests' rows, once every document is projected: a guest's host may be a guest after it. Either shape:
        // a program that declares its base relations itself (T's) reads guests as core.rr's do.
        for ((p, d), &withheld) in docs.iter().zip(&documents).zip(&withheld_at) {
            g.document(p, d, d.guest.as_deref(), withheld, &docs, [projection.wants("host_node"), projection.wants("unparse_of")], &mut string);
        }
        let vocabulary = python.filter(|tag| dialects.contains_key(tag)).map(|_| {
            (py::CONTEXTS.map(|name| string(name) as i64), py::SPREADS.map(|name| string(name) as i64))
        });
        let intern = crate::code_rule_dsl::logic::profile::span(|| "intern atoms".into());
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, strings.len() as u64);
        let atoms = store.intern_atoms(&strings.iter().map(String::as_str).collect::<Vec<_>>())?;
        drop(intern);
        let atom = |i: i64| i64::from(atoms[i as usize]);
        let py = vocabulary.zip(python).map(|((contexts, spreads), tag)| Py {
            grammar: py::Grammar::new(tag, &dialects[&tag].catalog),
            contexts: contexts.map(atom),
            spreads: spreads.map(atom),
            rows: Default::default(),
        });
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
            ("intval", pairs(&["n", "v"], &rows.intval, [false, false])),
            ("op_t", pairs(&["n", "s"], &rows.op, [false, true])),
        ];
        relations.extend(g.relations(&atom));
        if core {
            extra.literal_values(c.values_of(&unvalued));
        }
        let (mut read, matched) = extra.relations(&atom);
        read.extend(tree.relations(&atom));
        read.extend(fstrings.relations(&atom));
        read.extend(cpython.relations(&atom));
        read.extend(annotations.relations());
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
        let fields = offsets(&|p| p.fielded.len());
        let locs = offsets(&|p| p.locs() as usize);
        let batch = Arc::new(Batch { docs, dialects, nodes, children, fields, locs, shape, py, made });
        #[cfg(test)]
        BATCHES.lock().push((batch.docs.iter().map(|p| p.key).collect(), Arc::downgrade(&batch)));
        Ok((batch, relations, matched, (strings, atoms)))
    }

    fn offsets(&self, view: View) -> &[usize] {
        match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth => &self.nodes,
            View::Child => &self.children,
            View::Field => &self.fields,
            View::Loc(_) => &self.locs,
        }
    }
    /// The document and its row index of `row`.
    fn locate(&self, view: View, row: usize) -> (&Projected, usize) {
        let offsets = self.offsets(view);
        let doc = offsets.partition_point(|&o| o <= row) - 1;
        (&self.docs[doc], row - offsets[doc])
    }
    /// The document of key `d`, by index.
    fn doc(&self, d: i64) -> Option<usize> {
        let d = u32::try_from(d).ok()?;
        self.docs.binary_search_by_key(&d, |p| p.key).ok()
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
        (n >= 0 && local < self.docs[doc].locs()).then_some((doc, local))
    }
    /// The view's row of node `local` of document `doc`, if it has one.
    fn row(&self, view: View, doc: usize, local: u32) -> Option<usize> {
        let p = &self.docs[doc];
        let within = match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Loc(_) => Some(local as usize),
            View::Child => (local > 0).then(|| local as usize - 1),
            View::Field => p.fielded.binary_search(&local).ok(),
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
            View::Loc(Place::Line | Place::EndLine) => &["n", "l"],
            View::Loc(Place::Col) => &["n", "c"],
            View::Loc(Place::Docof) => &["n", "d"],
        }
    }
    /// The column holding the row's own node, which ranges and direct lookups address.
    fn own(&self) -> usize {
        match self.view {
            View::Node | View::Child => 1,
            View::Line | View::Depth | View::Loc(_) => 0,
            View::Field | View::Core => 2,
            View::Tape => 0,
        }
    }
    /// The column holding the row's parent, looked up through the children lists.
    fn parent(&self) -> Option<usize> {
        match self.view {
            View::Child | View::Field => Some(0),
            View::Tape => Some(1),
            View::Node | View::Line | View::Core | View::Depth | View::Loc(_) => None,
        }
    }
}
impl Source for Viewed {
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
            let (line, last, col) = p.extra.as_ref().expect("core.rr's lines").place(p.span(local));
            return match place {
                Place::Line => i64::from(line),
                Place::EndLine => i64::from(last),
                Place::Col => i64::from(col),
                Place::Docof => i64::from(p.key),
            };
        }
        let local = match self.view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Loc(_) => i as u32,
            View::Child => i as u32 + 1,
            View::Field => p.fielded[i],
        };
        let l = local as usize;
        let id = p.core[l] as usize;
        let parent = || if p.parent[l] == NONE { -1 } else { p.global(p.parent[l]) };
        match (self.view, column) {
            (View::Node | View::Core, 0) => i64::from(p.key),
            (View::Core, 1) => id as i64,
            (View::Node, 2) => self.batch.dialects[&p.tag].kind(&p.facts, id as u32),
            (View::Node, 3) => i64::from(p.facts.start[id]),
            (View::Node, 4) => i64::from(p.facts.end[id]),
            (View::Line, 1) => i64::from(p.facts.start_line[id]),
            (View::Child | View::Field, 0) | (View::Tape, 1) => parent(),
            (View::Child, 2) => i64::from(p.idx[l]),
            (View::Field, 1) => i64::from(field_atom(p.tag, p.facts.field[id])),
            (View::Tape, 2) => i64::from(p.size[l]),
            (View::Tape, 3) => i64::from(p.depth[l]),

            (View::Depth, 1) => i64::from(p.depth[l]),
            _ => p.global(local),
        }
    }
}

/// The field atom of field `code` in the dialect whose tape tag is `tag`.
pub(crate) fn field_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) << 16 | u32::from(code)
}

/// Document `key`'s `function_definition` and `class_definition` nodes by byte range (start, end), as ids of
/// `node`: what W's call summaries name a definition by (`calls::summary::Def`).
pub(crate) fn definitions(key: u32, facts: &Arc<Facts>) -> StoreResult<HashMap<(u32, u32), i64>> {
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
pub(crate) fn sources(store: &ContentStore, documents: Vec<Document>) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    shaped(store, documents, &Projection::default())
}

/// The base relations over `documents` in `projection`'s shape, and the `py.*` families, by table name.
pub(crate) fn shaped(store: &ContentStore, documents: Vec<Document>, projection: &Projection) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    let (batch, relations, _, _) = Batch::build(store, documents, projection, &[], &[], false)?;
    let mut out: Vec<(&'static str, Arc<dyn Source>)> =
        relations.into_iter().map(|(name, r)| (name, Arc::new(r) as Arc<dyn Source>)).collect();
    out.extend(views(&batch));
    Ok(out)
}

/// The batch's views: `node`, `child`, `field`, `tape` and the `py.*` families.
fn views(batch: &Arc<Batch>) -> Vec<(&'static str, Arc<dyn Source>)> {
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = Vec::new();
    let mut named = vec![("node", View::Node), ("child", View::Child), ("field", View::Field), ("tape", View::Tape), ("core_node", View::Core)];
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
                let rows = py::rows(&py.grammar, &p.facts);
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
        &self.batch.py()[self.family]
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

/// T's base relations over `documents` (`sources`) in `projection`'s shape and those of core.rr's the program reads
/// that T had not (`Extra`), each of `tests`' relations, and each of `sets` (a relation and its set's kind atoms):
/// `rr_nearest_<set>(n, a)`, the nearest ancestor in the set, `rr_inside_<set>(n, a)`, every one (decision 0036), or
/// `rr_has_<set>(n, a)`, n in the set and a each of its strict ancestors. Each is made per looked-up node on the host
/// (`within::Axis`); `binding` (the transducer's file-local FIX, `typing::file_local::nearest`) no longer makes them
/// for the whole batch first (O87).
pub(crate) fn with_nearest(
    store: &ContentStore,
    documents: Vec<Document>,
    sets: &[(String, Vec<i64>)],
    tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
    binding: crate::code_rule_dsl::typing::file_local::Binding,
    projection: &Projection,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    let built = crate::code_rule_dsl::logic::profile::span(|| "project and intern".into());
    let metal = matches!(binding, crate::code_rule_dsl::typing::file_local::Binding::Metal);
    let (batch, relations, matched, (strings, symbols)) = Batch::build(store, documents, projection, &tests.matches, &tests.markers, metal)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, batch.nodes.last().copied().unwrap_or(0) as u64);
    drop(built);
    // The batch's relations and views, but none the program derives (`Projection::derived`).
    let mut out: Vec<(String, Arc<dyn Source>)> =
        relations.into_iter().filter(|(name, _)| !projection.derived.contains(*name)).map(|(name, r)| (name.to_owned(), Arc::new(r) as Arc<dyn Source>)).collect();
    // The `match_all` relations (`logic::strings::as_relations`): each regex's matches in the batch's spans.
    out.extend(matched.into_iter().map(|(name, r)| (name, Arc::new(r) as Arc<dyn Source>)));
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
    use crate::code_rule_dsl::logic::front::{AXIS, DESCENDANTS};
    for (name, members) in sets {
        let axis = if name.starts_with(DESCENDANTS) {
            within::Axis::Has
        } else if name.starts_with(AXIS) {
            within::Axis::Inside
        } else {
            within::Axis::Nearest
        };
        out.push((name.clone(), Arc::new(within::Within::axis(Arc::clone(&batch), axis, members))));
    }
    // Containment (decision 0050): made per looked-up node, so a program that never reads it pays nothing.
    out.push((WITHIN.to_owned(), Arc::new(within::Within::new(Arc::clone(&batch), within::Mode::Tree))));
    out.push((SPAN_WITHIN.to_owned(), Arc::new(within::Within::new(Arc::clone(&batch), within::Mode::Span))));
    Ok(out)
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
pub(crate) mod annotation;
pub(crate) mod cpython;
mod fstring;
mod tree;
