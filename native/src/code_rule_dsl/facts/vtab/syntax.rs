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
use crate::code_rule_dsl::facts::core::{Facts, KIND_STRIDE, NONE, kind_atom};
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
mod python_order;
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
    /// The grammars whose guests keep their partial tree when they do not parse (`.on guest_error partial`,
    /// decision 0055 §6); `"*"` is every grammar. Any other broken guest is withheld.
    pub(crate) partial: BTreeSet<String>,
}

impl Projection {
    pub(crate) fn core() -> Self {
        Projection { shape: Shape::Core, reads: None, rx_spans: Vec::new(), ast_patterns: Vec::new(), text_spans: None, partial: BTreeSet::new() }
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
        Projection { shape, reads: Some(reads), rx_spans: program.rx_spans.iter().cloned().collect(), ast_patterns: program.ast_patterns.iter().cloned().collect(), text_spans: Some(span_text_demand(program)), partial: partial.collect() }
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
    /// A guest's place in its host (decision 0055); `None` for a file's own tree. A guest's `path` is its host's.
    pub guest: Option<Arc<Guest>>,
    /// The module a loaded document is (t21 E2: `load(path, module, …)`, the module its resolution gave); `None` for
    /// a file of the corpus, whose module follows from its path.
    pub module: Option<Arc<str>>,
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
    let back = definition_ids().then(|| DefIds::new(&p, &dialect, &[])).flatten().map(|d| d.back);
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
    extra: Option<Box<Extra>>,
    /// A demanded semantic visit column, shared by every consumer in this document batch.
    python_visit: std::sync::OnceLock<Vec<u32>>,
    /// Node ids numbered per definition (`DefIds`), when the run asks for them; else a node's id is its preorder.
    defs: Option<DefIds>,
}

/// Node ids numbered per definition (decision 0083 §3, `RERAIL_NODE_IDS=definition`): a node's id is
/// `document << 32 | slot << 18 | index`. Its definition unit (the document's root, each class and function:
/// `core_rr::is_unit`) has a slot, by a hash of its qualified name and its ordinal among same-named units in its
/// parent; the index counts the unit's own nodes in preorder, then its tokens. An edit inside one function then
/// renumbers that function's nodes and no other's. Spans that are positions (a document's text, its lines, matches)
/// are in slot `SPANS`, in order.
struct DefIds {
    /// The low 32 bits of each node's and token's id, by local id.
    low: Vec<u32>,
    /// `(low, local)`, ascending: the way back.
    back: Vec<(u32, u32)>,
}

const SLOT_BITS: u32 = 14;
const INDEX_BITS: u32 = 32 - SLOT_BITS;
const SPANS: u32 = (1 << SLOT_BITS) - 1;

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

impl DefIds {
    /// The ids of `p`'s nodes, and of `tokens` (each token's node, in order), by definition; `None` when a unit
    /// would overflow its slot's indexes or there are more units than slots (the document keeps preorder ids).
    fn new(p: &Projected, dialect: &Dialect, tokens: &[u32]) -> Option<Self> {
        let f = &*p.facts;
        let n = p.core.len();
        // Each node's unit: itself when it is one, else its parent's.
        let mut unit = vec![0u32; n];
        let mut units: Vec<u32> = Vec::new();
        for l in 0..n {
            unit[l] = if l == 0 || core_rr::is_unit(dialect.family, dialect.name(f, p.core[l])) { units.push(l as u32); l as u32 } else { unit[p.parent[l] as usize] };
        }
        if units.len() >= SPANS as usize {
            return None;
        }
        // A unit's key: its parent unit's, its name, and its ordinal among same-named units of that parent.
        let mut key: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
        let mut seen: std::collections::HashMap<(u32, String), u32> = std::collections::HashMap::new();
        let mut slot_of: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut taken = vec![false; SPANS as usize];
        taken[0] = true;
        slot_of.insert(0, 0);
        key.insert(0, String::new());
        for &u in units.iter().skip(1) {
            let parent = unit[p.parent[u as usize] as usize];
            let name = core_rr::unit_name(f, dialect, p.core[u as usize]).unwrap_or_default().to_owned();
            let ord = seen.entry((parent, name.clone())).or_insert(0);
            let k = format!("{}.{}#{}", key[&parent], name, *ord);
            *ord += 1;
            let mut slot = (u64::from_le_bytes(<sha2::Sha256 as sha2::Digest>::digest(k.as_bytes())[..8].try_into().unwrap()) % u64::from(SPANS - 1)) as u32 + 1;
            while taken[slot as usize] {
                slot = slot % (SPANS - 1) + 1;
            }
            taken[slot as usize] = true;
            slot_of.insert(u, slot);
            key.insert(u, k);
        }
        let mut next: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut low = Vec::with_capacity(n + tokens.len());
        for owner in (0..n as u32).map(|l| unit[l as usize]).chain(tokens.iter().map(|&t| unit[t as usize])) {
            let i = next.entry(owner).or_insert(0);
            if *i >= 1 << INDEX_BITS {
                return None;
            }
            low.push(slot_of[&owner] << INDEX_BITS | *i);
            *i += 1;
        }
        let mut back: Vec<(u32, u32)> = low.iter().enumerate().map(|(l, &g)| (g, l as u32)).collect();
        back.sort_unstable();
        Some(DefIds { low, back })
    }
}

/// A document's lines, and its spans past its nodes: from local id `len()`, its anonymous tokens (`tokens` of them,
/// in preorder), then its whole text, then each line.
struct Extra {
    lines: Arc<core_rr::Lines>,
    spans: Vec<[u32; 2]>,
    tokens: u32,
    /// Where `rx_span`'s spans start in `spans` (after the lines); `spans.len()` when there are none.
    matched: u32,
    /// A guest's host lines and placement: its places are the host's (decision 0055). A guest has no source-text
    /// spans of its own (`source`, `src_line`): those are its file's.
    host: Option<(Arc<core_rr::Lines>, Placement)>,
}

impl Extra {
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
        let f = &*facts;
        let mut p = Projected {
            key,
            tag: dialect.tag,
            core: Vec::new(),
            local: vec![NONE; f.len()],
            parent: Vec::new(),
            idx: Vec::new(),
            size: Vec::new(),
            depth: Vec::new(),
            fielded: Vec::new(),
            extra: None,
            python_visit: Default::default(),
            defs: None,
            facts: Arc::clone(&facts),
        };
        let mut next_idx: Vec<u32> = Vec::new();
        let (first, last) = match guest {
            Some((g, _)) => (g.root, f.subtree_end.get(g.root as usize).copied().unwrap_or(g.root + 1).max(g.root + 1)),
            None => (0, f.len() as u32),
        };
        for id in first..if withheld { first } else { last.min(f.len() as u32) } {
            if !f.named(id) || (shape == Shape::T && dialect.class(f, id) == Class::Comment) {
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
            let lines = Arc::new(core_rr::Lines::new(&f.source, dialect.family == Family::Python));
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
            if definition_ids() && guest.is_none() {
                p.defs = DefIds::new(&p, dialect, &token_nodes);
            }
            let tokens = spans.len() as u32;
            // Raw file text remains valid when its tree is withheld. Guests have no own text spans.
            if guest.is_none() {
                spans.push([0, f.source.len() as u32]);
                spans.extend((1..=lines.count()).map(|l| lines.span(l)));
            }
            let matched = spans.len() as u32;
            let host = guest.filter(|(g, _)| g.made == Made::Host).map(|(g, lines)| (lines, g.in_file.clone()));
            p.extra = Some(Box::new(Extra { lines, spans, tokens, matched, host }));
        } else if definition_ids() && guest.is_none() {
            // T's shape has no token spans: its nodes alone are numbered per definition.
            p.defs = DefIds::new(&p, dialect, &[]);
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
    /// `visit_index(n, i)`: semantic field order in core Python, preorder elsewhere.
    Visit,
    /// `ast_depth(n, k)`: the node's depth, the root's 0.
    Depth,
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
        let wants_text = core && projection.wants("text");
        let wants_source_text = core && projection.wants_span_text(crate::code_rule_dsl::logic::front::SpanIn::Source);
        let wants_line_text = core && projection.wants_span_text(crate::code_rule_dsl::logic::front::SpanIn::Line);
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
            let error = if core && (projection.wants("syntax_error") || guest.is_some()) { core_rr::verdict(&d.facts, dialect) } else { None };
            let withheld = error.is_some() && !guest.is_some_and(|g| projection.keeps_partial(&g.grammar));
            let placed = guest.map(|g| (g, lines_of.get(&file_of(g.host)).cloned().unwrap_or_else(|| Arc::new(core_rr::Lines::new(&d.facts.source, dialect.family == Family::Python)))));
            let mut p = Projected::new(d.key, Arc::clone(&d.facts), dialect, shape, withheld, placed);
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
            for (local, &id) in p.core.iter().enumerate() {
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
                    c.node(&p, dialect, local as u32, &mut string, wants_text, wants_parts, wants_part_starts, wants_words);
                }
            }
            if let Some((line, msg)) = &error {
                let line = p.extra.as_ref().map_or(*line, |x| x.file_line(*line));
                c.withheld(&p, &(line, msg.clone()), &mut string);
            }
            if core && (!withheld || guest.is_none()) {
                c.document(&p, dialect, &mut string, wants_text, wants_tokens && !withheld, wants_split_lines, wants_lf_lines, wants_source_text, wants_line_text, guest.is_none());
                if wants_digests && !withheld {
                    c.digests(&p, dialect, &mut string, wants_trees, wants_units);
                }
            }
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
        let fields = offsets(&|p| p.fielded.len());
        let locs = offsets(&|p| p.locs() as usize);
        let batch = Arc::new(Batch { docs, dialects, nodes, children, fields, locs, shape, py });
        #[cfg(test)]
        BATCHES.lock().push((batch.docs.iter().map(|p| p.key).collect(), Arc::downgrade(&batch)));
        Ok((batch, relations, (strings, atoms)))
    }

    fn offsets(&self, view: View) -> &[usize] {
        match view {
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth => &self.nodes,
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
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth | View::Loc(_) => Some(local as usize),
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
            View::Visit => &["n", "i"],
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
            View::Line | View::Visit | View::Depth | View::Loc(_) => 0,
            View::Field | View::Core => 2,
            View::Tape => 0,
        }
    }
    /// The column holding the row's parent, looked up through the children lists.
    fn parent(&self) -> Option<usize> {
        match self.view {
            View::Child | View::Field => Some(0),
            View::Tape => Some(1),
            View::Node | View::Line | View::Core | View::Visit | View::Depth | View::Loc(_) => None,
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
            View::Node | View::Line | View::Tape | View::Core | View::Visit | View::Depth | View::Loc(_) => i as u32,
            View::Child => i as u32 + 1,
            View::Field => p.fielded[i],
        };
        let l = local as usize;
        let id = p.core[l] as usize;
        let parent = || if p.parent[l] == NONE { -1 } else { p.global(p.parent[l]) };
        match (self.view, column) {
            (View::Node | View::Core, 0) => i64::from(p.key),
            (View::Core, 1) => id as i64,
            (View::Node, 2) => {
                let code = crate::code_rule_dsl::facts::core::kind_code(p.facts.kind[id]);
                i64::from(kind_atom(p.tag, self.batch.dialects[&p.tag].canonical.get(usize::from(code)).copied().unwrap_or(code)))
            }
            (View::Node, 3) => i64::from(p.facts.start[id]),
            (View::Node, 4) => i64::from(p.facts.end[id]),
            (View::Line, 1) => i64::from(p.facts.start_line[id]),
            (View::Child | View::Field, 0) | (View::Tape, 1) => parent(),
            (View::Child, 2) => i64::from(p.idx[l]),
            (View::Field, 1) => i64::from(field_atom(p.tag, p.facts.field[id])),
            (View::Tape, 2) => i64::from(p.size[l]),
            (View::Tape, 3) => i64::from(p.depth[l]),
            (View::Visit, 1) => {
                let dialect = &self.batch.dialects[&p.tag];
                if self.batch.shape == Shape::Core && dialect.family == Family::Python {
                    i64::from(p.python_visit.get_or_init(|| python_order::ranks(p, dialect))[local as usize])
                } else {
                    i64::from(local)
                }
            },
            (View::Depth, 1) => i64::from(p.depth[l]),
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
    let mut named = vec![("node", View::Node), ("child", View::Child), ("field", View::Field), ("tape", View::Tape), ("core_node", View::Core)];
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

/// T's base relations over `documents` (`sources`), each of `tests`' relations, and each of `sets` (a relation and its set's kind atoms):
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
    let (batch, relations, (strings, symbols)) = Batch::build(store, documents, projection, metal)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, batch.nodes.last().copied().unwrap_or(0) as u64);
    drop(built);
    let mut out: Vec<(String, Arc<dyn Source>)> = relations.into_iter().map(|(name, r)| (name.to_owned(), Arc::new(r) as Arc<dyn Source>)).collect();
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
