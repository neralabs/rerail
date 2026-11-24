//! T's base relations served from a batch of core facts (decision 0008; 05 Phase 7 row 3): the
//! relations Typing-support's exporter (`spikes/datalog-engines/exporter/src/main.rs`) wrote as
//! rows, here read off `facts::core` columns when a statement filters them.
//!
//! The rows are T's: over each document's named nodes other than comments, in preorder,
//!
//! ```text
//! doc(d, path, module)   pkg(d, p)   unpackaged(d)   tape_error(d)   node(d, n, kind, start, end_)
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
//! themselves read. A program that includes `core.rerail` reads `core.rerail`'s ([`Shape::Core`], decision 0046, `core_rr`):
//! comments are nodes, `text` is every node's, `op` every anonymous token's, and the source-text, token and literal
//! relations are made. T's `op` is `op_t` in both.

use rerail_tape_format::f16::ZCache;
use crate::code_rule_dsl::facts::core::FactsIn;
use rerail_blocks::vector::Vector;
use super::{Modules, Relation, Source};
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::core::{Facts, NONE, TEXT, kind_atom, kind_code};
use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::Connection;
use crate::code_rule_dsl::logic::front::ast::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};
use cells::{Block, Cells};

pub mod within;
pub mod places;
pub mod values;

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
mod units;
pub mod ranks;
pub mod tokens;
/// A batch's documents and rows, each made when first read (decision 0494).
pub mod made;
mod cells;
#[cfg(any(target_os = "macos", target_os = "linux", windows))]
pub mod device_chunk;
pub mod view_docs;
use guests::GuestRows;

/// Which projection of the tree a batch serves.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Shape {
    /// T's: named nodes without comments, `text` of identifiers, `op` of operator fields.
    #[default]
    T,
    /// `core.rerail`'s (decision 0046).
    Core,
}

/// The projection a run binds: its shape, and the relations its program reads (`None`: all). A relation no rule
/// reads is not made where making it costs (`text` of every node, the tokens); one the program derives is not bound
/// (`derived`).
#[derive(Clone, Debug, Default)]
pub struct Projection {
    pub shape: Shape,
    pub reads: Option<BTreeSet<String>>,
    /// The regexes the program runs over `source` or `src_line` spans (`Field::RxSpans`): their matches'
    /// groups are spans, rows of `rx_span`.
    pub rx_spans: Vec<(crate::code_rule_dsl::logic::front::ast::SpanIn, String)>,
    /// The AST patterns the program matches (`Field::AstPatterns`): rows of `ast_match`.
    pub ast_patterns: Vec<String>,
    /// Source/line spans whose complete text a rule reads, traced through copied relation columns.
    /// None is the unprojected test/export interface; AST subtree text keeps its independent bound.
    pub text_spans: Option<BTreeSet<crate::code_rule_dsl::logic::front::ast::SpanIn>>,
    /// Whether a rule reads a token's text (`first_token(n, t), text(t, "(")`: a span of `first_token`, `last_token`,
    /// `token_before` or `token_after`; `span_text_demand`). A token that is no node has text only where one does.
    pub token_text: bool,
    /// The kinds whose subtrees hold every node the program reads (`node_anchors`): a file's projection is its root
    /// and those subtrees (`Projected::narrowed`). `None` is every node.
    pub node_anchors: Option<BTreeSet<String>>,
    /// The node kinds whose text a rule reads (`logic::plan::text_demand::text_kind_demand`): their nodes' `text` is made at any length. Any
    /// other node's is made up to `core_rr::TEXT_INLINE` bytes, unless it is token-like, or `long_text_any`: some rule
    /// reads the text of a node of any kind.
    pub long_text: BTreeSet<String>,
    pub long_text_any: bool,
    /// The nodes whose `text` a rule reads (`TextDemand::read`): those of these kinds, or in these fields of their
    /// parent. `None` is every node.
    pub text_nodes: Option<TextNodes>,
    /// The nodes whose `op` and `op_at` rows a rule reads (`TextDemand::op`), as `text_nodes` are read (their `values`
    /// none). `None` is every node.
    pub op_nodes: Option<TextNodes>,
    /// The grammars whose guests keep their partial tree when they do not parse (`.on guest_error partial`,
    /// decision 0055 §6); `"*"` is every grammar. Any other broken guest is withheld.
    pub partial: BTreeSet<String>,
    /// In T's shape, `syntax_error` is core.rerail's `syntax_error(d, l, msg)`, CPython's verdict, not T's
    /// `syntax_error(d)`: the program declares that one (a program of core.rerail's base relations, not T's).
    pub verdict: bool,
    /// The relations the program's rules derive: the batch binds none of them, made or viewed (T's programs derive
    /// `docof` from `node`, `rr::lower::projections`; a program may name an output as a base relation is named). A
    /// bound table of that name would be the planner's to drop.
    pub derived: BTreeSet<String>,
    /// `rx_spans` compiled, once for the run's projection, not again for every batch it binds.
    pub compiled_rx: CompiledSpans,
}

/// A projection's `rx_span` patterns compiled: each one's regex and, over a document's source or lines, its block
/// program (`span_prefilter`). Made on the first batch and shared by the projection's clones; kept with the patterns it
/// was made from, so a clone whose patterns differ compiles its own. A run's library loads bind a batch a window of
/// files, and each compiled every pattern again: 3% of a cold batch's main thread.
#[derive(Clone, Default)]
pub struct CompiledSpans(Arc<std::sync::OnceLock<(Vec<(crate::code_rule_dsl::logic::front::ast::SpanIn, String)>, Vec<CompiledSpan>)>>);

/// One `rx_span` pattern, compiled: its regex and its block program, if it has one.
type CompiledSpan = (regex::Regex, Option<Arc<dyn crate::code_rule_dsl::facts::span_prefilter::SpanEnds>>);

impl std::fmt::Debug for CompiledSpans {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CompiledSpans({})", self.0.get().map_or(0, |(_, c)| c.len()))
    }
}

impl CompiledSpans {
    /// `patterns` compiled: the kept ones when they were made from these, else compiled now (and kept if none were).
    fn of(&self, patterns: &[(crate::code_rule_dsl::logic::front::ast::SpanIn, String)]) -> StoreResult<Vec<CompiledSpan>> {
        if let Some((from, compiled)) = self.0.get()
            && from.as_slice() == patterns
        {
            return Ok(compiled.clone());
        }
        let compiled = compile_spans(patterns)?;
        let _ = self.0.set((patterns.to_vec(), compiled.clone()));
        Ok(compiled)
    }
}

/// Compiles `rx_span`'s patterns, each on the pool's threads, a class of every character first written as any
/// character (`unfolded`). Over a document's source or its lines, each with its block program (decision 0081 §3): the
/// bytes a match can end after, chunk at a time, so the regex runs only where one can.
fn compile_spans(patterns: &[(crate::code_rule_dsl::logic::front::ast::SpanIn, String)]) -> StoreResult<Vec<CompiledSpan>> {
    use crate::code_rule_dsl::logic::front::ast::SpanIn;
    use rayon::prelude::*;
    let on = std::env::var("RERAIL_SPAN_PREFILTER").map_or(true, |v| v != "off");
    crate::code_rule_dsl::logic::profile::on_cores(|| {
        patterns
            .par_iter()
            .map(|(within, re)| {
                let compiled = crate::code_rule_dsl::facts::span_prefilter::unfolded(re);
                let prefilter = match within {
                    SpanIn::Source | SpanIn::Line if on => crate::code_rule_dsl::facts::span_prefilter::compile(&compiled, *within == SpanIn::Line),
                    _ => None,
                };
                regex::Regex::new(&compiled).map(|r| (r, prefilter)).map_err(|e| StoreError::Invalid(format!("rx_span: bad regex {re:?}: {e}")))
            })
            .collect()
    })
}

impl Projection {
    pub fn core() -> Self {
        Projection { shape: Shape::Core, reads: None, rx_spans: Vec::new(), ast_patterns: Vec::new(), text_spans: None, token_text: true, node_anchors: None, long_text: BTreeSet::new(), long_text_any: false, text_nodes: None, op_nodes: None, partial: BTreeSet::new(), verdict: false, derived: BTreeSet::new(), compiled_rx: CompiledSpans::default() }
    }
    fn wants(&self, rel: &str) -> bool {
        self.reads.as_ref().is_none_or(|r| r.contains(rel))
    }
    /// A digest of what the projection makes a base relation's rows of, beside the document: its shape, patterns,
    /// text bounds, narrowed nodes and guest policy. Not which relations it makes (`reads`, `derived`): a relation
    /// made has the same rows whichever others are. A memo entry's rows are a relation's under its identity only
    /// with this in it (decision 0225 §2): two programs with the same rules over different projections, a broken
    /// guest kept (`.on guest_error partial`) or withheld, read different rows of `parsed`.
    pub fn rows_digest(&self) -> [u8; 32] {
        use sha2::{Digest as _, Sha256};
        let mut h = Sha256::new();
        h.update(b"rerail.projection-rows/v1\0");
        h.update(format!("{:?}\0{:?}\0{:?}\0{:?}\0{}\0{:?}\0{:?}\0{}\0{:?}\0{:?}\0{}", self.shape, self.rx_spans, self.ast_patterns, self.text_spans, self.token_text, self.node_anchors, self.long_text, self.long_text_any, self.text_nodes, self.partial, self.verdict).as_bytes());
        h.update(format!("\0{:?}", self.op_nodes).as_bytes());
        h.finalize().into()
    }
    /// Whether a guest of `grammar` that does not parse keeps its partial tree.
    fn keeps_partial(&self, grammar: &str) -> bool {
        self.partial.contains("*") || self.partial.contains(grammar)
    }

    /// The projection of a program from what it reads of the program, found by its caller over its version
    /// (`logic::plan::text_demand::projection_of`).
    pub fn of_parts(parts: ProjectionParts, reads: BTreeSet<String>) -> Self {
        let ProjectionParts { spans, token_text, core_shape, partial, error_triple, derived, rx_spans, ast_patterns } = parts;
        crate::code_rule_dsl::logic::profile::decision("text of tokens", || if token_text { "made (a rule reads a token's text)".into() } else { "none (no rule reads a token's text)".into() });
        let shape = if core_shape { Shape::Core } else { Shape::T };
        let verdict = shape == Shape::T && error_triple;
        let reads = reads.into_iter().filter(|r| !derived.contains(r)).collect();
        Projection { shape, reads: Some(reads), rx_spans, ast_patterns, text_spans: Some(spans), token_text, node_anchors: None, long_text: BTreeSet::new(), long_text_any: false, text_nodes: None, op_nodes: None, partial, verdict, derived, compiled_rx: CompiledSpans::default() }
    }

    /// This projection, with `text` made at any length for the node kinds whose text the program's rules read
    /// (`demand`), and its phase's node anchors (`anchors`): found by its caller over its version
    /// (`logic::plan::text_demand::projection`).
    pub fn with_long_text_parts(mut self, demand: TextDemand, anchors: Result<BTreeSet<String>, String>) -> Self {
        let TextDemand { long, any, negated_any, read, wide, op, op_wide } = demand;
        self.long_text = long;
        self.long_text_any = any.len() + negated_any > 0;
        crate::code_rule_dsl::logic::profile::decision("text past the inline bound", || {
            if self.long_text_any { format!("every node ({} rules read the text of a node of any kind)", any.len() + negated_any) } else { format!("{} kinds: {}", self.long_text.len(), self.long_text.iter().take(12).cloned().collect::<Vec<_>>().join(", ")) }
        });
        crate::code_rule_dsl::logic::profile::decision("nodes with text", || match &read {
            None => format!("every node ({} reads the text of a node it does not narrow)", wide.as_deref().unwrap_or("a rule")),
            Some(TextNodes { kinds, fields, parents, parent_fields, values }) => {
                let some = |s: &BTreeSet<String>| s.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
                format!("{} kinds ({}), the nodes in {} fields ({}), the children of {} kinds ({}) and of the nodes in {} fields ({}), and those whose text is one of {} constants ({})", kinds.len(), some(kinds), fields.len(), some(fields), parents.len(), some(parents), parent_fields.len(), some(parent_fields), values.len(), some(values))
            }
        });
        if !std::env::var("RERAIL_TEXT_NODES").is_ok_and(|v| v == "all") {
            self.text_nodes = read;
        }
        crate::code_rule_dsl::logic::profile::decision("nodes with op rows", || match &op {
            None => format!("every node ({} reads the ops of a node it does not narrow)", op_wide.as_deref().unwrap_or("a rule")),
            Some(TextNodes { kinds, fields, parents, parent_fields, .. }) => {
                let some = |s: &BTreeSet<String>| s.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
                format!("{} kinds ({}), the nodes in {} fields ({}), the children of {} kinds ({}) and of the nodes in {} fields ({})", kinds.len(), some(kinds), fields.len(), some(fields), parents.len(), some(parents), parent_fields.len(), some(parent_fields))
            }
        });
        // `RERAIL_OP_NODES=all`: every node's, to compare.
        if !std::env::var("RERAIL_OP_NODES").is_ok_and(|v| v == "all") {
            self.op_nodes = op;
        }
        crate::code_rule_dsl::logic::profile::decision("nodes projected", || match &anchors {
            Ok(kinds) => format!("the root and the subtrees of {} kinds: {}", kinds.len(), kinds.iter().cloned().collect::<Vec<_>>().join(", ")),
            Err(why) => format!("every node ({why})"),
        });
        self.node_anchors = anchors.ok();
        self
    }

    /// The columns the batch binds base relation `rel` with (`select::input_columns`'), but T's `syntax_error(d)`
    /// where the program declares that one and not core.rerail's verdict (`verdict`): the cross-file part binds a base
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

/// The nodes whose text a rule reads: those of `kinds`, those in a field of `fields` of their parent, the children of a
/// node of `parents`' kinds or in one of `parent_fields`, and those whose text is one of `values`: a rule that only compares the text of a node it
/// does not narrow with a constant (`text(v, "Literal")`) needs no other node's text, nor one that reads a child of a
/// node it narrows (`kind(n, "dotted_name"), child(n, c, 0), text(c, t)`) any but those children's. Other texts are
/// then not made, nor interned: every statement's, comment's and docstring's was half a file's atoms.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextNodes {
    pub kinds: BTreeSet<String>,
    pub fields: BTreeSet<String>,
    pub parents: BTreeSet<String>,
    pub parent_fields: BTreeSet<String>,
    pub values: BTreeSet<String>,
}

impl TextNodes {
    /// Whether node `id` of `f` is one of these: of a kind read, in a field read, a child of a kind read or of a node in
    /// a field read, or with a text a rule compares it with.
    /// `id` is the node's tape node (`Projected::node`), which the caller's walk holds; `f` its facts.
    pub fn reads(&self, f: FactsIn<'_>, id: u32, dialect: &Dialect) -> bool {
        self.kinds.contains(dialect.name(f, id))
            || dialect.catalog.field_name(f.field(id)).is_some_and(|name| self.fields.contains(name))
            || !(self.parents.is_empty() && self.parent_fields.is_empty())
                && f.parent(id).is_some_and(|q| {
                    self.parents.contains(dialect.name(f, q))
                        || dialect.catalog.field_name(f.field(q)).is_some_and(|name| self.parent_fields.contains(name))
                })
            || !self.values.is_empty() && self.values.contains(f.text(id))
    }
}

/// `TextNodes` resolved to one dialect's codes (`TextNodes::of_dialect`): whether each kind code and field code is
/// one read, so a walk's test of a node is table reads, not a search of the sets' names per node (its kind's, its
/// field's and its parent's, for every node of every document a phase reads the ops or text of).
pub struct NodesOf {
    tag: u8,
    kinds: Vec<bool>,
    fields: Vec<bool>,
    parents: Vec<bool>,
    parent_fields: Vec<bool>,
}

impl TextNodes {
    /// These nodes as `dialect`'s codes.
    pub fn of_dialect(&self, dialect: &Dialect) -> NodesOf {
        let kinds = |names: &BTreeSet<String>| (0..dialect.catalog.kind_count()).map(|k| names.contains(dialect.code_name(k as u16))).collect::<Vec<bool>>();
        let fields = |names: &BTreeSet<String>| (0..=dialect.catalog.field_count()).map(|k| dialect.catalog.field_name(k as u16).is_some_and(|n| names.contains(n))).collect::<Vec<bool>>();
        NodesOf { tag: dialect.tag, kinds: kinds(&self.kinds), fields: fields(&self.fields), parents: kinds(&self.parents), parent_fields: fields(&self.parent_fields) }
    }
}

impl NodesOf {
    /// Whether these are dialect `tag`'s.
    pub fn is_of(&self, tag: u8) -> bool {
        self.tag == tag
    }

    /// `nodes.reads(f, id, dialect)`, by the codes (a code past the tables, ERROR's, by its name).
    pub fn reads(&self, nodes: &TextNodes, f: FactsIn<'_>, id: u32, dialect: &Dialect) -> bool {
        let kind_in = |set: &[bool], names: &BTreeSet<String>, x: u32| {
            let code = dialect.code(f, x);
            set.get(usize::from(code)).copied().unwrap_or_else(|| names.contains(dialect.code_name(code)))
        };
        let field_in = |set: &[bool], names: &BTreeSet<String>, x: u32| {
            let code = f.field(x);
            set.get(usize::from(code)).copied().unwrap_or_else(|| dialect.catalog.field_name(code).is_some_and(|name| names.contains(name)))
        };
        kind_in(&self.kinds, &nodes.kinds, id)
            || field_in(&self.fields, &nodes.fields, id)
            || !(nodes.parents.is_empty() && nodes.parent_fields.is_empty())
                && f.parent(id).is_some_and(|q| kind_in(&self.parents, &nodes.parents, q) || field_in(&self.parent_fields, &nodes.parent_fields, q))
            || !nodes.values.is_empty() && nodes.values.contains(f.text(id))
    }
}

/// What the rules read of `text` (`logic::plan::text_demand::text_kind_demand`).
#[derive(Debug, PartialEq)]
pub struct TextDemand {
    /// The kinds whose text is read at any length.
    pub long: BTreeSet<String>,
    /// The positive text atoms (rule, body position) of a node of any kind, no twin reading its long text.
    pub any: Vec<(usize, usize)>,
    /// The negated text atoms of a node of any kind that is not compared with a constant within the bound: no twin
    /// stands in for a negation, so every node's text is made at any length.
    pub negated_any: usize,
    /// The nodes whose text (within the bound) is read; `None` every node.
    pub read: Option<TextNodes>,
    /// Where `read` is every node: the first rule that reads the text of a node it does not narrow.
    pub wide: Option<String>,
    /// The nodes whose `op` or `op_at` a rule reads, narrowed as `read` is; `None` every node.
    pub op: Option<TextNodes>,
    /// Where `op` is every node: a rule that reads the ops of a node it does not narrow.
    pub op_wide: Option<String>,
}

/// What `Projection::of_parts` reads of a program: the spans whose text a rule reads and whether a token's, whether the
/// program declares every relation `core.rerail` does (core.rerail's shape), the grammars whose errors keep a partial tree,
/// whether `syntax_error` has three columns, the derived relations, and the regex spans and AST patterns.
pub struct ProjectionParts {
    pub spans: BTreeSet<crate::code_rule_dsl::logic::front::ast::SpanIn>,
    pub token_text: bool,
    pub core_shape: bool,
    pub partial: BTreeSet<String>,
    pub error_triple: bool,
    pub derived: BTreeSet<String>,
    pub rx_spans: Vec<(crate::code_rule_dsl::logic::front::ast::SpanIn, String)>,
    pub ast_patterns: Vec<String>,
}

/// `rr_long_text_node(n)`: the batch's nodes whose text is past `text`'s inline bound, with the batch that holds
/// their documents. A node's text or value (`@rr_node_text`, and decision 0174's answers `@rr_node_text_any` and
/// `@rr_node_value`) is read through the run's own binding of this relation (`functions::node_read`): the native
/// engine's `func` step and SQL's functions find it on the run's connection, so a text is read from the batch the run
/// reads, never from a process-wide or per-thread table of batches by document key.
pub struct LongTexts {
    nodes: Arc<dyn Source>,
    batch: Arc<Batch>,
    /// Each document key's place in `batch.docs`.
    docs: HashMap<u32, usize>,
    /// Its rows, counted on the tape (`statistics`).
    counted: std::sync::OnceLock<usize>,
}

impl LongTexts {
    fn new(nodes: Arc<dyn Source>, batch: &Arc<Batch>) -> Self {
        let docs = batch.docs.keys().iter().enumerate().map(|(i, &k)| (k, i)).collect();
        Self { nodes, batch: Arc::clone(batch), docs, counted: std::sync::OnceLock::new() }
    }

    /// How many rows it has, counted on the tape as the batch's rows are made (each node of a document of a grammar
    /// whose text is past the inline bound), with no row made: its group's rows are made only where they are read.
    fn tape_len(&self) -> usize {
        *self.counted.get_or_init(|| {
            (0..self.batch.docs.len())
                .map(|i| {
                    let p = &self.batch.docs[i];
                    if p.tag == TEXT || self.batch.dialects.get(p.tag).is_none() {
                        return 0;
                    }
                    p.node_ids().filter(|&(_, id, _)| (p.f().end(id) - p.f().start(id)) as usize > core_rr::TEXT_INLINE).count()
                })
                .sum()
        })
    }

    /// Node `n`'s text when it is past `text`'s inline bound (`core_rr::TEXT_INLINE`); none for a shorter node
    /// (whose text `text` holds), a span, or a node of a document not in the batch. Only the nodes a rule binds are
    /// read and interned (decision 0108 §1).
    pub fn long_text(&self, n: i64) -> Option<String> {
        let (key, low) = ((n >> 32) as u32, n as u32);
        let p = &self.batch.docs[*self.docs.get(&key)?];
        let local = p.local_of(low)?;
        let core = p.ranks().select(Item::Node, local)?;
        let text = p.f().text(core);
        (text.len() > core_rr::TEXT_INLINE).then(|| text.to_owned())
    }

    /// Node `n`'s exact source text at any length, or a comment span's (a comment no node is: `Extra`'s `node_text`
    /// rows); none for another span or a node of a document not in the batch. It answers `node_text`'s requests
    /// (decision 0174): only the nodes a rule asks for are read and interned.
    pub fn text_any(&self, n: i64) -> Option<String> {
        let (key, low) = ((n >> 32) as u32, n as u32);
        let p = &self.batch.docs[*self.docs.get(&key)?];
        let f = p.f();
        if let Some(k) = granules::comment_of(low) {
            // The k-th comment kind's node not inside another that no node is, as `Extra::nodes` numbers them.
            use crate::code_rule_dsl::language::is_comment_kind;
            let dialect = self.batch.dialects.get(p.tag)?;
            let mut j = 0;
            for (id, up) in f.with_parents(0, f.len() as u32) {
                if !f.named(id) || !is_comment_kind(dialect.name(f, id)) || up.is_some_and(|q| is_comment_kind(dialect.name(f, q))) {
                    continue;
                }
                if p.local_of_node(id) == NONE {
                    j += 1;
                    if j == k {
                        return Some(f.text(id).to_owned());
                    }
                }
            }
            return None;
        }
        let core = p.ranks().select(Item::Node, p.local_of(low)?)?;
        Some(f.text(core).to_owned())
    }

    /// Node `n`'s value as W reads it (`value(n, v)`, the plain reading), read off its document's tape: the
    /// document's values are read at its first request, and only those asked for are interned. None for a node with no
    /// value, a document of a language W reads no values of (its literals are `rr_value_lit`'s), or one not in the
    /// batch. It answers `value`'s requests (decision 0174).
    pub fn value_any(&self, n: i64) -> Option<String> {
        let (key, low) = ((n >> 32) as u32, n as u32);
        let i = *self.docs.get(&key)?;
        let p = &self.batch.docs[i];
        let valued = self.batch.valued.get(i)?.as_ref()?;
        let local = p.local_of(low)?;
        let values = valued.values.get_or_init(|| {
            let mut values = HashMap::new();
            if let (Some(reader), Some((tape, file))) = (crate::code_rule_dsl::facts::core::value_reader(), &self.batch.docs.input(i).tape) {
                reader.values(&in_batch(tape, self.batch.docs.cache()), file.as_ref(), true, false, &mut |followed, ordinal, v, _| {
                    let local = p.local_of_node(ordinal);
                    if !followed && local != NONE && let Some(v) = v {
                        values.insert(local, v);
                    }
                });
            }
            values
        });
        values.get(&local).cloned()
    }

    /// Node `n`'s string literal's content, as the batch's shape states `str(n, s)`: `core.rerail`'s literal reading
    /// (`core_rr::node_str`), or T's (a string node's content between its quotes, `NodeRows`). It answers `str`'s
    /// requests (decisions 0174, 0353 step 3): only the nodes a rule asks for are read and interned.
    pub fn str_any(&self, n: i64) -> Option<String> {
        let (key, low) = ((n >> 32) as u32, n as u32);
        let p = &self.batch.docs[*self.docs.get(&key)?];
        let local = p.local_of(low)?;
        let x = p.ranks().select(Item::Node, local)?;
        let dialect = self.batch.dialects.get(p.tag)?;
        let f = p.f();
        match self.batch.shape {
            Shape::Core => core_rr::node_str(f, dialect, x),
            Shape::T => match dialect.class(f, x) {
                Class::String => dialect.string(f, x).and_then(|(_, content)| content.map(str::to_owned)),
                _ => None,
            },
        }
    }
}

impl Source for LongTexts {
    fn syntax_batch(&self) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        Some(Arc::clone(&self.batch) as _)
    }
    fn node_read(&self, name: &str, n: i64) -> Option<String> {
        match name {
            "rr_node_text" => self.long_text(n),
            "rr_node_text_any" => self.text_any(n),
            "rr_node_value" => self.value_any(n),
            "rr_node_str" => self.str_any(n),
            _ => None,
        }
    }
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.nodes.rows_of_documents(docs)
    }
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn revision(&self) -> Option<u64> {
        self.nodes.revision()
    }
    fn columns(&self) -> Vec<String> {
        self.nodes.columns()
    }
    fn len(&self) -> usize {
        self.nodes.len()
    }
    /// Its rows, made of none (`Source::estimated_len`'s default is `len`, which makes them): counted on the tape, or
    /// for a batch whose documents' counts were given (`Batch::lazy`), the counts' estimate, which projects none. The
    /// relation's own estimate makes every document's rows where no count was given (`MadeRows::estimated_len`): the
    /// planner's measure made the whole group that way (36 MB of node rows at the M3's 300-file peak, ab160).
    fn estimated_len(&self) -> usize {
        if self.batch.lazy() { self.nodes.estimated_len() } else { self.tape_len() }
    }
    /// Its one column's statistics, of its rows counted on the tape (decision 0353): one row a node, so each of its
    /// counts is its rows. Measured as rows (`sampled_statistics_unanswered`), the measure made its whole group's
    /// rows, every relation's of every document's (the M3's 300-file peak held 77 MB of them, and a 158 MB block).
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        // The made rows' own, once made; before, counted on the tape (the nodes' sampled statistics make documents).
        let made = self.nodes.as_any().and_then(|a| a.downcast_ref::<made::MadeRows>()).is_none_or(made::MadeRows::is_done);
        if made && let Some(known) = self.nodes.statistics() {
            return Some(known);
        }
        let n = self.estimated_len() as i64;
        Some(self.nodes.columns().into_iter().map(|c| (c, n, n, n, n)).collect())
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.nodes.estimated_matches(column, batch)
    }
    fn indexed(&self, column: usize) -> bool {
        self.nodes.indexed(column)
    }
    fn range_indexed(&self, column: usize) -> bool {
        self.nodes.range_indexed(column)
    }
    fn column_counts(&self, c: usize) -> Option<Vec<(i64, u64)>> {
        self.nodes.column_counts(c)
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.nodes.range_lookup(column, lower, upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.nodes.lookup(column, values)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.nodes.value(row, column)
    }
}

/// The batch's relation `name` as a source: `rr_long_text_node` with the batch (`LongTexts`), any other as rows.
/// How a batch's relations are made of their rows: written into a block (`Relation::from_rows`), or staged
/// (`Relation::staged`).
pub(super) type Make = fn(&[&str], &mut dyn Iterator<Item = Vector<i64>>) -> Relation;

fn as_source(name: &str, rows: Arc<dyn Source>, batch: &Arc<Batch>) -> Arc<dyn Source> {
    if name == LONG_TEXT_NODES { Arc::new(LongTexts::new(rows, batch)) } else { rows }
}

/// The batch's relations, each over its groups (`made::MadeRows`, decision 0494), but those of the batch's own the
/// program derives (`Projection::derived`): each name, whether it is one of the batch's own, and its source.
pub fn made_sources(batch: &Arc<Batch>) -> Vec<(&str, Option<&'static str>, Arc<dyn Source>)> {
    let rows = &batch.rows;
    rows.names
        .iter()
        .enumerate()
        .filter(|(rel, name)| *rel >= rows.relations || !batch.projection.derived.contains(name.as_str()))
        .map(|(rel, name)| (name.as_str(), rows.statics.get(rel).copied(), as_source(name, Arc::new(made::MadeRows { batch: Arc::clone(batch), rel, sampled: OnceLock::new() }), batch)))
        .collect()
}

// The node reads' relation and functions are the virtual tables' (`vtab.rs`), which the string functions read.
pub use super::{LONG_TEXT_NODES, NODE_READS};

/// A source's syntax batch (`Source::syntax_batch`) as the syntax table's `Batch`.
pub trait PythonBatch {
    fn python_batch(&self) -> Option<Arc<Batch>>;
}

impl<S: Source + ?Sized> PythonBatch for S {
    fn python_batch(&self) -> Option<Arc<Batch>> {
        self.syntax_batch()?.downcast::<Batch>().ok()
    }
}

/// The request relation of `value` (decision 0174): a batch reads W's values off its documents' tapes where a program
/// derives it.
pub const VALUE_REQ: &str = "rr_value_req";

/// The values W reads off a document's tape (`ValueReader`), read at the first request.
pub(crate) struct Valued {
    /// Read off its document's tape at the first request (`value_any`): the document is made by then (its node's
    /// projection is read), and a pending one is not made for it before (Q332).
    values: OnceLock<HashMap<u32, String>>,
}

/// The relation a program's guest settings are facts of (`.on guest_error`; decision 0055; `.guests` is retired, 0523):
/// `guest_policy("errors" | "errors:GRAMMAR", "skip" | "partial")`.
pub const GUEST_POLICY: &str = "guest_policy";

/// The first guest document's key. A key's range is what the document is to the run, by role and grammar, never by
/// language (decisions 0058 §1, 0512): files take `1..GUEST_KEYS` (`corpus::memo::doc_keys`), a file read for its
/// programs among them (its text); code guests in the run's language (programs) `GUEST_KEYS..HIDDEN_KEYS`; and every
/// other document (a guest in another grammar, a file's own tree in another grammar, a value: a string a rule parses,
/// Python's string annotations among them) `HIDDEN_KEYS..2^31`. What a rule sees is a bound on these keys: a rule that
/// does not name a guest sees files and code guests. A value's rules are a context's (decision 0597), over its roots.
pub const GUEST_KEYS: u32 = 1 << 30;
pub const HIDDEN_KEYS: u32 = (1 << 30) + (1 << 29);

/// A guest document's place (decision 0055): the host document and the node or span it stands at, its grammar and
/// role, its root, and how its bytes map to the host's.
#[derive(Clone, Debug)]
pub struct Guest {
    pub host: u32,
    /// Its file's key (decision 0431 §4): a program's file (its host's, through a host that is a program too), whose
    /// `docof`, lines and findings are its own; 0 for a tape value or a value, which are no file's.
    pub file: u32,
    /// `at`: the host's node or span (`host << 32 | local`).
    pub at: i64,
    pub grammar: String,
    /// "program" (a file's) or "fragment" (a string's).
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

impl Guest {
    /// The key `node`'s and `docof`'s document column gives a node of document `key` that is this guest (decision
    /// 0431 §4): a program's file, in either shape; any other guest's own key, for it is no file's.
    pub fn file_key(guest: Option<GuestOf<'_>>, key: u32) -> u32 {
        guest.filter(|g| matches!(g.made, Made::Host | Made::File) && g.file != 0).map_or(key, |g| g.file)
    }

    /// What its batch reads of it before its document is made (`GuestOf`).
    pub fn of(&self) -> GuestOf<'_> {
        GuestOf { host: self.host, file: self.file, made: &self.made }
    }
}

/// A guest as its batch reads it before its document is made (`Input::guest`): its host, its file (`Guest::file`) and
/// what made it. Its place is its document's, read once it is made.
#[derive(Clone, Copy, Debug)]
pub struct GuestOf<'a> {
    pub host: u32,
    pub file: u32,
    pub made: &'a Made,
}

/// A pending guest's `GuestOf`, given ahead (`Pending::with_guest`): a host file's tree (`Made::File`) or one of its
/// programs (`Made::Host`), as the documents' table keeps them (`retain::BaseCount::guest`, decision 0512).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hosted {
    pub host: u32,
    pub file: u32,
    pub made: Made,
}

impl Hosted {
    pub fn of(&self) -> GuestOf<'_> {
        GuestOf { host: self.host, file: self.file, made: &self.made }
    }
}

/// What made a guest document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Made {
    /// The code at `at` in its host (`embed`, `parsed`, `sub_tape`).
    Host,
    /// A file's own tree, where the run reads the file for its programs (decision 0512): the file is a text document
    /// under its own key, shown, and this, its tree in its grammar, a guest of it under a hidden key, standing at its
    /// whole text. It is no program: no `rr_program`, `sub_tape` or `parsed` row. Its programs are guests of it.
    File,
    /// `@parse(grammar, text)`: a value document, with no host; its places are its own text's.
    Parse { text: String },
    /// `@splice(tape, node, with)`: tape `tape` with node `node`'s text replaced by tape `with`'s, parsed in its
    /// grammar; a value document.
    Splice { tape: String, node: i64, with: String },
    /// A string a rule parses (`parsed`, `sub_tape`, `embed_value`): a tape value (decision 0431). One per grammar
    /// and text, whatever holds it; `places` are the strings it was parsed at. Its nodes are its tape's: their
    /// positions are in its own text, and it is no file's (`doc` has no row for it), so a file's place of one of
    /// its nodes is reached through the string.
    Tape { places: Vec<TapePlace> },
}

/// A string a tape value was parsed at (decision 0431): its host document, its node, and where the value's bytes
/// stand in the host (for `host_node`). Two places are one when they are the same node.
#[derive(Clone, Debug)]
pub struct TapePlace {
    pub host: u32,
    pub at: i64,
    pub place: Placement,
}

impl PartialEq for TapePlace {
    fn eq(&self, o: &Self) -> bool {
        (self.host, self.at) == (o.host, o.at)
    }
}
impl Eq for TapePlace {}

/// A document's text as a rule gets it back (`@unparse`): its root's text where its root is not its first node, else
/// the whole text (a fragment's tape is rooted at its start node, decision 0466).
/// `f` is its facts, read in its batch (`FactsIn`).
pub fn unparsed<'a>(f: FactsIn<'a>, d: &Document) -> &'a str {
    match d.guest.as_deref() {
        Some(g) if g.root != 0 => f.text(g.root),
        _ => f.facts.source.as_str(),
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
    /// A program an embedder wrote (`embed::Embedder`: a Vue component's view, a Blade view's PHP): copied stretches
    /// and written tokens, each standing for the host bytes its segment names, in any order (`SegmentMap`, decision
    /// 0222). A node is placed where its first segment stands (`FileRanges::primary`, as main reports one) and lies
    /// in the host inside the least range holding all it stands for (`SegmentMap::bounds`, which `host_node` reads).
    Segments(Arc<crate::code_rule_dsl::embed::segments::SegmentMap>),
}

impl Placement {
    pub fn host(&self, [a, b]: [u32; 2]) -> [u32; 2] {
        let clamp = |x: i64, [lo, hi]: [u32; 2]| x.clamp(i64::from(lo), i64::from(hi)) as u32;
        match self {
            Placement::Copied { delta, within } => [clamp(i64::from(a) + delta, *within), clamp(i64::from(b) + delta, *within)],
            Placement::Mapped { pieces, within } => {
                let start = clamp(mapped_start(pieces, a), *within);
                if b <= a { [start, start] } else { [start, clamp(mapped_end(pieces, b), *within).max(start)] }
            }
            Placement::At(at) => *at,
            Placement::Segments(map) => {
                let r = map.file_ranges(a as usize..b.max(a) as usize).primary();
                [r.start as u32, r.end as u32]
            }
        }
    }

    /// The least host range holding every byte the guest bytes `[a, b]` stand for: where a node lies in its host
    /// (`host_node`). The bytes `host` gives, but for an embedder's program, whose node may stand for host ranges
    /// apart (a `v-for` arrow: its element and its alias).
    pub(crate) fn bounds(&self, [a, b]: [u32; 2]) -> [u32; 2] {
        match self {
            Placement::Segments(map) => {
                let r = map.bounds(a as usize..b.max(a) as usize);
                [r.start as u32, r.end as u32]
            }
            _ => self.host([a, b]),
        }
    }

    /// This placement's stretches: a copied stretch is one, unbounded.
    fn pieces(&self) -> Option<Vec<normalized::Piece>> {
        match self {
            Placement::Copied { delta, .. } => Some(vec![normalized::Piece { d: 0, s: *delta, escape: false }]),
            Placement::Mapped { pieces, .. } => Some(pieces.to_vec()),
            Placement::At(_) | Placement::Segments(_) => None,
        }
    }

    /// `inner`, a placement in this one's text, as a placement in this one's host's.
    pub fn then(&self, inner: Placement) -> Placement {
        match (self, &inner) {
            (Placement::At(at), _) => Placement::At(*at),
            (_, Placement::At(at)) => Placement::At(self.host(*at)),
            // A document inside an embedder's program, or a program inside a guest, stands at the bytes it lies in
            // as a whole: segments do not compose with a string's map (open in decision 0222).
            (Placement::Segments(_), Placement::Copied { within, .. } | Placement::Mapped { within, .. }) => Placement::At(self.host(*within)),
            (_, Placement::Segments(map)) => {
                let r = map.bounds(0..u32::MAX as usize);
                Placement::At(self.host([r.start as u32, r.end as u32]))
            }
            (Placement::Copied { delta, .. }, Placement::Copied { delta: d, within }) => Placement::Copied { delta: delta + d, within: self.host(*within) },
            _ => {
                let (Some(outer), Some(pieces), Placement::Copied { within, .. } | Placement::Mapped { within, .. }) = (self.pieces(), inner.pieces(), &inner) else {
                    unreachable!("At is matched above")
                };
                // Cut where either map cuts: the inner's stretches, and the outer's, pulled back through a copied
                // inner stretch.
                let mut cuts: Vector<u32> = std::iter::once(0).chain(pieces.iter().map(|p| p.d)).collect();
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
    /// A Python file no regular package holds: one of the root, or whose folder has no `__init__.py` (core.rerail's
    /// `unpackaged`, `corpus::package_root_modules`).
    pub unpackaged: bool,
}

/// A document's facts' size: its nodes and its kinds' codes (`Facts::len`, `Facts::code_count`), what its projection
/// takes of its batch's block (`Projected::bytes_of`). The documents' table keeps it (`retain::BaseCounts`), so a
/// document made when first read (`Pending`) has its place in the block before it is made.
pub type FactsSize = [u32; 2];

/// `facts`' size.
pub fn facts_size(facts: &Facts) -> FactsSize {
    // A code count is the tape's table's, read without its nodes.
    [u32::try_from(facts.len()).unwrap_or(u32::MAX), u32::try_from(facts.read_in(None).code_count()).unwrap_or(u32::MAX)]
}

/// What a batch takes of its block for a document of facts of size `size`: its projection's arrays (`Projected::bytes_of`)
/// and the chunk's copies of the facts' arrays, 22 bytes a node (`Batch::add`).
fn block_bytes(size: FactsSize) -> usize {
    Projected::bytes_of(size) + 22 * size[0] as usize + 4096
}

/// A document made when it is first read (Q332), as decision 0494 makes a document's rows when first read: a memo
/// hit's, whose key, path, tag, digest and facts' size the documents' table keeps (`retain::BaseCounts`, of the
/// content the file has now). Its facts and tape are built by `make` (from its kept tape, or parsed) on its first read,
/// once, whichever batch reads it first; it is shared with the batches it grows into and the session's next run.
/// A document made whose key or digest is not the table's fails its batch, named (`Docs::input`). A host file's tree
/// (decision 0512) and its programs are pending too, each one's host and file given ahead (`with_guest`), so the batch
/// knows each host's guests before any is made (`Batch::guests_of`).
pub struct Pending {
    key: u32,
    path: String,
    tag: u8,
    digest: [u8; 32],
    size: FactsSize,
    /// Its module as the corpus names it (`Document::module`), which a read of `doc` by module routes by.
    module: Option<Arc<str>>,
    /// Its host and file, a guest's: a host file's tree (`Made::File`) or one of its programs (`Made::Host`).
    guest: Option<Hosted>,
    make: Box<dyn Fn() -> Result<Document, String> + Send + Sync>,
    made: OnceLock<Document>,
    failed: OnceLock<String>,
}

impl Pending {
    pub fn new(key: u32, path: String, tag: u8, digest: [u8; 32], size: FactsSize, module: Option<Arc<str>>, make: Box<dyn Fn() -> Result<Document, String> + Send + Sync>) -> Self {
        Self { key, path, tag, digest, size, module, guest: None, make, made: OnceLock::new(), failed: OnceLock::new() }
    }

    /// The same, a guest of `guest`'s host and file: what it is made as must be.
    pub fn with_guest(mut self, guest: Hosted) -> Self {
        self.guest = Some(guest);
        self
    }

    /// Whether `made`, a guest as made, is of the host and file given ahead (`with_guest`).
    fn stands_as_given(&self, made: Option<&Guest>) -> bool {
        match (made, self.guest.as_ref()) {
            (None, None) => true,
            (Some(m), Some(g)) => m.host == g.host && m.file == g.file && m.made == g.made,
            _ => false,
        }
    }

    /// The document, made if it is not. One that fails to be made is a text with no node, its failure kept
    /// (`failure`): the batch reading it fails.
    fn document(&self) -> &Document {
        self.made.get_or_init(|| {
            if std::env::var_os("RERAIL_PARSE_TRACE").is_some() {
                eprintln!("RERAIL_PARSE_TRACE pending {}:\n{}", self.path, std::backtrace::Backtrace::force_capture());
            }
            let made = (self.make)().and_then(|d| {
                if d.key == self.key && d.digest == self.digest && self.stands_as_given(d.guest.as_deref()) && d.module == self.module {
                    Ok(d)
                } else {
                    Err(format!("{}: made when first read, it is not the document the documents' table keeps (key {} for {}, its digest another, Q332)", self.path, d.key, self.key))
                }
            });
            made.unwrap_or_else(|why| {
                let _ = self.failed.set(why);
                let facts = Arc::new(crate::code_rule_dsl::facts::core::text(Arc::new(String::new())));
                Document { key: self.key, path: self.path.clone(), core: format!("/c#core:{}", self.path), digest: self.digest, facts, tape: None, guest: None, module: None, unpackaged: false }
            })
        })
    }
}

/// One of a batch's documents as it is given: made, or made when first read (`Pending`).
#[derive(Clone)]
pub enum Input {
    // Boxed: a document is large beside a pending one, and most of a batch is one kind or the other.
    Made(Box<Document>),
    Pending(Arc<Pending>),
}

impl From<Document> for Input {
    fn from(d: Document) -> Self {
        Input::Made(Box::new(d))
    }
}

impl Input {
    pub fn key(&self) -> u32 {
        match self {
            Input::Made(d) => d.key,
            Input::Pending(p) => p.key,
        }
    }
    pub fn path(&self) -> &str {
        match self {
            Input::Made(d) => &d.path,
            Input::Pending(p) => &p.path,
        }
    }
    /// Its digest (`Document::digest`): a pending one's the table's, which its facts are checked against when made.
    pub fn digest(&self) -> [u8; 32] {
        match self {
            Input::Made(d) => d.digest,
            Input::Pending(p) => p.digest,
        }
    }
    /// Its tape tag (`Facts::tag`).
    pub fn tag(&self) -> u8 {
        match self {
            Input::Made(d) => crate::code_rule_dsl::facts::core::facts_tag(&d.facts),
            Input::Pending(p) => p.tag,
        }
    }
    /// Its module as the corpus names it (`Document::module`); none is made for it.
    pub fn module(&self) -> Option<&str> {
        match self {
            Input::Made(d) => d.module.as_deref(),
            Input::Pending(p) => p.module.as_deref(),
        }
    }
    /// Its host and file, a guest's: a pending one's as given ahead (`Pending::with_guest`).
    pub fn guest(&self) -> Option<GuestOf<'_>> {
        match self {
            Input::Made(d) => d.guest.as_deref().map(Guest::of),
            Input::Pending(p) => p.guest.as_ref().map(Hosted::of),
        }
    }
    pub fn size(&self) -> FactsSize {
        match self {
            Input::Made(d) => facts_size(&d.facts),
            Input::Pending(p) => p.size,
        }
    }
    /// The document, made if it is not (`Pending`).
    pub fn document(&self) -> &Document {
        match self {
            Input::Made(d) => d,
            Input::Pending(p) => p.document(),
        }
    }
    /// Whether its document is made: a pending one once first read.
    pub fn is_made(&self) -> bool {
        match self {
            Input::Made(_) => true,
            Input::Pending(p) => p.made.get().is_some(),
        }
    }
    /// Why its document could not be made, once it was read.
    pub fn failure(&self) -> Option<&str> {
        match self {
            Input::Made(_) => None,
            Input::Pending(p) => p.failed.get().map(String::as_str),
        }
    }
}

/// W's values (`Extra`) and the AST patterns' matches (`ast_match`, matched over the tape), which a batch reads off
/// its documents' tapes.
const VALUES_TAPED: [&str; 6] = ["value", "static_value", "value_type_of", "value_type", "ast_match", VALUE_REQ];

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
    /// The template of string node `id` of `doc`, a document of its dialect: its text, each interpolation `marker`,
    /// and each character's place, `[its byte in the text, its source start, its source end]`. `None` where the node
    /// has no template (a plain string, bytes, a value not known). Python's f-strings (`template`).
    fn template(&self, _doc: &crate::code_rule_dsl::tape::TapeDoc, _id: u32, _marker: &str) -> Option<(String, Vec<[u32; 3]>)> {
        None
    }
}

/// The text of a template's holes (decision 0597): an f-string's template parsed as a guest has this identifier for
/// each interpolation, standing at its field (`template`). An identifier, so it parses where a name or a value goes.
pub const HOLE: &str = "rr_hole";

/// The template of string node `id` of `doc` (of `dialect`), as its family's producer reads it (`Producer::template`),
/// its holes [`HOLE`], placed: a copied stretch at its bytes, an escape's output at the escape, a hole whole at its
/// field; kept within `within`, the node's bytes. An f-string's template is a text root (decision 0597).
pub fn template(dialect: crate::code_rule_dsl::language::GrammarDialect, doc: &crate::code_rule_dsl::tape::TapeDoc, id: u32, within: [u32; 2]) -> Option<(String, Placement)> {
    let (text, spans) = producers().iter().filter(|p| p.dialect() == dialect).find_map(|p| p.template(doc, id, HOLE))?;
    let mut pieces: Vec<normalized::Piece> = Vec::new();
    // The previous character: whether it is copied, and its source bytes.
    let mut prev: Option<(bool, [u32; 2])> = None;
    for (i, &[at, start, end]) in spans.iter().enumerate() {
        let len = spans.get(i + 1).map_or(text.len() as u32, |n| n[0]) - at;
        let copied = end - start == len;
        // A copied character goes on its stretch at the same offset; a hole's characters (or an escape's) stand
        // together at their one source.
        let continues = match (prev, pieces.last()) {
            (Some((true, _)), Some(p)) if copied => !p.escape && p.s - i64::from(p.d) == i64::from(start) - i64::from(at),
            (Some((false, span)), _) if !copied => span == [start, end],
            _ => false,
        };
        if !continues {
            pieces.push(normalized::Piece { d: at, s: i64::from(start), escape: !copied });
        }
        prev = Some((copied, [start, end]));
    }
    let end = spans.last().map_or(within[1], |s| s[2]);
    pieces.push(normalized::Piece { d: text.len() as u32, s: i64::from(end), escape: false });
    Some((text, Placement::Mapped { pieces: pieces.into(), within }))
}

/// A [`Producer`]'s run over one batch.
pub trait ProducerRun: Send {
    /// Document `p`'s rows (`d` its document, `dialect` its grammar's kinds, `own`: it is of the producer's dialect).
    fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, own: bool, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<()>;
    /// The relations of the rows it made since it was last asked, with `atom` mapping a string's index to its atom. A
    /// batch that grows (decision 0431 §6) asks after each round, and keeps the producer: what it knows of a file
    /// (its annotations, for the guests a later round adds) stays.
    fn relations(&mut self, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, Relation)>;
    /// Whether it reads guests' documents too (else only files').
    fn reads_guests(&self) -> bool {
        false
    }
    /// Its name, for the profile's spans.
    fn name(&self) -> &'static str;
}

static PRODUCERS: std::sync::OnceLock<Vec<&'static dyn Producer>> = std::sync::OnceLock::new();

/// Registers the families' producers (once, at load), in the order their relations are listed.
pub fn install_producers(producers: Vec<&'static dyn Producer>) {
    let _ = PRODUCERS.set(producers);
}

fn producers() -> &'static [&'static dyn Producer] {
    PRODUCERS.get().map_or(&[], |v| v.as_slice())
}

/// A comment's text without its marker (core.rerail's `comment`): `#`, `--`, `//` (and a doc comment's `/` or `!`),
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
/// It reads the projection in place: no list of the nodes, no way back by id.
pub struct Numbering {
    p: Option<(Projected, std::rc::Rc<Dialect>)>,
}

impl Numbering {
    /// Its document's facts, read through its projection's cache (`Projected::f`).
    pub fn f(&self) -> Option<FactsIn<'_>> {
        self.p.as_ref().map(|(p, _)| p.f())
    }
    /// The local id of the node whose id's low 32 bits are `low`.
    pub fn local(&self, low: u32) -> Option<u32> {
        let (p, _) = self.p.as_ref()?;
        p.local_of(low).filter(|&l| l < p.len())
    }
    /// The node whose id's low 32 bits are `low`: its local id, core id and kind.
    pub fn at(&self, low: u32) -> Option<(u32, u32, &'static str)> {
        let (p, dialect) = self.p.as_ref()?;
        let l = self.local(low)?;
        let id = p.node(l);
        Some((l, id, dialect.name(p.f(), id)))
    }
    /// Local id `local`'s core id and kind name.
    pub fn node(&self, local: u32) -> Option<(u32, &'static str)> {
        let (p, dialect) = self.p.as_ref()?;
        (local < p.len()).then(|| p.node(local)).map(|id| (id, dialect.name(p.f(), id)))
    }
    /// The root's id; `None` for a document with no nodes.
    pub fn root(&self) -> Option<i64> {
        let (p, _) = self.p.as_ref()?;
        (p.len() > 0).then(|| p.global(0))
    }
    /// The id of the innermost numbered node over host bytes `[a, b]`: from the root, down through the child whose
    /// span holds them (`GuestRows::host_nodes`' walk); `None` without a root.
    pub fn over(&self, [a, b]: [u32; 2]) -> Option<i64> {
        let (p, _) = self.p.as_ref()?;
        if p.len() == 0 {
            return None;
        }
        let mut over = 0;
        while let Some(c) = p.children(over).find(|&c| {
            let [x, y] = p.span(c);
            x <= a && b <= y
        }) {
            over = c;
        }
        Some(p.global(over))
    }
    /// The kind name of core id `id`, if the projection numbers it.
    pub fn kind_of(&self, id: u32) -> Option<&'static str> {
        let (p, dialect) = self.p.as_ref()?;
        p.ranks().is(Item::Node, id).then(|| dialect.name(p.f(), id))
    }
}

pub fn numbering(doc: &Document, shape: Shape) -> StoreResult<Numbering> {
    // A document read as text only has no nodes.
    if doc.facts.is_empty() {
        return Ok(Numbering { p: None });
    }
    let tag = crate::code_rule_dsl::facts::core::facts_tag(&doc.facts);
    let dialect = Dialect::kept(tag)?;
    // In `core.rerail`'s shape a guest's nodes are its root's subtree; in T's, its whole tree (`Batch::build`).
    let guest = doc.guest.as_deref().filter(|_| shape == Shape::Core).map(|g| (g, Arc::clone(&doc.facts.lines)));
    let p = Projected::new(doc.key, Arc::clone(&doc.facts), Some(&*dialect), shape, false, guest);
    Ok(Numbering { p: Some((p, dialect)) })
}

/// Node `id`'s kind name in its facts' grammar (`""` where the facts' tag names no dialect).
pub fn kind_name(f: FactsIn<'_>, id: u32) -> &'static str {
    let tag = crate::code_rule_dsl::facts::core::facts_tag(f.facts);
    Dialect::kept(tag).map_or("", |d| d.name(f, id))
}

/// A path's module: `a/b.py` is `a.b`, `a/__init__.py` is `a`; a Rust file's is its path from
/// the crate root under `crate` (`src/lib.rs` and `src/main.rs` are `crate`, `src/a/mod.rs` and
/// `src/a.rs` are `crate.a`).
/// A file's `doc` row's module (core.rerail): the one the corpus names, else its path's dotted name for a Python or
/// Rust file, and "" for any other in the core shape.
pub(super) fn doc_module(path: &str, named: Option<&str>, core: bool) -> String {
    let dotted = path.ends_with(".py") || path.ends_with(".pyi") || path.ends_with(".rs");
    match named {
        Some(m) => m.to_owned(),
        None if core && !dotted => String::new(),
        None => module(path),
    }
}

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

/// The last segment of `path`: a library document is named as the engine has the file, with `\` on Windows.
pub(crate) fn file_name(path: &str) -> &str {
    path.rsplit(|c| c == '/' || (cfg!(windows) && c == '\\')).next().unwrap_or(path)
}

/// The package relative imports resolve against (`__package__`): a package's own name, else its
/// module's parent.
/// The package of module `m`, the file at `path`: `m` itself for a package's `__init__`, else `m`'s parent.
pub(crate) fn package_of(path: &str, m: &str) -> String {
    let name = file_name(path);
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

/// A batch's dialects by tape tag: a table the tag indexes, since a row's `kind` reads its document's dialect per
/// row (a hash map's probe was a third of that read).
/// Shared by the batches a chunk's batch grows into (`Growth`).
#[derive(Default, Clone)]
pub struct Dialects(Vec<Option<Arc<Dialect>>>);

impl Dialects {
    pub fn get(&self, tag: u8) -> Option<&Dialect> {
        self.0.get(usize::from(tag)).and_then(|d| d.as_deref())
    }
    /// Tag `tag`'s dialect, made on first use.
    fn get_or_new(&mut self, tag: u8) -> StoreResult<&Dialect> {
        let at = usize::from(tag);
        if self.0.len() <= at {
            self.0.resize_with(at + 1, || None);
        }
        if self.0[at].is_none() {
            self.0[at] = Some(Arc::new(Dialect::new(tag)?));
        }
        Ok(self.0[at].as_deref().expect("just made"))
    }
    /// Each dialect with its tag, in tag order.
    pub fn iter(&self) -> impl Iterator<Item = (u8, &Dialect)> {
        self.0.iter().filter_map(|d| d.as_deref()).map(|d| (d.tag, d))
    }
}

impl std::ops::Index<u8> for Dialects {
    type Output = Dialect;
    fn index(&self, tag: u8) -> &Dialect {
        self.get(tag).expect("a document's dialect")
    }
}

/// A dialect's kinds as T's exporter reads them. T's kinds were names, and a grammar gives some names several codes
/// (aliases): the `kind` column holds each name's canonical code (`KindCatalog::kind_id`, what a program's constant
/// lowers to), and a lookup reads the postings of every code of that name.
pub struct Dialect {
    tag: u8,
    family: Family,
    pub catalog: KindCatalog,
    /// Each kind code's name: the grammar's C string, read once (`name` was a `strlen` and a UTF-8 check per node,
    /// 0.9 s of a cold PostHog run in string annotations' host scan alone).
    names: Vec<&'static str>,
    classes: Vec<Class>,
    /// Each kind code's `granules::kind_flags`.
    granule_kinds: Vec<u8>,
    operator: Option<u16>,
    /// The `operators` field: a comparison's operator tokens (`a < b <= c`).
    operators: Option<u16>,
    pub canonical: Vector<u16>,
    /// Whether W reads values of the dialect's nodes (its family's `ValueRules::SUPPORTED`): `Extra`'s `value` and
    /// `value_type` are W's for its documents, `core_rr`'s literal values for the others (an `.rerail` file's).
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
        let names: Vec<&'static str> = (0..catalog.kind_count()).map(|k| catalog.kind_name(k as u16).unwrap_or("")).collect();
        let classes = (0..catalog.kind_count()).map(|k| catalog.kind_name(k as u16).map_or(Class::Other, Class::of)).collect();
        let granule_kinds = (0..catalog.kind_count()).map(|k| catalog.kind_name(k as u16).map_or(0, granules::kind_flags)).collect();
        let canonical: Vector<u16> = (0..catalog.kind_count() as u16)
            .map(|k| match catalog.kind_name(k) {
                Some(name) if catalog.kind_is_named(k) => Some(catalog.kind_id(name)).filter(|&c| c != 0).unwrap_or(k),
                _ => k,
            })
            .collect();
        let family = Family::of(from_tape_tag(tag).and_then(crate::code_rule_dsl::language::label));
        let grammar = crate::code_rule_dsl::tape::tokens::TokenGrammar::of_dialect(dialect);
        Ok(Self { tag, family, catalog, names, classes, granule_kinds, operator: catalog.field_id("operator"), operators: catalog.field_id("operators"), canonical, values, grammar })
    }
    /// What kind code `code` is to the granules (`granules::kind_flags`); ERROR and an unknown code are nothing.
    pub fn granule_kind(&self, code: u16) -> u8 {
        self.granule_kinds.get(usize::from(code)).copied().unwrap_or(0)
    }
    /// Node `id`'s kind code (ERROR's is `u16::MAX`).
    pub(super) fn code(&self, f: FactsIn<'_>, id: u32) -> u16 {
        kind_code(f.kind(id))
    }
    fn class(&self, f: FactsIn<'_>, id: u32) -> Class {
        self.classes.get(usize::from(self.code(f, id))).copied().unwrap_or(Class::Other)
    }
    pub fn name(&self, f: FactsIn<'_>, id: u32) -> &'static str {
        self.code_name(self.code(f, id))
    }
    /// Kind code `code`'s name; one past the grammar's kinds (ERROR's) is the catalog's.
    fn code_name(&self, code: u16) -> &'static str {
        self.names.get(usize::from(code)).copied().unwrap_or_else(|| self.catalog.kind_name(code).unwrap_or(""))
    }
    /// Node `id`'s kind atom as the `kind` column holds it: its name's canonical code's.
    fn kind(&self, f: FactsIn<'_>, id: u32) -> i64 {
        i64::from(self.kind_of_atom(f.kind(id)))
    }
    /// A node's kind atom (`Facts::kind`) as the `kind` column holds it: its name's canonical code's.
    pub fn kind_of_atom(&self, atom: u32) -> u32 {
        let code = kind_code(atom);
        kind_atom(self.tag, self.canonical.get(usize::from(code)).copied().unwrap_or(code))
    }
    /// A kind atom's name (a token's, `Facts::tokens`).
    pub fn atom_name(&self, atom: u32) -> &'static str {
        self.code_name(crate::code_rule_dsl::facts::core::kind_code(atom))
    }
    /// The child of `id` in field `field`.
    fn field_child(&self, f: FactsIn<'_>, id: u32, field: &str) -> Option<u32> {
        let field = self.catalog.field_id(field)?;
        f.children(id).iter().copied().find(|&c| f.field(c) == field)
    }
    /// A name's dotted text: an identifier's, or an attribute's object's and attribute's joined by a dot (`a.b.c`),
    /// whatever spaces the source has between them. `None` where the object has no name (a call, a subscript).
    fn dotted(&self, f: FactsIn<'_>, id: u32) -> Option<String> {
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
    fn string<'f>(&self, f: FactsIn<'f>, id: u32) -> Option<(String, Option<&'f str>)> {
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
/// (`core.rerail`'s).
/// Which place a projection's node's is, as its tape's index reads it (`Projected::idx`; `ViewDoc::place`'s numbers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PlaceAmong {
    /// Among its parent's named children (core.rerail's shape).
    Named = rerail_clif::view_reads::PLACE_NAMED,
    /// Among its parent's named children that are no comment (T's shape, which numbers no comment).
    PastComments = rerail_clif::view_reads::PLACE_PAST_COMMENTS,
}

/// The positions with `text` (the `text` view's rows after its nodes'): every token's where a rule reads a token's
/// text (`Projection::token_text`), the source's and each line's where one reads theirs (`Projection::wants_span_text`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextSpans {
    pub tokens: bool,
    pub source: bool,
    pub lines: bool,
}

pub struct Projected {
    pub module: Option<crate::code_rule_dsl::module_path::ModulePath>,
    pub key: u32,
    /// The key of the file it is part of (decision 0431 §4): its own, or a program's file's (`Guest::file`). Its
    /// `docof`; its nodes keep its own key, which hiding reads.
    pub file: u32,
    pub tag: u8,
    pub facts: Arc<Facts>,
    /// The cache its 16z tape's blocks are decoded into: its batch's (`Growth::cache`), the block the device's reads
    /// share; its own where it is projected alone (`new`). Every read of its nodes names it (`f`).
    cache: Option<Arc<ZCache>>,
    /// The tape nodes it numbers, its nodes and tokens, as a rank index (`ranks::Ranks`, decision 0353 step 2): a
    /// node's local id is its rank among the nodes. In its batch's block (decision 0289 §3), which the device reads.
    ranks: ranks::Ranks,
    /// Which place a node's is (`idx`): among its named siblings, or among those that are no comment (T's shape
    /// numbers no comment).
    place: PlaceAmong,
    /// Walked by anchors: an outermost anchor hangs from the root, which is not its tape parent, so a walk's parent
    /// is not always the tape's (a sibling is then read off the tape, `prev_sibling`).
    anchored: bool,
    /// `core.rerail`'s lines and spans; `None` in T's shape.
    pub extra: Option<Box<Locs>>,
    /// Node ids numbered per definition (`DefIds`), when the run asks for them; else a node's id is its preorder.
    defs: Option<DefIds>,
    /// Its positions with `text` (`View::Text`); its nodes with text are `Item::Text`'s.
    text_spans: TextSpans,
    /// Each term of its tape's index (`DocIndex::term`): its text's atom, interned at the first read of a leaf of it
    /// (`u32::MAX` before), so every other leaf of the term reads it with no hash of its text (decision 0656 step 4).
    term_atoms: OnceLock<Box<[std::sync::atomic::AtomicU32]>>,
}

/// Node ids per definition (decisions 0083 §3, 0094 §2; `granules`): a node's id is `document << 32 | slot << 18 |
/// index`, its granule's slot and its index there. An edit renumbers the granules it touches and no other's.
/// Spans that are positions (a document's text, its lines, its tokens) are in the top slots, in order; `Extra`'s spans
/// in the slots below them (`granules::ExtraSpan`), so a node by definition is never read as one of those spans, nor
/// one of them as a node.
///
/// An id is computed in place: the granule table (`granules::Table`), with the projection's rank index, answers a tape
/// node's id, an id's tape node, and a node's order key (decision 0353, "Node identity"). No array by node.
struct DefIds {
    table: granules::Table,
}

use granules::INDEX_BITS;
pub use ranks::Item;

/// Whether this process numbers nodes per definition: the default; `RERAIL_NODE_IDS=preorder` numbers them in
/// preorder, to compare.
pub fn definition_ids() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| !crate::code_rule_dsl::logic::profile::var("RERAIL_NODE_IDS").is_ok_and(|v| v == "preorder"))
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
    /// The ids of `p`'s nodes, and of `tokens` (each token's node, in order), by definition: the granule table.
    fn new(p: &Projected, dialect: &Dialect, tokens: &[u32]) -> Self {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "granules".into());
        let table = granules::number(p, dialect, tokens, None);
        table.describe(p.ranks());
        DefIds { table }
    }
}

/// A document's lines, and its spans past its nodes (`Shape::Core`'s): from local id `len()`, its anonymous tokens
/// (`tokens` of them, in preorder), then its whole text, then each line. Not `Extra`'s spans (`SPAN`), which the
/// relations core.rerail's programs read beyond T's are made over.
pub struct Locs {
    /// The facts' lines (`Facts::lines`): the one definition of a place, the language's (decision 0612).
    lines: Arc<crate::code_rule_dsl::lines::LineTable>,
    pub tokens: u32,
    /// Each node's end point (`end_point`, where a program reads it): a zero-width span at its end offset, after the
    /// lines, one per node in local order. 0 where no program reads them.
    pub ends: u32,
    /// The whole source and its lines follow the tokens, implicit (`Projected::span`): a guest has none of its own.
    own_text: bool,
    /// A guest's host lines and placement: its places are the host's (decision 0055). A guest has no source-text
    /// spans of its own (`source`, `src_line`): those are its file's.
    host: Option<(Arc<crate::code_rule_dsl::lines::LineTable>, Placement)>,
}

impl Locs {
    /// The document's spans past its nodes: its tokens, then (its own text) the whole source and each line.
    fn positions(&self) -> u32 { self.ends_from() + self.ends }
    /// The first end point's position: past the tokens, the whole source and the lines.
    fn ends_from(&self) -> u32 { self.tokens + if self.own_text { 1 + self.lines.count() as u32 } else { 0 } }
    /// The lines its places are read in: its host's for a guest, else its own (`ViewRow::lines`).
    pub(super) fn place_lines(&self) -> &Arc<crate::code_rule_dsl::lines::LineTable> {
        self.host.as_ref().map_or(&self.lines, |(lines, _)| lines)
    }
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
                let at = self.lines.start((l as usize).clamp(1, self.lines.count()) - 1) as u32;
                lines.line(place.host([at, at])[0] as usize) as u32 + 1
            }
            None => l,
        }
    }
}


/// `tape` read in its batch, whose cache is `cache` (`TapeDoc::in_batch`): as it is where none is needed.
fn in_batch(tape: &crate::code_rule_dsl::tape::TapeDoc, cache: Option<&Arc<ZCache>>) -> crate::code_rule_dsl::tape::TapeDoc {
    match cache {
        Some(c) => tape.in_batch(c),
        None => tape.clone(),
    }
}

/// A cache for one document's 16z blocks, where it is projected alone (`Projected::new`: a batch of its own).
fn alone_cache(facts: &Facts) -> Option<Arc<ZCache>> {
    facts.z_tape().map(|t| Arc::new(ZCache::of([t])))
}

/// The cache a batch's documents' 16z blocks are decoded into: `prior`, a batch it grows from's, where its entries
/// hold every new document's blocks, else one made for them all (in the device's memory where there is a device,
/// so the device's reads share it, `device_chunk`).
fn batch_cache<'a>(prior: Option<&Arc<ZCache>>, tapes: impl IntoIterator<Item = &'a rerail_tape_format::f16::Tape16Z> + Clone) -> Option<Arc<ZCache>> {
    let (blocks, entry) = tapes.clone().into_iter().fold((0, 0), |(b, e), t| (b + t.block_count(), e.max(t.entry_size())));
    if blocks == 0 {
        return prior.cloned();
    }
    if let Some(prior) = prior.filter(|c| c.entry_size() >= entry) {
        return Some(Arc::clone(prior));
    }
    // Its entries hold the prior batch's documents' blocks too: the device reads them all through this one.
    let entry = entry.max(prior.map_or(0, |c| c.entry_size()));
    Some(Arc::new(device_chunk::batch_cache(blocks, entry)))
}

impl Projected {
    /// Document `key`'s projection in `shape`; `dialect` is its facts' (`None` for a text-only document, which has no
    /// node). A `withheld` document (`core.rerail`: one that does not parse) has no nodes and no spans: no facts. A guest's
    /// nodes are its root's subtree, placed in its host's lines (`host`).
    /// Its arrays are in a block of its own; a batch's documents' are in the batch's (`narrowed`).
    pub fn new(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape, withheld: bool, guest: Option<(&Guest, Arc<crate::code_rule_dsl::lines::LineTable>)>) -> Self {
        let block = cells::block(Self::bytes(&facts));
        let cache = alone_cache(&facts);
        Self::narrowed(key, facts, dialect, shape, withheld, guest, None, shape == Shape::Core, None, &block, cache)
    }
    /// `new`, narrowed to `anchors` (`narrowed`): the root and each outermost anchor's subtree.
    pub fn anchored(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape, anchors: &BTreeSet<String>) -> Self {
        let block = cells::block(Self::bytes(&facts));
        let cache = alone_cache(&facts);
        Self::narrowed(key, facts, dialect, shape, false, None, Some(anchors), shape == Shape::Core, None, &block, cache)
    }

    /// Its facts, read through its batch's cache (`FactsIn`).
    #[inline]
    pub fn f(&self) -> FactsIn<'_> {
        self.facts.read_in(self.cache.as_deref())
    }
    /// The cache its 16z tape's blocks are decoded into, where it has one.
    pub fn cache(&self) -> Option<&Arc<ZCache>> {
        self.cache.as_ref()
    }

    /// What a projection's arrays can take in a block: `core`, `local`, `up`, `low`, the ordinals and the fielded
    /// nodes, none longer than the tape, with room for alignment.
    pub fn bytes(facts: &Facts) -> usize {
        Self::bytes_of(facts_size(facts))
    }

    /// `bytes` of facts of size `size` (`FactsSize`), before they are built.
    pub fn bytes_of(size: FactsSize) -> usize {
        // The views' entry and its codes' table too (`view_docs::ViewEntry`).
        6 * 4 * size[0] as usize + 6 * 16 + 8 * size[1] as usize + 256
    }

    /// As `new`, but with `anchors` (`Projection::node_anchors`) a file's nodes are its root and the subtrees of the
    /// nodes of those kinds, found by the kind postings: no other node is walked. An anchor below no other is its
    /// root's child here; the program reads no node outside the subtrees, nor an anchor's parent or place among its
    /// siblings, and its node ids stay inside its phase.
    #[allow(clippy::too_many_arguments)]
    /// `token_order`: its token order's items (`ranks::Marks::token_order`) are marked, where a program reads
    /// `first_token`, `last_token`, `token_before` or `token_after` (`tokens::VIEWS`). `text`: where a program reads
    /// `text`, which nodes have it (`Item::Text`, `core_rr::TextRows`) and which positions.
    #[allow(clippy::too_many_arguments)]
    pub fn narrowed(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape, withheld: bool, guest: Option<(&Guest, Arc<crate::code_rule_dsl::lines::LineTable>)>, anchors: Option<&BTreeSet<String>>, token_order: bool, text: Option<(Option<&core_rr::TextRows<'_>>, TextSpans)>, block: &Arc<Block>, cache: Option<Arc<ZCache>>) -> Self {
        let identity_span = crate::code_rule_dsl::logic::profile::span(|| "identity maps".into());
        // A document made when first read (`Pending`), after its batch's cache was sized, whose blocks its entries do
        // not hold: read in a batch of its own.
        let cache = cache.filter(|c| facts.z_tape().is_none_or(|t| c.entry_size() >= t.entry_size())).or_else(|| alone_cache(&facts));
        let f = facts.read_in(cache.as_deref());
        let (first, last) = match guest {
            Some((g, _)) => (g.root, if (g.root as usize) < f.len() { f.subtree_end(g.root) } else { g.root + 1 }.max(g.root + 1)),
            None => (0, f.len() as u32),
        };
        let end = if withheld { first } else { last.min(f.len() as u32) };
        // The id ranges walked: the document's, or (`anchors`) its root's and each outermost anchor's subtree.
        let ranges: Vec<(u32, u32)> = match (anchors, dialect) {
            (Some(kinds), Some(dialect)) if guest.is_none() && first < end => {
                let mut starts: Vector<u32> = f.kinds().into_iter().filter(|&k| kinds.contains(dialect.atom_name(k))).flat_map(|k| f.of_kind(k)).collect();
                starts.sort_unstable();
                let mut out = vec![(first, first + 1)];
                for a in starts {
                    let b = if (a as usize) < f.len() { f.subtree_end(a) } else { a + 1 }.max(a + 1).min(end);
                    match out.last_mut() {
                        Some(last) if a < last.1 => last.1 = last.1.max(b),
                        _ => out.push((a, b)),
                    }
                }
                out
            }
            _ => vec![(first, end)],
        };
        // The walk: which tape nodes the projection numbers, marked in its rank index (`ranks`) in the block. A node's
        // local id is its rank among them, in preorder.
        let core_tokens = shape == Shape::Core && dialect.is_some() && !withheld;
        // Walked by anchors, the root's tokens (below) lie anywhere in the document, past the last anchored subtree
        // too: the index then reaches the document's end.
        let root_tokens = ranges.len() > 1 && core_tokens && f.named(first);
        let (lo, hi) = (ranges.first().map_or(first, |r| r.0), if root_tokens { end } else { ranges.last().map_or(first, |r| r.1) }.max(first + 1));
        let ranks = ranks::Ranks::build(block, lo, hi, |marks| {
            if withheld {
                return;
            }
            // Walked by anchors, the root's own range is the root alone: its tokens between the anchored subtrees (a
            // `;` between two statements) are in no range, yet are tokens of an included node, as `Projected::span`
            // reads them.
            if root_tokens {
                let outside = |c: u32| !ranges.iter().any(|&(a, b)| a <= c && c < b);
                for c in f.children(first) {
                    if !f.named(c) && outside(c) {
                        marks.mark(Item::Op, c);
                        if f.end(c) > f.start(c) {
                            marks.mark(Item::Token, c);
                        }
                    }
                }
            }

            let mut n = 0usize;
            // The numbered nodes on the walk's path, each with whether a numbered direct child of it was marked yet (a
            // later one is after a sibling: `next_sibling`'s rows). A node's tape parent comes with the walk, so the
            // path is popped down to it, with no subtree's end read: a node above it on the path is closed.
            let mut open: Vec<(u32, bool)> = Vec::new();
            for (id, above) in ranges.iter().flat_map(|&(a, b)| f.with_parents(a, b)) {
                if !f.named(id) {
                    // A token (`Facts::tokens`) of an included node: its span is made on first read (`Projected::span`).
                    // Any anonymous child of one, zero-width too, is one of `op`'s rows.
                    if core_tokens && above.is_some_and(|q| marks.has(Item::Node, q)) {
                        marks.mark(Item::Op, id);
                        if f.end(id) > f.start(id) {
                            marks.mark(Item::Token, id);
                        }
                    }
                    continue;
                }
                if dialect.is_none_or(|dialect| shape == Shape::T && dialect.class(f, id) == Class::Comment) {
                    continue;
                }
                // T walks named children from the root: a node under one left out is not reached.
                let rooted = match above {
                    _ if id == first => false,
                    None => false,
                    Some(q) if marks.has(Item::Node, q) => true,
                    // An outermost anchor's parent is left out: it hangs from the root.
                    Some(_) if ranges.len() > 1 && ranges.iter().any(|&(a, _)| a == id) => true,
                    Some(_) => continue,
                };
                if !rooted && n != 0 {
                    continue;
                }
                marks.mark(Item::Node, id);
                // An outermost anchor: the root's child in the projection (its tape parent the root, or left out).
                if ranges.len() > 1 && id != first && ranges.binary_search_by_key(&id, |r| r.0).is_ok() {
                    marks.mark(Item::Anchor, id);
                }
                if n != 0 && f.field(id) != 0 {
                    marks.mark(Item::Field, id);
                }
                if dialect.is_some_and(|d| d.granule_kind(d.code(f, id)) & granules::SPREAD != 0) {
                    marks.mark(Item::Spread, id);
                }
                if let (Some((Some(rows), _)), Some(d)) = (text, dialect)
                    && rows.has(f, d, id)
                {
                    marks.mark(Item::Text, id);
                }
                // A direct child of a numbered node: its parent is on the path. An anchor that hangs from the root is no
                // child of it, and leaves the path as it is (what it closes, the next child of an open node pops).
                if let Some(q) = above.filter(|&q| marks.has(Item::Node, q)) {
                    while open.last().is_some_and(|&(x, _)| x != q) {
                        open.pop();
                    }
                    if let Some(top) = open.last_mut() {
                        if top.1 {
                            marks.mark(Item::Sibling, id);
                        }
                        top.1 = true;
                    }
                }
                open.push((id, false));
                n += 1;
            }
            // The token order: the lexemes are the tokens and the named leaves with text that are no comment.
            if let Some(dialect) = dialect.filter(|_| token_order && core_tokens) {
                marks.token_order(
                    |x| f.subtree_end(x) == x + 1 && f.end(x) > f.start(x) && dialect.granule_kind(dialect.code(f, x)) & granules::COMMENT == 0,
                    |x| f.subtree_end(x),
                );
            }
        });
        let tokens = ranks.count(Item::Token);
        let file = Guest::file_key(guest.as_ref().map(|(g, _)| g.of()), key);
        let mut p = Projected {
            module: None,
            key,
            file,
            tag: f.tag,
            ranks,
            place: if shape == Shape::T { PlaceAmong::PastComments } else { PlaceAmong::Named },
            anchored: ranges.len() > 1,
            extra: None,
            defs: None,
            facts: Arc::clone(&facts),
            cache: cache.clone(),
            text_spans: if shape == Shape::Core { text.map(|(_, spans)| spans).unwrap_or_default() } else { TextSpans::default() },
            term_atoms: OnceLock::new(),
        };
        drop(identity_span);
        if shape == Shape::Core {
            let lines = Arc::clone(&f.lines);
            let _ids_span = crate::code_rule_dsl::logic::profile::span(|| "definition ids".into());
            if definition_ids() && let Some(dialect) = dialect {
                // Each token's node, in preorder of the tokens (`token_order`).
                let owners: Vector<u32> = p.token_order().into_iter().map(|(_, l)| l).collect();
                debug_assert_eq!(owners.len(), tokens as usize);
                p.defs = Some(DefIds::new(&p, dialect, &owners));
            }
            // Raw file text remains valid when its tree is withheld. Guests have no own text spans.
            let own_text = guest.is_none();
            let host = guest.filter(|(g, _)| matches!(g.made, Made::Host | Made::File)).map(|(g, lines)| (lines, g.in_file.clone()));
            p.extra = Some(Box::new(Locs { lines, tokens, ends: 0, own_text, host }));
        } else if definition_ids() && let Some(dialect) = dialect {
            // T's shape has no token spans: its nodes alone are numbered per definition.
            p.defs = Some(DefIds::new(&p, dialect, &[]));
        }
        p
    }
    /// The tape node of local id `local` (a select of the rank index).
    #[inline]
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn node(&self, local: u32) -> u32 {
        self.ranks.select(Item::Node, local).expect("a local id of the projection")
    }
    /// The local id of tape node `id`, `NONE` for one the projection leaves out.
    #[inline]
    pub fn local_of_node(&self, id: u32) -> u32 {
        self.ranks.local(Item::Node, id).unwrap_or(NONE)
    }
    /// Every node, in preorder: `(local id, tape node)`.
    pub fn nodes(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        self.ranks.iter(Item::Node)
    }
    /// Every node, in preorder, with its parent (`parent`): `(local id, tape node, parent's local id)`, the root's
    /// `NONE`. One pass that keeps the open subtrees, so no parent is read off the tape: a node's parent is the
    /// nearest included node whose subtree holds it (an anchor's, the root).
    pub fn walk(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        self.walk_up().map(|(l, x, up)| (l, x, up.map_or(NONE, |(q, _)| q)))
    }
    /// `walk`, with the parent's tape node: `(local id, tape node, (parent's local id, its tape node))`.
    pub fn walk_up(&self) -> impl Iterator<Item = (u32, u32, Option<(u32, u32)>)> + '_ {
        let mut open: Vec<(u32, u32, u32)> = Vec::new();
        self.ranks.iter(Item::Node).map(move |(l, x)| {
            while open.last().is_some_and(|&(_, _, end)| end <= x) {
                open.pop();
            }
            let parent = open.last().map(|&(q, qx, _)| (q, qx));
            open.push((l, x, self.f().subtree_end(x)));
            (l, x, parent)
        })
    }
    /// Every node's and span's byte span, in local order (`span`): the nodes in one pass of the index, then the spans
    /// past them (tokens, the text, lines, end points). No select a node.
    pub fn spans(&self) -> impl Iterator<Item = (u32, [u32; 2])> + '_ {
        let f = self.f();
        self.nodes().map(move |(l, x)| (l, [f.start(x), f.end(x)])).chain((self.len()..self.locs()).map(|l| (l, self.span(l))))
    }
    /// The block the projection's cells are in.
    pub fn block(&self) -> &Arc<Block> {
        self.ranks.block()
    }
    /// The rank index of the tape nodes it numbers.
    pub fn ranks(&self) -> &ranks::Ranks {
        &self.ranks
    }
    /// Local id `local`'s parent, as a local id: its tape parent's, or the root's for an anchor of a narrowed
    /// projection (`narrowed`), whose parent is left out; `NONE` for the root.
    #[inline]
    #[cfg_attr(feature = "tape-reads", track_caller)]
    pub fn parent(&self, local: usize) -> u32 {
        if local == 0 {
            return NONE;
        }
        self.parent_of_node(self.node(local as u32))
    }
    /// The parent's local id of the node at tape node `x` (a node of the projection, not its root): its tape parent,
    /// searched from the innermost granule holding it (`granules::Table::holding`), else its document's root.
    pub fn parent_of_node(&self, x: u32) -> u32 {
        let anchor = self.defs.as_ref().map_or(0, |d| d.table.holding(x));
        match self.f().parent_within(x, anchor).map_or(NONE, |q| self.local_of_node(q)) {
            NONE => 0,
            p => p,
        }
    }
    /// Local id `local`'s place among its parent's children (`child`'s `i`). The tape holds no sibling's place, and
    /// hops along the siblings are as many as they are (a module's statements: a probe for each of them would be
    /// quadratic), so a read reads its tape's auxiliary index (decision 0656 step 4: the last every-8th named sibling,
    /// then at most 7 named steps; past comments for T's shape), as the device's reads do (`tape_place`,
    /// `tape_place_past_comments`). An outermost anchor's parent is the root, whose children are the anchors: its place
    /// is its rank among them (`Item::Anchor`). A scan carries the place in its walk (`places`).
    #[inline]
    pub fn idx(&self, local: usize) -> u32 {
        if local == 0 {
            return 0;
        }
        let x = self.node(local as u32);
        if self.anchored && self.ranks.is(Item::Anchor, x) {
            return self.ranks.rank(Item::Anchor, x);
        }
        match self.place {
            PlaceAmong::Named => self.f().place(x),
            PlaceAmong::PastComments => self.f().place_past_comments(x),
        }
    }
    /// Which place a node's is (`ViewDoc::place`).
    pub fn place_kind(&self) -> PlaceAmong {
        self.place
    }
    /// Every node in preorder with its place among its parent's children and its depth (`idx`, `depth`), carried in
    /// one walk of the open subtrees: `(local id, place, depth)`.
    pub fn places(&self) -> impl Iterator<Item = (u32, u32, u32)> + '_ {
        let mut open: Vec<(u32, u32)> = Vec::new();
        self.ranks.iter(Item::Node).map(move |(l, x)| {
            while open.last().is_some_and(|&(end, _)| end <= x) {
                open.pop();
            }
            let depth = open.len() as u32;
            let idx = open.last_mut().map_or(0, |o| {
                o.1 += 1;
                o.1 - 1
            });
            open.push((self.f().subtree_end(x), 0));
            (l, idx, depth)
        })
    }
    /// The nodes under local id `local`: those of its tape subtree, by rank.
    #[inline]
    pub fn size(&self, local: usize) -> u32 {
        let id = self.node(local as u32);
        self.ranks.rank(Item::Node, self.f().subtree_end(id)) - local as u32 - 1
    }
    /// Local id `local`'s numbered sibling before it (`next_sibling(a, local)`'s `a`): the tape node before it is in
    /// its previous tape sibling's subtree, climbed to that sibling, and past siblings left out. Only for a node after
    /// a sibling (`Item::Sibling`), which has one.
    pub fn prev_sibling(&self, local: u32) -> Option<u32> {
        let x = self.node(local);
        if !self.ranks.is(Item::Sibling, x) {
            return None;
        }
        let q = self.f().parent(x)?;
        let mut c = x - 1;
        loop {
            while let Some(up) = self.f().parent(c)
                && up != q
            {
                c = up;
            }
            if self.ranks.is(Item::Node, c) {
                return Some(self.ranks.rank(Item::Node, c));
            }
            c -= 1;
        }
    }
    /// Local id `local`'s numbered sibling after it (`next_sibling(local, b)`'s `b`): the tape's siblings after its
    /// subtree, those left out hopped. A node whose tape parent is not numbered has none.
    pub fn next_sibling(&self, local: u32) -> Option<u32> {
        let x = self.node(local);
        let q = self.f().parent(x).filter(|&q| self.ranks.is(Item::Node, q))?;
        let end = self.f().subtree_end(q);
        let mut s = self.f().subtree_end(x);
        while s < end {
            if self.ranks.is(Item::Node, s) {
                return Some(self.ranks.rank(Item::Node, s));
            }
            s = self.f().subtree_end(s);
        }
        None
    }
    /// The tokens (`op`'s rows): the anonymous children with text of the included nodes.
    pub fn tokens(&self) -> u32 {
        self.ranks.count(Item::Token)
    }
    /// The nodes after a sibling (`next_sibling`'s rows).
    pub fn after_siblings(&self) -> u32 {
        self.ranks.count(Item::Sibling)
    }
    /// The spread nodes (`spread`'s rows).
    pub fn spreads(&self) -> u32 {
        self.ranks.count(Item::Spread)
    }
    /// `op`'s rows: the numbered nodes' anonymous children (`Item::Op`).
    pub fn ops(&self) -> u32 {
        self.ranks.count(Item::Op)
    }
    /// `text`'s rows (`View::Text`): its nodes with text (`Item::Text`), then its positions with text (`TextSpans`):
    /// its tokens, its source, its lines, in their order among the positions.
    pub fn text_rows(&self) -> u32 {
        let Some(x) = self.extra.as_ref() else { return self.ranks.count(Item::Text) };
        let (t, own) = (self.text_spans, x.own_text);
        self.ranks.count(Item::Text) + if t.tokens { x.tokens } else { 0 } + u32::from(t.source && own) + if t.lines && own { x.lines.count() as u32 } else { 0 }
    }
    /// `text`'s row `i`: its local id, and a node's tape node (`NONE` for a position).
    pub fn text_local(&self, i: u32) -> (u32, u32) {
        let nodes = self.ranks.count(Item::Text);
        if i < nodes {
            let x = self.ranks.select(Item::Text, i).expect("a node with text");
            return (self.ranks.rank(Item::Node, x), x);
        }
        let x = self.extra.as_ref().expect("a position with text");
        let (t, mut k) = (self.text_spans, i - nodes);
        if t.tokens {
            if k < x.tokens {
                return (self.len() + k, NONE);
            }
            k -= x.tokens;
        }
        // The source, then a line (1-based: the source's position and one).
        let line = if t.source { k } else { k + 1 };
        (self.len() + x.tokens + line, NONE)
    }
    /// `text`'s row of local id `local` (its tape node `x` where known, else `NONE`), where it has text.
    pub fn text_row_of(&self, local: u32, x: u32) -> Option<u32> {
        if local < self.len() {
            return self.ranks.local(Item::Text, if x == NONE { self.node(local) } else { x });
        }
        let e = self.extra.as_ref()?;
        let (t, nodes, i) = (self.text_spans, self.ranks.count(Item::Text), local - self.len());
        let tokens = if t.tokens { e.tokens } else { 0 };
        if i < e.tokens {
            return t.tokens.then_some(nodes + i);
        }
        if !e.own_text {
            return None;
        }
        if i == e.tokens {
            return t.source.then_some(nodes + tokens);
        }
        let line = i - e.tokens - 1;
        (t.lines && (line as usize) < e.lines.count()).then_some(nodes + tokens + u32::from(t.source) + line)
    }
    /// `f` of the text of local id `local` (a node's tape node `x`, else `NONE`): a node's span's, a position's.
    pub fn with_text<R>(&self, local: u32, x: u32, f: impl FnOnce(&str) -> R) -> R {
        let facts = self.f();
        if local < self.len() {
            return f(facts.text(if x == NONE { self.node(local) } else { x }));
        }
        let [a, b] = self.span(local);
        f(&facts.source[a as usize..b as usize])
    }
    /// The atom of the text of local id `local` (`text`'s `s`): a leaf's by its term in its tape's index
    /// (`DocIndex::term`), interned at the term's first read and read after with no hash of its text; any other's
    /// interned where it is read, as `op`'s are (`Batch::op_atom`). `-1` where it could not be interned.
    pub fn text_atom(&self, atoms: &crate::content_store::atoms::Atoms, local: u32, x: u32) -> i64 {
        use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
        let intern = |text: &str| atoms.intern(text).map_or(-1, i64::from);
        let f = self.f();
        if local < self.len() && self.facts.tape.is_some() {
            let x = if x == NONE { self.node(local) } else { x };
            let index = f.index();
            if let Some(term) = index.term(x) {
                let table = self.term_atoms.get_or_init(|| (0..index.terms()).map(|_| AtomicU32::new(u32::MAX)).collect());
                let at = &table[term as usize];
                let a = at.load(Relaxed);
                if a != u32::MAX {
                    return i64::from(a);
                }
                let a = intern(f.text(x));
                if let Ok(a32) = u32::try_from(a) {
                    at.store(a32, Relaxed);
                }
                return a;
            }
            return intern(f.text(x));
        }
        self.with_text(local, x, intern)
    }
    /// Local id `local`'s depth, the root's 0: its numbered tape ancestors, by a climb (decision 0353 step 3). A scan
    /// carries it in its walk.
    pub fn depth(&self, local: usize) -> u32 {
        let mut x = self.node(local as u32);
        let mut d = 0;
        while let Some(q) = self.f().parent(x) {
            d += u32::from(self.ranks.is(Item::Node, q));
            x = q;
        }
        d
    }
    /// The `field` view's rows: the nodes below the root that hang from their parent under a field, in order.
    pub fn fielded(&self) -> u32 {
        self.ranks.count(Item::Field)
    }
    /// The local id of the `field` view's row `i`.
    pub fn fielded_local(&self, i: u32) -> u32 {
        let x = self.ranks.select(Item::Field, i).expect("a field row");
        self.ranks.rank(Item::Node, x)
    }
    /// The tokens (`Locs::tokens`: the anonymous children with text of the included nodes) in preorder, each with its
    /// node's local id: the order their ids and spans follow.
    pub fn token_order(&self) -> Vec<(u32, u32)> {
        let f = self.f();
        self.ranks.iter(Item::Token).map(|(_, x)| (x, f.parent(x).map_or(0, |q| self.local_of_node(q)))).collect()
    }
    /// The granule table, where ids are by definition (decision 0353 step 2).
    pub fn table(&self) -> Option<&granules::Table> {
        self.defs.as_ref().map(|d| &d.table)
    }
    /// The ids' low bits (nodes, then tokens) and the order keys as arrays, numbered again: the reference the table is
    /// checked and measured against, not a reader's path.
    pub fn reference(&self, dialect: &Dialect) -> granules::Reference {
        let owners: Vector<u32> = if self.extra.is_some() { self.token_order().into_iter().map(|(_, l)| l).collect() } else { rerail_blocks::vector::Vector::new() };
        let mut r = granules::Reference::default();
        granules::number(self, dialect, &owners, Some(&mut r));
        r
    }
    /// Nodes and spans: the ids `text`, `line` and `docof` take.
    pub fn locs(&self) -> u32 {
        self.len() + self.extra.as_ref().map_or(0, |x| x.positions())
    }
    /// The byte span of node or span `local`.
    pub fn span(&self, local: u32) -> [u32; 2] {
        match local.checked_sub(self.len()) {
            None => {
                let id = self.node(local);
                [self.f().start(id), self.f().end(id)]
            }
            Some(i) => self.extra.as_ref().map_or([0, 0], |x| {
                if i < x.tokens {
                    // The token's tape node, a select on the rank index: its span is the tape's.
                    let c = self.ranks.select(Item::Token, i).expect("a numbered token");
                    [self.f().start(c), self.f().end(c)]
                } else if i >= x.ends_from() {
                    let id = self.node(i - x.ends_from());
                    [self.f().end(id), self.f().end(id)]
                } else if i == x.tokens {
                    [0, self.facts.source.len() as u32]
                } else {
                    x.lines.text_of((i - x.tokens) as usize, self.facts.source.as_bytes())
                }
            }),
        }
    }
    pub fn len(&self) -> u32 {
        self.ranks.count(Item::Node)
    }
    /// Node or span `local`'s id: `key << 32 |` its low bits, by definition from the granule table (a select and
    /// the table's read), else its local id.
    pub fn global(&self, local: u32) -> i64 {
        let low = match &self.defs {
            None => local,
            Some(d) => {
                let (nodes, tokens) = (self.len(), self.ranks.count(Item::Token));
                if local < nodes {
                    d.table.low_of(&self.ranks, Item::Node, local).expect("a numbered node")
                } else if local < nodes + tokens {
                    d.table.low_of(&self.ranks, Item::Token, local - nodes).expect("a numbered token")
                } else {
                    granules::span_low(local - nodes - tokens)
                }
            }
        };
        i64::from(self.key) << 32 | i64::from(low)
    }
    /// `global` of a node, its granule searched from `hint` (`granules::Table::low_of_near`): a probe's matches, and
    /// probes in key order, read ids near each other.
    pub fn global_near(&self, local: u32, hint: &mut u32) -> i64 {
        match &self.defs {
            Some(d) if local < self.len() => i64::from(self.key) << 32 | i64::from(d.table.low_of_near(&self.ranks, Item::Node, local, hint).expect("a numbered node")),
            _ => self.global(local),
        }
    }
    /// Every node in order with its id: `(local id, tape node, id)`. One walk (`granules::ids`), no point read.
    pub fn node_ids(&self) -> impl Iterator<Item = (u32, u32, i64)> + '_ {
        let key = i64::from(self.key) << 32;
        granules::ids(self.table(), &self.ranks, Item::Node, self.len()).map(move |(l, x, low)| (l, x, key | i64::from(low)))
    }
    /// Every token in order with its id: `(local id, tape node, id)`, the local ids from `len()`.
    pub fn token_ids(&self) -> impl Iterator<Item = (u32, u32, i64)> + '_ {
        let (key, n) = (i64::from(self.key) << 32, self.len());
        granules::ids(self.table(), &self.ranks, Item::Token, n).map(move |(k, x, low)| (n + k, x, key | i64::from(low)))
    }
    /// Every node, token and span in local order with its id: `(local id, id)` (`node_ids`, `token_ids`, then the
    /// spans', which are their positions).
    pub fn loc_ids(&self) -> impl Iterator<Item = (u32, i64)> + '_ {
        let past = self.len() + self.ranks.count(Item::Token);
        self.node_ids().chain(self.token_ids()).map(|(l, _, n)| (l, n)).chain((past..self.locs()).map(|l| (l, self.global(l))))
    }
    /// Node `local`'s order key: `node_order`'s low 32 bits (`granules::Table::order_of`, from its number; its
    /// preorder index without `DefIds`).
    pub fn order(&self, local: u32) -> u32 {
        self.defs.as_ref().map_or(local, |d| d.table.order_of(&self.ranks, local).expect("a node's order key"))
    }
    /// The node whose order key (`node_order`'s low 32 bits) is `key`.
    pub fn local_of_order(&self, key: u32) -> Option<u32> {
        match &self.defs {
            Some(d) => d.table.number_of_order(&self.ranks, key),
            None => (key < self.len()).then_some(key),
        }
    }
    /// The node or span whose id's low 32 bits are `low` (`global`'s inverse): from the table's numbers, no select.
    pub fn local_of(&self, low: u32) -> Option<u32> {
        match &self.defs {
            Some(_) if granules::span_of(low).is_some() => granules::span_of(low).map(|i| self.len() + self.ranks.count(Item::Token) + i),
            Some(d) => d.table.number_of(&self.ranks, low).map(|(item, k)| if item == Item::Node { k } else { self.len() + k }),
            None => Some(low),
        }
    }
    /// The included children of `local`, in order.
    pub fn children(&self, local: u32) -> impl Iterator<Item = u32> + '_ {
        self.children_of_node(self.node(local))
    }
    /// `children` of the node whose tape node is `x`.
    pub fn children_of_node(&self, x: u32) -> impl Iterator<Item = u32> + '_ {
        self.f().children(x).into_iter().map(move |c| self.local_of_node(c)).filter(|&c| c != NONE)
    }
    /// `children`, each handed to `f`: a parent's probe makes no list of them.
    pub fn each_child(&self, local: u32, mut f: impl FnMut(u32)) {
        self.each_child_node(local, |c, _| f(c));
    }
    /// `each_child`, with each child's tape node: `f(local id, tape node)`.
    pub fn each_child_node(&self, local: u32, mut f: impl FnMut(u32, u32)) {
        self.f().each_child(self.node(local), |x| {
            let c = self.local_of_node(x);
            if c != NONE {
                f(c, x);
            }
        });
    }
}

/// One wave's documents, sorted by key, and the row offsets of the views.
pub struct Batch {
    /// Its documents, each one's projection made on its first read (decision 0494), shared with the batches it grows
    /// into (`Growth`, decision 0431 §6). Their keys (`Docs::keys`, ascending) are the index a node's document is
    /// searched in (Q220).
    pub docs: made::Docs,
    /// The programs among the documents (decision 0431 §4), by file: `(file, document)`, ascending; a lookup of `node`
    /// by its document reads a file's programs' rows with its own. Found from the documents' inputs (`Guest::file_key`): none is made for it.
    programs: Vec<(u32, usize)>,
    /// Each document's tape, for W's values answered on request (`LongTexts::value_any`, decision 0174), and its values,
    /// read off it at the first request; empty where no program asks for values.
    valued: Vec<Option<Arc<Valued>>>,
    pub dialects: Arc<Dialects>,
    /// The views' row offsets by document: nodes, children, fielded nodes, siblings, and nodes and spans
    /// (`Projected::locs`), counted (decision 0494).
    offsets: made::Offsets,
    /// The documents of each tape tag, ascending: a probe by kind reads its language's documents alone (a batch's
    /// programs, decision 0431 §4, are documents of other languages).
    of_tag: [Vector<u32>; 256],
    /// The `node` view's rows by kind, one index for each tape tag the batch has documents of (`of_tag`), made at the
    /// first lookup of one of its kinds over that tag's documents alone (Q183; decision 0494: a program's language
    /// stays unmade).
    kinds: Vec<(u8, OnceLock<KindRows>)>,
    shape: Shape,
    /// The registered lazy families (`Families`, Python's `py.*`: list step 25), by registration; `None` for one
    /// whose dialect has no document in the batch.
    families: Vec<Option<Box<dyn BoundFamilies>>>,
    /// The relations made as rows for what the program reads (`Extra` and those beside it), bound in place of a view
    /// of the same name (`line`, `end_line`, `col`, `docof`: their spans have places too).
    made: BTreeSet<&'static str>,
    /// The batch's block (decision 0289 §3): its documents' projections are made in it, and its chunk
    /// (`device_chunk`) is nested in it, so the device reads the projections where the host does.
    block: Arc<Block>,
    /// The batch's chunk (decision 0289): its views' arrays, made on the device's first read of a view.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    chunk: OnceLock<device_chunk::Chunk>,
    /// What a group's rows are made with (`make_rows`): the projection, the `match_all` regexes, the markers of the
    /// `fstring_template` atoms, the span prefilter's binding, the store's atoms, and the `ast` matchers' wave.
    projection: Arc<Projection>,
    matches: Vec<Arc<regex::Regex>>,
    markers: Vec<String>,
    metal: bool,
    atoms: Arc<crate::content_store::atoms::Atoms>,
    wave: Option<(Matchers, crate::code_rule_dsl::facts::nearest::Binding)>,
    /// The rows made, by group (decision 0494).
    rows: made::Rows,
    /// Each relation's rows, by name, as its kept statistics count them (`give_totals`, decision 0494).
    given_rows: OnceLock<HashMap<String, usize>>,
    /// Its documents' keys by their paths' atoms (`keys_of_paths`), once a read by a path asked.
    by_path: OnceLock<Option<HashMap<i64, Vec<u32>>>>,
    /// Its documents' keys by their `doc` rows' modules' atoms (`keys_of_modules`), once a read by a module asked.
    by_module: OnceLock<Option<HashMap<i64, Vec<u32>>>>,
    /// Each host's guests' keys (`Batch::guests_of`), once a read by a host's node asked.
    guests_of: OnceLock<Option<HashMap<u32, Vec<u32>>>>,
    /// Its documents' keys by the document key their `docof` rows give (`keys_of_documents`), once a read by it asked.
    by_file_key: OnceLock<HashMap<u32, Vec<u32>>>,
    /// What every group of the batch, and of the batches it grows into, makes its rows with (`Shared`).
    shared: Arc<parking_lot::Mutex<Shared>>,
    /// A tape value's batch alone (decision 0431 §8): the chunk's batch it is admitted to, whose documents hold its
    /// hosts.
    outer: Option<Arc<Batch>>,
    /// Whether its groups keep the strings their rows use (`made::Made::used`): a session's corpus batch, which the
    /// next run's renews from (decision 0616).
    keeps_strings: bool,
    /// Whether it was renewed from a held batch, whose strings it shares (decision 0616): its vocabulary is its groups'.
    renewed: bool,
    /// Each tape tag's kinds' names' atoms, by kind code (`op_atom`): interned on the first read of a token of the tag.
    op_atoms: Vec<OnceLock<Vector<u32>>>,
}

/// What a batch's groups share, and the batches it grows into (decision 0431 §6): its strings and their atoms, by
/// index (the rows a group makes name strings by index, and a producer keeps what it read of a file by them), and the
/// producers of its relations, kept so a later group's or round's rows are read with what they know of the files.
#[derive(Default)]
pub(super) struct Shared {
    /// Each string once, by index.
    strings: rerail_blocks::maps::BlockInterner,
    /// Each string's atom, by its index.
    atoms: rerail_blocks::maps::BlockLog<u32>,
    produced: Option<Vec<(Option<u8>, Box<dyn ProducerRun>)>>,
}

/// What a chunk's batch keeps to grow by a round's documents (decision 0431 §6): the batch it grew last, whose
/// documents the next one's first are (shared, made), its block, which the later documents are projected into, and
/// what its groups share (`Shared`: strings, atoms, producers). A batch grows once complete (decision 0494: each
/// relation is one block relation that each group goes on in, so the chunk's rows are a prefix the next round's and
/// the values' follow, as the device binds them whole).
#[derive(Default)]
pub struct Growth {
    last: Option<Arc<Batch>>,
    dialects: Dialects,
    host: HashMap<u32, u32>,
    made: BTreeSet<&'static str>,
    block: Option<Arc<Block>>,
    /// Its documents' 16z blocks' cache (`batch_cache`): the block's, shared as the block is.
    cache: Option<Arc<ZCache>>,
    shared: Arc<parking_lot::Mutex<Shared>>,
    /// The files' lines, by key, that their guests are placed by.
    lines_of: Arc<parking_lot::RwLock<HashMap<u32, Arc<crate::code_rule_dsl::lines::LineTable>>>>,
    /// The batch a session's last run made, which the batch made next is renewed from (decision 0616).
    renew: Option<Arc<Batch>>,
    /// Whether the batch made keeps its groups' strings, for a later run to renew from it (decision 0616).
    keeps_strings: bool,
    /// Each tape value admitted (decision 0431 §8): its batch, the places it has rows of, and whether it is withheld
    /// (no root). One given more places adds their rows alone (`GuestRows::places_added`).
    values: std::collections::BTreeMap<u32, (Arc<Batch>, usize, bool)>,
    /// The program's `ast` matchers, which run over each group's files' tapes (a round adds guests, whose have none).
    wave: Option<(Matchers, crate::code_rule_dsl::facts::nearest::Binding)>,
}

/// The `node` view's rows by kind: an index of the view (decision 0353: an index in its relation's place, not a copy of
/// its rows), made in one walk of each document's numbered nodes (`Projected::nodes`, no select) at the batch's first
/// lookup by kind. Each match keeps its document, local id and tape node, so a lookup reads no rank and no select.
/// Q220: a probe `kind($m, $k)` scanned every document's tape for the kind's codes (`Facts::of_kind`), 55 k probes
/// of a cold run at 39 µs each, where t41's `kind`, a relation of its own, had an index.
///
/// An index of the `node` view, so in a block of the run's (decision 0353): on the heap before (4.1 MB under one
/// stratum's joins at a 300-file run's peak, on the M3).
struct KindRows {
    /// The kinds, ascending.
    keys: rerail_blocks::vector::Vector<i64>,
    /// Where each kind's matches start in `at`, and one past the last.
    starts: rerail_blocks::vector::Vector<u32>,
    /// Each kind's nodes in row order: `[document, local id, tape node]`.
    at: rerail_blocks::vector::Vector<[u32; 3]>,
}

impl KindRows {
    /// Over the documents of tape tag `tag` alone (`Batch::of_tag`).
    fn new(b: &Batch, tag: u8) -> Self {
        let Some(dialect) = b.dialects.get(tag) else { return KindRows { keys: crate::code_rule_dsl::facts::rows::vector_in(None, 0), starts: crate::code_rule_dsl::facts::rows::collect_in(None, [0]), at: crate::code_rule_dsl::facts::rows::vector_in(None, 0) } };
        let docs = &b.of_tag[usize::from(tag)];
        // The nodes in row order, and each one's kind; then the rows ordered by kind, stable: within a kind, the rows
        // stay in order.
        let n: usize = docs.iter().map(|&d| b.docs[d as usize].len() as usize).sum();
        let mut at: rerail_blocks::vector::Vector<[u32; 3]> = crate::code_rule_dsl::facts::rows::vector_in(None, n);
        let mut kinds: rerail_blocks::vector::Vector<i64> = crate::code_rule_dsl::facts::rows::vector_in(None, n);
        for &doc in docs {
            let p = &b.docs[doc as usize];
            for (l, x) in p.nodes() {
                at.push([doc, l, x]);
                kinds.push(dialect.kind(p.f(), x));
            }
        }
        let mut order: rerail_blocks::vector::Vector<u32> = crate::code_rule_dsl::facts::rows::collect_in(None, 0..at.len() as u32);
        order.sort_by_key(|&i| kinds[i as usize]);
        let (mut keys, mut starts) = (crate::code_rule_dsl::facts::rows::vector_in(None, 0), crate::code_rule_dsl::facts::rows::vector_in(None, 0));
        let mut sorted: rerail_blocks::vector::Vector<[u32; 3]> = crate::code_rule_dsl::facts::rows::vector_in(None, n);
        for (i, &o) in order.iter().enumerate() {
            let k = kinds[o as usize];
            if keys.last() != Some(&k) {
                keys.push(k);
                starts.push(i as u32);
            }
            sorted.push(at[o as usize]);
        }
        starts.push(sorted.len() as u32);
        KindRows { keys, starts, at: sorted }
    }
}

/// A span's id (core.rerail's `span`: a piece of a document's text): `d << 32 | granules::extra_low(kind, i)`, in slots of
/// its kind's past every node of its document, so no node view has a row of it (`Batch::node`). `ExtraSpan::Line` `0`
/// is the whole source (`source`), `Line` `l` line `l` (`src_line`), `Comment` `k` the document's `k`-th comment from 1,
/// in source order, of those no node is (`comment`: T's nodes leave out a grammar's `comment` kind, as Python's, but
/// keep Rust's `line_comment`, which is then the comment's id) and `Match` `i` the document's `i`th match or group of
/// a `match_all`, `match_group` or `rx_span` regex. A document with more of a kind than its capacity fails, named.
fn extra_span(key: i64, kind: granules::ExtraSpan, i: u32) -> StoreResult<i64> {
    granules::extra_low(kind, i).map(|low| key << 32 | i64::from(low)).ok_or_else(|| {
        StoreError::Invalid(format!("document {key} has more than {} {kind:?} spans (core.rerail's span ids)", kind.capacity()))
    })
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
    /// Whether the `family`th relation's rows are made (a `len` before then is the count given, decision 0494).
    fn is_made(&self, family: usize) -> bool;
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

pub fn families() -> &'static [&'static dyn Families] {
    FAMILIES.get().map_or(&[], |v| v.as_slice())
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
        // The rows as their kept statistics count them, until they are made (decision 0494); then checked by them.
        let given = self.batch.given_rows(families()[self.at].names()[self.family]);
        let made = self.batch.families[self.at].as_ref().is_none_or(|b| b.is_made(self.family));
        match given {
            Some(n) if !made => n,
            _ => {
                let len = self.rows().len();
                if let Some(n) = given.filter(|&n| n != len) {
                    self.batch.docs.fail(format!("{}: {len} rows made, its kept statistics counted {n} (decision 0494)", families()[self.at].names()[self.family]));
                }
                len
            }
        }
    }
    fn failure(&self) -> Option<String> {
        self.batch.docs.failure()
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
    fn column_counts(&self, c: usize) -> Option<Vec<(i64, u64)>> {
        self.rows().column_counts(c)
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.rows().range_lookup(column, lower, upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
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
    /// `RERAIL_NODE_IDS=preorder`, its preorder index. Rules that order nodes read it (`lowering_tail::OrderNodes`).
    Order,
    /// `next_sibling(a, b)`: `b` is the numbered direct child of `a`'s parent after `a`, those left out skipped. A
    /// row a node after a sibling (`Item::Sibling`), as `field`'s are its fielded nodes.
    Sibling,
    /// `def_unit(n, u)`: `u` is the definition unit holding `n` (`units`: the innermost function or class, a
    /// decorator's nodes its definition's), the document's root where none does; the root has none (`-1`). Rows laid
    /// out as `node`'s, read off the granule table.
    Unit,
    /// `op(n, s)`: `s` is the text of a token of node `n` (an anonymous child the tape stores with text): a row a token
    /// (`Item::Token`), its kind's name's atom (`Batch::op_atom`), no text read. The rows the tape does not number (a
    /// zero-width child, Python's two-word operators) are made beside it (`ops::join`).
    Op,
    /// `end_point(n, p)`: `p` is node `n`'s end point, the zero-width span at its end (`Locs::ends`, one a node, in
    /// local order): a row a node, laid out as `node`'s, where a program reads it. Its id is `n`'s place among the
    /// positions, so nothing is read but the ids.
    End,
    /// `spread(n)`: the spread nodes (`*args`, `...xs`; `Item::Spread`), in preorder, where a program reads it.
    Spread,
    /// core.rerail's token order (`tokens`): a place's first or last token, the token before or after it, read off the
    /// rank index, a row a place that has one, where a program reads it.
    Tok(tokens::TokRel),
    /// `text(n, s)`: `s` is the text of node or position `n` (decision 0656 step 4): a row a node with text
    /// (`Item::Text`, `core_rr::TextRows`), then one a position with text (`TextSpans`: its tokens, source, lines).
    /// `s` is its text's atom (`Projected::text_atom`), a leaf's by its term; a read by `s` is routed to the documents
    /// whose index can hold it (`Batch::text_matches`). No row is made; where `Extra` makes spans with text, theirs
    /// follow the view's (`places::join`).
    Text,
    /// `core.rerail`'s place of a node or span (`loc`): `line(n, l)`, `end_line(n, l)`, `col(n, c)`, `docof(n, d)`,
    /// read from the document's lines (`LineTable::place`), rows laid out over `Projected::locs`. A batch that makes
    /// one of them as rows (`Extra`, whose spans have places too) binds those rows instead (`Batch::made`).
    Loc(Place),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Line,
    EndLine,
    Col,
    Docof,
    Report,
}

/// The relations of core.rerail a batch makes only for a program that reads them (its `reads`), T's exporter having
/// had none of them, so a program that reads none does no work and interns no text:
///
/// ```text
/// source(d, sp)          src_line(d, l, sp)     line_count(d, k)       node_text(n, s)
/// end_line(n, l)         col(n, c)              docof(n, d)
/// ```
///
/// with `sp` a span (`SPAN`). A file's lines are its facts' (`LineTable`, its language's line ends, decision 0612):
/// `src_line` each of them, the empty line after a final terminator included, and `line_count` the lines a linter
/// counts (that empty line left out). Every node, token and span is placed by its bytes on those lines
/// (`LineTable::place`): `line` the line of its start, `end_line` the line holding its end offset (a node ending with a
/// line terminator, as a module may, ends on the line after it), `col` its start's byte column. Spans have rows in
/// `text` (their text, terminators left out), `line`, `end_line` and `col`; `node_text` is a node's source text,
/// exactly.
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
/// In `core.rerail`'s shape (`Shape::Core`) a relation made here is the batch's, in place of `core_rr`'s rows or the `Loc`
/// view of that name (`Batch::build`), and its node rows cover that shape's anonymous tokens too (`Locs`); `comment`
/// is `core_rr`'s there (comments are nodes), so it is not made here.
///
///
/// And each `matchall_<k>(sp, m)` read (`preparation::text::TextRelations`, core.rerail's `match_all`): `m` each match of
/// regex `k` in span `sp`'s text, leftmost-first and non-overlapping (Rust's `find_iter`, as W's `regex` reads a
/// file), over every span above. A match is a span (`MATCH`) with rows in `text` (what it matched), `line`,
/// `end_line` and `col`, placed by its bytes as every span is (in a line, that line and the byte column in it).
///
/// And in `core.rerail`'s shape, `rx_span(p, c, j, g, m)` where read (`rr::lower`'s `match_all` and `match` over a span):
/// group `g` of match `j` (from 0) of regex `p` in span `c` is span `m` (`MATCH`, numbered with `match_all`'s), placed
/// as `match_all`'s. A span `c` is a document's `source` span, one of its `src_line` spans, a comment node, a node of a
/// kind, or a span an earlier pattern's group captured (`SpanIn`, decision 0081). Group "" is the whole match; a group
/// that takes no part in a match has no row. The spans are those `source` and `src_line` give here, so the two join.
#[derive(Default)]
struct Extra {
    source: bool,
    src_line: bool,
    line_count: bool,
    text: bool,
    /// `text` of the `source` and `src_line` spans: core.rerail's programs make it where a rule reads a copy of such a
    /// span's text (`Projection::wants_span_text`), T's wherever `text` is read.
    source_text: bool,
    line_text: bool,
    line: bool,
    end_line: bool,
    col: bool,
    node_text: bool,
    docof: bool,
    /// Whether the nodes' and tokens' places (`line`, `end_line`, `col`, `docof`) are made as rows: in core.rerail's
    /// shape they are the `Loc` view's, and only the spans' are rows (`places`, decision 0353).
    node_places: bool,
    value: bool,
    /// `rr_value_lit`: the literal values of documents W reads no values of, where `value` is answered on request
    /// (decision 0174): kept eager, as they are few.
    value_lit: bool,
    static_value: bool,
    value_type_of: bool,
    value_type: bool,
    comment: bool,
    sources: Vector<[i64; 2]>,
    src_lines: Vector<[i64; 3]>,
    line_counts: Vector<[i64; 2]>,
    lines: Vector<[i64; 2]>,
    end_lines: Vector<[i64; 2]>,
    cols: Vector<[i64; 2]>,
    node_texts: Vector<[i64; 2]>,
    docofs: Vector<[i64; 2]>,
    /// The document's file's key (`Projected::file`), the `docof` of its spans: set per document.
    file: i64,
    values: Vector<[i64; 2]>,
    value_lits: Vector<[i64; 2]>,
    static_values: Vector<[i64; 2]>,
    value_types_of: Vector<[i64; 2]>,
    value_types: BTreeSet<[i64; 2]>,
    comments: Vector<[i64; 2]>,
    /// The regexes of the `match_all` and `match_group` relations read (`Found`).
    matches: Vec<Found>,
    rx_span: bool,
    rx_spans: Vector<[i64; 5]>,
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
    rows: Vector<[i64; 2]>,
    group_rows: Vector<[i64; 4]>,
}

/// A span made for a document: its id and its bytes.
type MadeSpan = (i64, std::ops::Range<usize>);

impl Extra {
    fn new(reads: &BTreeSet<String>, matches: &[Arc<regex::Regex>], shape: Shape, [source_text, line_text]: [bool; 2]) -> Self {
        let mut extra = Self::made(reads, matches, shape, [source_text, line_text]);
        // In core.rerail's shape a node's and a position's places are views of the tape (`View::Loc`): made as rows only
        // where spans are, which have places too (decision 0353 step 3). Made for every node and token beside the
        // views, they were hogql's 0.1 s and 255 MB a run, for a program that read no span.
        if shape == Shape::Core && !extra.spans() {
            (extra.end_line, extra.col, extra.docof) = (false, false, false);
        }
        extra
    }

    /// What the program's reads ask `Extra` to make.
    fn made(reads: &BTreeSet<String>, matches: &[Arc<regex::Regex>], shape: Shape, [source_text, line_text]: [bool; 2]) -> Self {
        let read = |rel: &str| reads.contains(rel);
        Self {
            source: read("source"),
            src_line: read("src_line"),
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
            value_lit: read("rr_value_lit"),
            static_value: read("static_value"),
            value_type_of: read("value_type_of"),
            value_type: read("value_type"),
            comment: shape == Shape::T && read("comment"),
            rx_span: shape == Shape::Core && read("rx_span"),
            node_places: shape != Shape::Core,
            matches: matches
                .iter()
                .enumerate()
                .map(|(k, regex)| Found {
                    k,
                    regex: Arc::clone(regex),
                    all: read(&format!("{}{k}", crate::code_rule_dsl::logic::strings::MATCHES)),
                    groups: read(&format!("{}{k}", crate::code_rule_dsl::logic::strings::GROUPS)),
                    rows: rerail_blocks::vector::Vector::new(),
                    group_rows: rerail_blocks::vector::Vector::new(),
                })
                .filter(|found| found.all || found.groups)
                .collect(),
            ..Self::default()
        }
    }

    /// Whether spans are made (their matches' too), with places in `line`, `end_line` and `col`.
    fn spans(&self) -> bool {
        self.source || self.src_line || self.comment || self.rx_span
    }

    /// Whether `line` is made as rows rather than read off the node view: spans have lines too (in core.rerail's shape
    /// the spans' alone, after the `Loc` view's rows: `places::Places`).
    fn line_rows(&self) -> bool {
        self.line && self.spans()
    }

    /// A span's rows in `text` (into `text`, with `text_too`), `line`, `end_line` and `col` (its start's byte column).
    #[allow(clippy::too_many_arguments)]
    fn span(&mut self, sp: i64, first: usize, last: usize, col: usize, body: &str, text_too: bool, text: &mut Vector<[i64; 2]>, string: &mut dyn FnMut(&str) -> usize) {
        if self.docof {
            self.docofs.push([sp, self.file]);
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
    /// `docof` of `core.rerail`'s anonymous tokens (`Locs`), which follow the nodes. A guest's nodes are placed in its file
    /// (`Locs::place`, decision 0055).
    fn nodes(&mut self, p: &Projected, string: &mut dyn FnMut(&str) -> usize) {
        let f = p.f();
        let ends = p.extra.as_ref().map_or(0..0, |x| p.len() + x.ends_from()..p.len() + x.ends_from() + x.ends);
        let guest = p.extra.as_deref().filter(|x| x.host.is_some());
        // Every place by its bytes (`LineTable::place`): a guest's in its host's lines.
        let place = |span: [u32; 2]| match guest {
            Some(x) => x.place(span),
            None => f.lines.place(span),
        };
        // In core.rerail's shape a node's place is the `Loc` view's (`places`): only its text is a row here.
        let (line, end_line, col) = (self.line_rows() && self.node_places, self.end_line && self.node_places, self.col && self.node_places);
        let mut placed = |n: i64, span: [u32; 2]| {
            let (first, last, c) = place(span);
            if line {
                self.lines.push([n, i64::from(first)]);
            }
            if end_line {
                self.end_lines.push([n, i64::from(last)]);
            }
            if col {
                self.cols.push([n, i64::from(c)]);
            }
        };
        if line || end_line || col {
            for (_, id, n) in p.node_ids() {
                placed(n, [f.start(id), f.end(id)]);
            }
            for (local, _, n) in p.token_ids() {
                placed(n, p.span(local));
            }
            // Each node's end point (`end_point`, where a program reads it): placed where the node ends.
            for local in ends.clone() {
                placed(p.global(local), p.span(local));
            }
        }
        if self.node_text {
            for (_, id, n) in p.node_ids() {
                self.node_texts.push([n, string(f.text(id)) as i64]);
            }
        }
        if self.docof && self.node_places {
            let key = i64::from(p.file);
            self.docofs.extend(p.node_ids().chain(p.token_ids()).map(|(_, _, n)| n).chain(ends.map(|local| p.global(local))).map(|n| [n, key]));
        }
    }

    /// `core_rr`'s literal values (n, v, v's type), of documents W reads no values of (`Dialect::values`), in the
    /// value relations made here: `value` and `static_value` (no names to follow), and `value_type`.
    fn literal_values(&mut self, rows: impl Iterator<Item = [i64; 3]>) {
        for [n, v, t] in rows {
            if self.value {
                self.values.push([n, v]);
            }
            if self.value_lit {
                self.value_lits.push([n, v]);
            }
            if self.static_value {
                self.static_values.push([n, v]);
            }
            if self.value_type {
                self.value_types.insert([v, t]);
            }
        }
    }

    /// Document `p`'s W values, read off its tape (`Document::tape`): a file's, or an embedded program's that keeps
    /// one (a Vue view, decision 0222).
    fn values(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<()> {
        let f = p.f();
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
                let local = p.local_of_node(ordinal);
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
        Ok(())
    }

    /// Document `p`'s rows (its `Document` `d`, its grammar's kinds `dialect`, none for a text-only document);
    /// `rx`: `rx_span`'s patterns, the spans each
    /// runs over and its block program, and `ends` (by pattern) the bytes of the document's source a match can end
    /// after, where the batch ran the pattern's block program (decision 0081 §3).
    #[allow(clippy::too_many_arguments)]
    fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, rx: &[core_rr::SpanPattern<'_>], ends: &[Option<Vector<u64>>], text: &mut Vector<[i64; 2]>, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<()> {
        let f = p.f();
        self.values(p, d, dialect, string)?;
        self.nodes(p, string);
        self.file = i64::from(p.file);
        let key = i64::from(p.key);
        use granules::ExtraSpan;
        let span = |kind: ExtraSpan, i: usize| extra_span(key, kind, u32::try_from(i).unwrap_or(u32::MAX));
        let source = f.source.as_str();
        if self.comment && let Some(dialect) = dialect {
            use crate::code_rule_dsl::language::is_comment_kind;
            let mut k = 0;
            for (id, up) in f.with_parents(0, f.len() as u32) {
                // A comment kind's node, not inside another (Rust's doc comment parts are not comments).
                if !f.named(id) || !is_comment_kind(dialect.name(f, id)) || up.is_some_and(|q| is_comment_kind(dialect.name(f, q))) {
                    continue;
                }
                let whole = f.text(id);
                let c = match p.local_of_node(id) {
                    NONE => {
                        k += 1;
                        let c = span(ExtraSpan::Comment, k)?;
                        let (first, last, col) = f.lines.place([f.start(id), f.end(id)]);
                        self.span(c, first as usize, last as usize, col as usize, whole, true, text, string);
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
        // Its lines are the facts' (`LineTable`, the language's): the one definition of a line and of a place.
        let lines = &f.lines;
        if self.line_count {
            self.line_counts.push([key, lines.counted() as i64]);
        }
        let mut made: Vec<MadeSpan> = Vec::new();
        // `rx_span`'s containers among them: the source's and the lines' (indexes in `made`).
        let (mut source_at, mut lines_at) = (None, 0..0);
        if self.source {
            let sp = span(ExtraSpan::Line, 0)?;
            self.sources.push([key, sp]);
            let (first, last, col) = lines.place([0, source.len() as u32]);
            self.span(sp, first as usize, last as usize, col as usize, source, self.source_text, text, string);
            source_at = Some(made.len());
            made.push((sp, 0..source.len()));
        }
        if self.src_line {
            lines_at.start = made.len();
            for l in 1..=lines.count() {
                let sp = span(ExtraSpan::Line, l)?;
                let [a, b] = lines.text_of(l, source.as_bytes()).map(|at| at as usize);
                self.src_lines.push([key, l as i64, sp]);
                self.span(sp, l, l, 0, &source[a..b], self.line_text, text, string);
                made.push((sp, a..b));
            }
            lines_at.end = made.len();
        }
        // Each regex's matches in each span, and their named groups, numbered through the document.
        let mut next: usize = 0;
        let mut id = |_: &Projected| -> StoreResult<i64> {
            next += 1;
            span(ExtraSpan::Match, next - 1)
        };
        // A piece of span `r` (`at` within it): placed by its bytes, as every span is.
        let place = |r: &std::ops::Range<usize>, at: std::ops::Range<usize>| {
            let (first, last, col) = lines.place([(r.start + at.start) as u32, (r.start + at.end) as u32]);
            (first as usize, last as usize, col as usize)
        };
        for k in 0..self.matches.len() {
            let (regex, groups) = (Arc::clone(&self.matches[k].regex), self.matches[k].groups);
            let names: Vec<&str> = regex.capture_names().flatten().collect();
            for (sp, r) in &made {
                let body = &source[r.clone()];
                for captures in regex.captures_iter(body) {
                    let found = captures.get(0).expect("group 0 always matches");
                    let m = id(p)?;
                    self.matches[k].rows.push([*sp, m]);
                    let (first, last, col) = place(r, found.range());
                    self.span(m, first, last, col, found.as_str(), true, text, string);
                    if !groups {
                        continue;
                    }
                    // A group that takes no part in the match has no row.
                    for name in &names {
                        let Some(group) = captures.name(name) else { continue };
                        let g = id(p)?;
                        self.matches[k].group_rows.push([*sp, m, string(name) as i64, g]);
                        let (first, last, col) = place(r, group.range());
                        self.span(g, first, last, col, group.as_str(), true, text, string);
                    }
                }
            }
        }
        // `rx_span`: each pattern over its spans, `j` each match in one (decision 0081). A container is a span made
        // here, a node (a comment, a node of a kind) or a span an earlier pattern's group captured: its id and its bytes.
        type Container = (i64, std::ops::Range<usize>);
        let rx = if self.rx_span { rx } else { &[] };
        let nodes_of = |test: &dyn Fn(&str) -> bool| -> Vec<Container> {
            match dialect {
                Some(dialect) => p
                    .node_ids()
                    .filter(|&(_, x, _)| test(dialect.name(f, x)))
                    .map(|(_, x, n)| (n, f.start(x) as usize..f.end(x) as usize))
                    .collect(),
                None => Vec::new(),
            }
        };
        // The children of the nodes of one kind (`SpanIn::ChildOf`).
        let children_of = |kind: &str| -> Vec<Container> {
            match dialect {
                Some(dialect) => p
                    .walk_up()
                    .zip(p.node_ids())
                    .filter(|&((_, _, up), _)| up.is_some_and(|(_, q)| dialect.name(f, q) == kind))
                    .map(|((_, x, _), (_, _, n))| (n, f.start(x) as usize..f.end(x) as usize))
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
                // The source is scanned once per pattern (decision 0042 §3); a line's, node's or capture's text once
                // per distinct text in the batch (0042 §2, `rx_memo`), its matches placed at each span holding it.
                let (containers, memo): (Vec<Container>, bool) = match within {
                    SpanIn::Source => (source_at.map(|i| made[i].clone()).into_iter().collect(), false),
                    SpanIn::Line => (made[lines_at.clone()].to_vec(), true),
                    SpanIn::Comment => (nodes_of(&core_rr::is_comment), true),
                    SpanIn::Kind(k) => (nodes_of(&|name| name == k.as_str()), true),
                    SpanIn::ChildOf(k) => (children_of(k.as_str()), true),
                    SpanIn::Capture(from, g) => (captured.get(&(from.as_str(), g.as_str())).cloned().unwrap_or_default(), true),
                };
                let pat = string(pattern) as i64;
                let names: Vec<Option<&str>> = re.capture_names().enumerate().map(|(g, n)| if g == 0 { Some("") } else { n }).collect();
                // The bytes a match can end after (decision 0081 §3), from the batch's run of the block programs: a
                // source or line where none can is not scanned.
                let made_ends;
                let ends = match (ends.get(i).and_then(Option::as_ref), prefilter) {
                    (Some(e), _) => Some(e.as_slice()),
                    (None, Some(f)) => {
                        made_ends = f.ends(source.as_bytes());
                        Some(&made_ends[..])
                    }
                    (None, None) => None,
                };
                for (c, r) in &containers {
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
                        let (first, last, col) = place(r, at.clone());
                        self.span(m, first, last, col, &body[at.clone()], true, text, string);
                        captured.entry((*pattern, name)).or_default().push((m, r.start + at.start..r.start + at.end));
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
    fn relations(self, atom: &dyn Fn(i64) -> i64, relation: Make) -> (Vec<(&'static str, Relation)>, Vec<(String, Relation)>) {
        let mut out = Vec::new();
        if self.source {
            out.push(("source", relation(&["d", "sp"], &mut self.sources.iter().map(|r| r.iter().copied().collect::<rerail_blocks::vector::Vector<_>>()))));
        }
        if self.src_line {
            out.push(("src_line", relation(&["d", "l", "sp"], &mut self.src_lines.iter().map(|r| r.iter().copied().collect::<rerail_blocks::vector::Vector<_>>()))));
        }
        if self.line_count {
            out.push(("line_count", relation(&["d", "k"], &mut self.line_counts.iter().map(|r| r.iter().copied().collect::<rerail_blocks::vector::Vector<_>>()))));
        }
        if self.line_rows() {
            out.push(("line", relation(&["n", "l"], &mut self.lines.iter().map(|r| r.iter().copied().collect::<rerail_blocks::vector::Vector<_>>()))));
        }
        if self.end_line {
            out.push(("end_line", relation(&["n", "l"], &mut self.end_lines.iter().map(|r| r.iter().copied().collect::<rerail_blocks::vector::Vector<_>>()))));
        }
        if self.col {
            out.push(("col", relation(&["n", "c"], &mut self.cols.iter().map(|r| r.iter().copied().collect::<rerail_blocks::vector::Vector<_>>()))));
        }
        if self.node_text {
            out.push(("node_text", relation(&["n", "s"], &mut self.node_texts.iter().map(|&[n, s]| rerail_blocks::vector![n, atom(s)]))));
        }
        if self.docof {
            out.push(("docof", relation(&["n", "d"], &mut self.docofs.iter().map(|r| r.iter().copied().collect::<rerail_blocks::vector::Vector<_>>()))));
        }
        if self.value {
            out.push(("value", relation(&["n", "v"], &mut self.values.iter().map(|&[n, v]| rerail_blocks::vector![n, atom(v)]))));
        }
        if self.value_lit {
            out.push(("rr_value_lit", relation(&["n", "v"], &mut self.value_lits.iter().map(|&[n, v]| rerail_blocks::vector![n, atom(v)]))));
        }
        if self.static_value {
            out.push(("static_value", relation(&["n", "v"], &mut self.static_values.iter().map(|&[n, v]| rerail_blocks::vector![n, atom(v)]))));
        }
        if self.value_type_of {
            out.push(("value_type_of", relation(&["n", "t"], &mut self.value_types_of.iter().map(|&[n, t]| rerail_blocks::vector![n, atom(t)]))));
        }
        if self.value_type {
            out.push(("value_type", relation(&["v", "t"], &mut self.value_types.iter().map(|&[v, t]| rerail_blocks::vector![atom(v), atom(t)]))));
        }
        if self.comment {
            out.push(("comment", relation(&["n", "s"], &mut self.comments.iter().map(|&[n, s]| rerail_blocks::vector![n, atom(s)]))));
        }
        if self.rx_span {
            out.push(("rx_span", relation(&["p", "c", "j", "g", "m"], &mut self.rx_spans.iter().map(|&[p, c, j, g, m]| rerail_blocks::vector![atom(p), c, j, atom(g), m]))));
        }
        let mut matched = Vec::new();
        for found in self.matches {
            if found.all {
                matched.push((format!("{}{}", crate::code_rule_dsl::logic::strings::MATCHES, found.k), relation(&["sp", "m"], &mut found.rows.iter().map(|r| r.iter().copied().collect::<rerail_blocks::vector::Vector<_>>()))));
            }
            if found.groups {
                let rows = &mut found.group_rows.iter().map(|&[sp, m, g, gsp]| rerail_blocks::vector![sp, m, atom(g), gsp]);
                matched.push((format!("{}{}", crate::code_rule_dsl::logic::strings::GROUPS, found.k), relation(&["sp", "m", "g", "gsp"], rows)));
            }
        }
        (out, matched)
    }
}

/// The small relations of a document's nodes, with string indexes: `text(n, s)`, `intval(n, v)`, `op(n, s)` (T's,
/// also bound as `op_t`), and the text relations (decision 0037) `name(n, s)` (an identifier's text, an attribute's
/// dotted text), `str(n, s)` (a string literal's content, between its quotes) and `strprefix(n, p)` (its prefix,
/// lowercased). In `core.rerail`'s shape only `intval` and T's `op`: `core_rr` makes the others as `core.rerail` states them.
#[derive(Default)]
struct NodeRows {
    text: Vector<[i64; 2]>,
    intval: Vector<[i64; 2]>,
    op: Vector<[i64; 2]>,
    name: Vector<[i64; 2]>,
    strs: Vector<[i64; 2]>,
    strprefix: Vector<[i64; 2]>,
}

impl NodeRows {
    /// The bytes of its rows so far (a copy's size, decision 0353's row 7).
    fn bytes(&self) -> usize {
        std::mem::size_of_val(self.text.as_slice()) + std::mem::size_of_val(self.intval.as_slice()) + std::mem::size_of_val(self.op.as_slice()) + std::mem::size_of_val(self.name.as_slice()) + std::mem::size_of_val(self.strs.as_slice()) + std::mem::size_of_val(self.strprefix.as_slice())
    }
}

impl NodeRows {
    /// Document `p`'s rows (`dialect` its grammar's kinds; `core`: in `core.rerail`'s shape).
    fn document(&mut self, p: &Projected, dialect: &Dialect, core: bool, string: &mut dyn FnMut(&str) -> usize) {
        let f = p.f();
        for (_, id, n) in p.node_ids() {
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
                    if let Some(&o) = f.children(id).iter().find(|&&c| operator == Some(f.field(c))) {
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
    /// The batch of `documents` (their keys distinct), none of them made (decision 0494): a document's projection is
    /// made on its first read, its rows in a group with those the read needs (`ensure`). What a group makes is named
    /// here, by an empty group's relations. `wave` runs the program's `ast` matchers over a group's tapes. `given`:
    /// documents' views' counts, by key, as the documents' table keeps them (decision 0494), which the views' row
    /// space is counted from where given; a document made whose counts differ fails, named.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: &ContentStore,
        documents: Vec<impl Into<Input>>,
        projection: &Projection,
        matches: &[Arc<regex::Regex>],
        markers: &[String],
        metal: bool,
        wave: Option<(Matchers, crate::code_rule_dsl::facts::nearest::Binding)>,
        given: &HashMap<u32, made::ViewCounts>,
    ) -> StoreResult<Arc<Self>> {
        Self::add(&mut Growth::default(), store, inputs(documents), projection, matches, markers, metal, wave, given, false).map(|(batch, _)| batch)
    }

    /// A batch with every document made, as one group, and that group's rows: the small relations, the `match_all`
    /// relations apart, and the batch's distinct strings and their symbols (`make_rows`).
    #[allow(clippy::type_complexity)]
    pub fn build(
        store: &ContentStore,
        documents: Vec<impl Into<Input>>,
        projection: &Projection,
        matches: &[Arc<regex::Regex>],
        markers: &[String],
        metal: bool,
    ) -> StoreResult<(Arc<Self>, Vec<(&'static str, Relation)>, Vec<(String, Relation)>, (rerail_blocks::maps::StringsView, rerail_blocks::maps::LogView<u32>))> {
        let batch = Self::new(store, documents, projection, matches, markers, metal, None, &HashMap::new())?;
        let made = batch.make_rows(&(0..batch.docs.len()).collect::<Vec<_>>(), true, &[])?;
        let vocabulary = {
            let shared = batch.shared.lock();
            (shared.strings.view(), shared.atoms.view())
        };
        Ok((batch, made.relations, made.matched, vocabulary))
    }

    /// Document `doc`'s file (decision 0431 §4): a program's is its file's, any other document's its own key. Read
    /// from its input, so no document is made for it.
    fn file(&self, doc: usize) -> u32 {
        Guest::file_key(self.docs.guest(doc).filter(|_| self.shape == Shape::Core), self.docs.key(doc))
    }

    /// Whether the batch's documents are made by its probes: one given its documents' counts (decision 0494), whose
    /// row space is known without making them. Its text tests are then probed, not scanned.
    pub fn lazy(&self) -> bool {
        self.docs.given_all()
    }

    /// Whether every document's views' counts were given (decision 0494): none is counted anew, so none is kept.
    pub fn counts_given(&self) -> bool {
        self.docs.given_every()
    }

    /// How many of the batch's documents are made, of how many it holds (decision 0494): a run makes those it reads.
    pub fn documents_made(&self) -> (usize, usize) {
        (self.docs.made_count(), self.docs.len())
    }

    /// Each document's views' counts, by key: the documents' table keeps them (decision 0494). A document neither made
    /// nor given counts is made for them.
    pub fn view_counts(&self) -> Vec<(u32, made::ViewCounts)> {
        (0..self.docs.len()).map(|i| (self.docs.key(i), self.docs.counts(i))).collect()
    }

    /// `view_counts`, each with its document's path, digest (`Document::digest`) and facts' size: what the
    /// documents' table keeps (`retain::put_base_counts`). No document is made for them but one neither made nor
    /// given counts.
    pub fn kept_counts(&self) -> Vec<made::Kept> {
        (0..self.docs.len())
            .map(|i| made::Kept { key: self.docs.key(i), path: self.docs.path(i).to_owned(), digest: self.docs.digest(i), counts: self.docs.counts(i), size: self.docs.size(i), file: self.docs.guest(i).is_none(), guest: self.docs.guest(i).filter(|g| matches!(g.made, Made::Host | Made::File)).map(|g| (Hosted { host: g.host, file: g.file, made: g.made.clone() }, self.docs.tag(i))) })
            .collect()
    }

    /// Each relation's rows, by name, as its kept statistics count them (decision 0494): a relation's `len` before
    /// every document is made. Once every one is, a relation whose rows are not its count fails, named.
    ///
    /// A place relation (`places::Places`) is the `Loc` view's rows, then `Extra`'s spans': its count is of both, so
    /// its span rows are the count less the view's, which its documents' counts give with none made. Given whole, the
    /// span rows were counted the relation's every row, its `len` the view's rows more than its walk read.
    pub fn give_totals(&self, totals: &HashMap<String, usize>) {
        let mut rows = totals.clone();
        if self.shape == Shape::Core {
            for (name, place) in named_views(self.shape) {
                if places::NAMES.contains(&name)
                    && self.made.contains(name)
                    && let Some(n) = rows.get_mut(name)
                {
                    *n = n.saturating_sub(*self.offsets(place).last().unwrap());
                }
            }
        }
        self.rows.give_totals(&rows);
        let _ = self.given_rows.set(rows);
    }

    /// Each column's distinct values of relation `rel` (`None` for one its rules do not read), as its kept statistics
    /// count them, of the rows `give_totals` gave: a probe's estimate before its rows are made.
    pub fn give_distinct(&self, rel: &str, distinct: Vec<Option<usize>>) {
        self.rows.give_distinct(rel, distinct);
    }

    /// Relation `rel`'s rows as `give_totals` gave them.
    fn given_rows(&self, rel: &str) -> Option<usize> {
        self.given_rows.get().and_then(|t| t.get(rel).copied())
    }

    /// The batch `grown` holds with `documents` added (decision 0431 §6), none of them made (decision 0494). The batch
    /// it grew last is made complete first: each of its relations is one block relation, which this one's groups go on
    /// in, so its rows are a prefix the device binds with the rest. The batch made holds every document, the earlier
    /// ones' shared, as made there. `alone`: a tape value admitted inside a chunk's fixpoint (decision 0431 §8), a
    /// batch of its own, made at once, its hosts the chunk's: the rows returned, staged to follow the chunk's. A value
    /// the growth holds, given again, has the rows of its new places alone.
    #[allow(clippy::too_many_arguments)]
    fn add(
        grown: &mut Growth,
        store: &ContentStore,
        documents: Vec<Input>,
        projection: &Projection,
        matches: &[Arc<regex::Regex>],
        markers: &[String],
        metal: bool,
        wave: Option<(Matchers, crate::code_rule_dsl::facts::nearest::Binding)>,
        given: &HashMap<u32, made::ViewCounts>,
        alone: bool,
    ) -> StoreResult<(Arc<Self>, Option<made::Made>)> {
        let shape = projection.shape;
        // A session's last batch (decision 0616), where its projection, regexes and markers are this one's: its
        // strings, atoms and producers are this batch's, and its files' lines.
        let renew = grown.renew.take().filter(|h| !alone && grown.last.is_none() && same_projection(&h.projection, projection) && h.markers == markers && h.matches.iter().map(|r| r.as_str()).eq(matches.iter().map(|r| r.as_str())));
        if let Some(h) = &renew {
            grown.shared = Arc::clone(&h.shared);
            grown.lines_of = Arc::clone(h.docs.lines_of());
        }
        let last = grown.last.clone();
        if let Some(last) = &last {
            if !last.is_complete() {
                crate::code_rule_dsl::logic::profile::decision("a batch's every document made by a read of a relation no document routes (decision 0494)", || "the batch grows (decision 0431 §6)".to_owned());
            }
            last.complete();
            if let Some(why) = last.docs.failure() {
                return Err(StoreError::Invalid(why));
            }
        }
        // A document the growth holds is a tape value given more places (decision 0431 §6): the rows of those alone.
        let holds = |k: u32| grown.values.contains_key(&k) || last.as_ref().is_some_and(|b| b.docs.position(k).is_some());
        let (held, mut documents): (Vec<Input>, Vec<Input>) = documents.into_iter().partition(|d| holds(d.key()));
        documents.sort_by_key(Input::key);
        if documents.windows(2).any(|w| w[0].key() == w[1].key()) {
            return Err(StoreError::Invalid("a batch holds a document key twice".into()));
        }
        if let Some(d) = held.iter().find(|d| !grown.values.contains_key(&d.key())) {
            return Err(StoreError::Invalid(format!("{}: a document the batch holds, given again, is not a tape value", d.path())));
        }
        if !alone && !documents.is_empty() && !grown.values.is_empty() {
            return Err(StoreError::Invalid("a chunk's batch grows by a round after a tape value was admitted: the values' rows follow the chunk's (decision 0431 §8)".into()));
        }
        // A document's tag, size and place are known before it is made (`Pending`).
        for d in &documents {
            if d.tag() != TEXT {
                grown.dialects.get_or_new(d.tag())?;
            }
        }
        let dialects = Arc::new(grown.dialects.clone());
        // The batch's block: what the projections can take, the chunk's copies of the facts' arrays (4 bytes a node for
        // each of `parent`, `start`, `end`, `kind`, `core.rerail`'s places; `field`'s 2), its tables, and the starts of the
        // axes the device solves (4 bytes a node; their rows are relations of their own, decision 0353 step 5). The
        // block reserves address space, not memory.
        // A batch that grows (decision 0431 §6) projects its later documents into the same block: the gigabyte past
        // its first documents' needs is address space for them.
        let block = Arc::clone(grown.block.get_or_insert_with(|| cells::block((documents.iter().map(|d| block_bytes(d.size())).sum::<usize>() + (1 << 20) + (1 << 30)).next_power_of_two())));
        // Decision 0055: each guest's host. A guest's key is past every file's (`corpus::guests`).
        grown.host.extend(documents.iter().filter_map(|d| d.guest().map(|g| (d.key(), g.host))));
        // The documents' 16z blocks' cache: the batch's, shared as its block is (`batch_cache`).
        // Of the documents made: one made when first read is read through it where its blocks fit (`Projected::narrowed`).
        grown.cache = batch_cache(grown.cache.as_ref(), documents.iter().filter(|d| d.is_made()).filter_map(|d| d.document().facts.z_tape()));
        // W's values on request (decision 0174): the documents W reads values of, a dialect's with values, each read off
        // its tape when first asked (`Valued`): none made for it here (a `Pending` stays so, Q332).
        let added_valued: Vec<Option<Arc<Valued>>> = if projection.derived.contains(VALUE_REQ) {
            documents.iter().map(|d| dialects.get(d.tag()).is_some_and(|x| x.values).then(|| Arc::new(Valued { values: OnceLock::new() }))).collect()
        } else {
            Vec::new()
        };
        let projection = Arc::new(projection.clone());
        let context = made::Context {
            cache: grown.cache.clone(),
            dialects: Arc::clone(&dialects),
            projection: Arc::clone(&projection),
            block: Arc::clone(&block),
            host: grown.host.clone(),
            lines_of: Arc::clone(&grown.lines_of),
            failure: Default::default(),
        };
        let mut renewed_at: Option<Vec<Option<usize>>> = None;
        let (docs, valued, prior_at) = match (&last, alone) {
            (Some(last), false) => {
                let (docs, at) = made::Docs::grown(&last.docs, documents, context, given);
                // Each document's tape for W's values, in the batch's order: the earlier ones' as held.
                let valued = if last.valued.is_empty() && added_valued.is_empty() {
                    Vec::new()
                } else {
                    let mut valued: Vec<Option<Arc<Valued>>> = vec![None; docs.len()];
                    for (k, &i) in at.iter().enumerate() {
                        valued[i] = last.valued.get(k).cloned().flatten();
                    }
                    let mut added = added_valued.into_iter();
                    for (i, v) in valued.iter_mut().enumerate() {
                        if at.binary_search(&i).is_err() {
                            *v = added.next().flatten();
                        }
                    }
                    valued
                };
                (docs, valued, Some(at))
            }
            _ => match &renew {
                Some(h) => {
                    let (docs, at) = made::Docs::renewed(&h.docs, documents, context, given);
                    renewed_at = Some(at);
                    (docs, added_valued, None)
                }
                None => (made::Docs::new(documents, context, given), added_valued, None),
            },
        };
        // Each lazy family's strings, where the batch has documents of its dialect, interned with the batch's.
        let vocabulary: Vec<Option<u8>> = families().iter().map(|f| crate::code_rule_dsl::language::tape_tag(f.dialect()).filter(|&tag| (0..docs.len()).any(|i| docs.tag(i) == tag))).collect();
        let named: Vec<&str> = families().iter().zip(&vocabulary).filter(|(_, v)| v.is_some()).flat_map(|(f, _)| f.strings().iter().copied()).collect();
        let mut atoms = store.intern_atoms(&named)?.into_iter();
        let bound: Vec<Option<Box<dyn BoundFamilies>>> = families()
            .iter()
            .zip(vocabulary)
            .map(|(f, v)| {
                v.map(|tag| {
                    let strings: Vector<i64> = f.strings().iter().map(|_| i64::from(atoms.next().expect("a family string's atom"))).collect();
                    f.bind(tag, &dialects[tag].catalog, &strings, docs.len())
                })
            })
            .collect();
        let n = docs.len();
        let mut of_tag: [Vector<u32>; 256] = std::array::from_fn(|_| rerail_blocks::vector::Vector::new());
        for doc in 0..n {
            of_tag[usize::from(docs.tag(doc))].push(doc as u32);
        }
        // The programs among the documents, by file (decision 0431 §4): a program's file is its guest's (`Guest::file`).
        let mut programs: Vec<(u32, usize)> = (0..n).filter_map(|i| {
            let file = Guest::file_key(docs.guest(i).filter(|_| shape == Shape::Core), docs.key(i));
            (file != docs.key(i)).then_some((file, i))
        }).collect();
        programs.sort_unstable();
        let mut batch = Batch {
            docs,
            programs,
            valued,
            dialects,
            offsets: made::Offsets::default(),
            kinds: (0..=u8::MAX).filter(|&t| !of_tag[usize::from(t)].is_empty()).map(|t| (t, OnceLock::new())).collect(),
            of_tag,
            shape,
            families: bound,
            made: BTreeSet::new(),
            block,
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            chunk: OnceLock::new(),
            projection,
            matches: matches.to_vec(),
            markers: markers.to_vec(),
            metal,
            atoms: store.atoms_handle(),
            wave,
            rows: made::Rows::new(n, Vec::new(), Vec::new(), Vec::new(), None, &HashMap::new()),
            given_rows: OnceLock::new(),
            by_path: OnceLock::new(),
            guests_of: OnceLock::new(),
            by_file_key: OnceLock::new(),
            op_atoms: (0..256).map(|_| OnceLock::new()).collect(),
            by_module: OnceLock::new(),
            shared: Arc::clone(&grown.shared),
            outer: if alone { last.clone() } else { None },
            keeps_strings: grown.keeps_strings && !alone,
            renewed: renew.is_some(),
        };
        // What a group makes: an empty group's relations, which answer what each is before its rows are made.
        let empty = batch.make_rows(&[], true, &[])?;
        grown.made.extend(empty.read.iter().copied());
        batch.made = grown.made.clone();
        let statics: Vec<&'static str> = empty.relations.iter().map(|(name, _)| *name).collect();
        let template: Vec<(String, Arc<Relation>)> = empty
            .relations
            .into_iter()
            .map(|(name, r)| (name.to_owned(), Arc::new(r)))
            .chain(empty.matched.into_iter().map(|(name, r)| (name, Arc::new(r))))
            .chain(empty.waved.into_iter().map(|(name, r)| (name, Arc::new(r))))
            .collect();
        let (names, template): (Vec<String>, Vec<Arc<Relation>>) = template.into_iter().unzip();
        let prior = match (&last, prior_at) {
            (Some(last), Some(at)) => Some((last.rows.whole(), at)),
            _ => None,
        };
        let node_columns: HashMap<String, Vec<usize>> = batch
            .wave
            .as_ref()
            .map(|(m, _)| m.node_columns())
            .unwrap_or_default()
            .into_iter()
            .chain(guests::OWN_NODES.iter().map(|&(name, cols)| (name.to_owned(), cols.to_vec())))
            .collect();
        batch.rows = made::Rows::new(n, names, template, statics, prior.as_ref().map(|(whole, at)| (whole, at.clone())), &node_columns);
        if let (Some(h), Some(at)) = (&renew, &renewed_at) {
            let [groups, docs, runs, left] = batch.rows.take(&h.rows, at);
            let shared = at.iter().filter(|a| a.is_some()).count();
            crate::code_rule_dsl::logic::profile::decision("a session's corpus batch renewed (decision 0616)", || {
                format!("{groups} of {} groups taken, {docs} documents, {runs} runs; {left} groups left past the segment limit; {shared} of {} documents' slots shared", h.rows.groups(), h.docs.len())
            });
        }
        let batch = Arc::new(batch);
        #[cfg(any(test, feature = "test-hooks"))]
        BATCHES.lock().push((batch.docs.keys().iter().copied().collect::<Vec<_>>(), Arc::downgrade(&batch)));
        if !alone {
            grown.last = Some(Arc::clone(&batch));
            return Ok((batch, None));
        }
        // A tape value alone: made at once, its rows staged to follow the chunk's; one held given more places, theirs.
        let mut places: Vec<(&Projected, &Document, std::ops::Range<usize>, bool)> = Vec::new();
        for d in held.iter().map(Input::document) {
            let (Some(guest), Some((value, had, withheld))) = (d.guest.as_deref(), grown.values.get(&d.key)) else { continue };
            // The value's projection in the batch it was admitted with: an answer's values are admitted together.
            if let Made::Tape { places: all } = &guest.made
                && all.len() > *had
                && let Some(at) = value.docs.position(d.key)
            {
                places.push((&value.docs[at], d, *had..all.len(), *withheld));
            }
        }
        let made = batch.make_rows(&(0..batch.docs.len()).collect::<Vec<_>>(), false, &places)?;
        let grew: Vec<(u32, usize)> = places.iter().map(|(_, d, at, _)| (d.key, at.end)).collect();
        drop(places);
        for (key, had) in grew {
            if let Some(v) = grown.values.get_mut(&key) {
                v.1 = had;
            }
        }
        for i in 0..batch.docs.len() {
            if let Some(Made::Tape { places }) = batch.docs.guest(i).map(|g| g.made) {
                grown.values.insert(batch.docs.key(i), (Arc::clone(&batch), places.len(), batch.docs.made(i).withheld));
            }
        }
        Ok((batch, Some(made)))
    }

    /// The rows of the documents `group` (indexes, ascending), made together (decision 0494): the batch's small
    /// relations (`doc`, `pkg`, `syntax_error`, `text`, `intval`, `op`), those of core.rerail's the program reads that T
    /// had not (`Extra`), the `match_all` relations among them (of the batch's regexes) apart, and the `ast`
    /// matchers'. A text-only document (`core::TEXT`) has the document's relations and its text's, and no node. Its
    /// strings are the batch's (`Shared`), the new ones interned. `first`: the batch's first rows, written into their
    /// block relations (else staged to go on in them). `held`: tape values the batch's growth holds, given more places
    /// (decision 0431 §6): each one's projection, document, new places and whether it is withheld.
    fn make_rows(&self, group: &[usize], first: bool, held: &[(&Projected, &Document, std::ops::Range<usize>, bool)]) -> StoreResult<made::Made> {
        let projection = &*self.projection;
        let (shape, matches, markers, metal) = (projection.shape, &self.matches[..], &self.markers[..], self.metal);
        let documents: Vec<&Document> = group.iter().map(|&i| self.docs.input(i)).collect();
        // Each document made before the batch's strings are held (`Shared`): a projection, the span prefilter and the
        // matchers may run on the pool, whose waiting thread may run a job that makes another group of this batch.
        for &i in group {
            self.docs.made(i);
        }
        let core = shape == Shape::Core;
        // What the program reads, for the relations made only for a program that reads them (`Extra` and those beside
        // it); a projection that names none (T's, `Projection::core`'s) reads none of them.
        let none = BTreeSet::new();
        let reads = projection.reads.as_ref().unwrap_or(&none);
        // Rows with string indexes; the indexes become atoms once the batch's strings are interned.
        let (mut doc, mut pkg, mut unpackaged) = (Vec::new(), Vec::new(), Vec::new());
        // `path_part(d, s)`: each directory of a document's path and its file name; `file_name(d, s)`: the last.
        let (mut path_part, mut file_name): (Vector<[i64; 2]>, Vector<[i64; 2]>) = (rerail_blocks::vector::Vector::new(), rerail_blocks::vector::Vector::new());
        // `text`, `intval`, T's `op` and the text relations (`NodeRows`).
        let mut rows = NodeRows::default();
        // `syntax_error(d)`: the documents whose parse has an error (`Facts::syntax_errors`); `core.rerail`'s
        // `syntax_error(d, l, msg)` (`core_rr`): the first error's line and message.
        let mut errors = Vec::new();
        // The nodes visited for their scalar rows (`NodeRows::document`'s, `core_rr::CoreRows::node`'s).
        let mut visited = 0usize;
        let span_text = [crate::code_rule_dsl::logic::front::ast::SpanIn::Source, crate::code_rule_dsl::logic::front::ast::SpanIn::Line].map(|within| !core || projection.wants_span_text(within));
        let mut extra = Extra::new(reads, matches, shape, span_text);
        let mut tree = tree::Tree::new(reads);
        let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        // `core.rerail`'s (decision 0046).
        let mut c = core_rr::CoreRows::default();
        // Node identity/ancestry and line views do not require scalar rows or text interning.
        let scalar_nodes=!core || ["name","str","strprefix","value","value_type","static_value","comment","op_at","op_t","intval","str_part","str_part_start","str_part_word"]
            .into_iter().any(|r|projection.wants(r));
        let wants_any = |rels: &[&str]| rels.iter().any(|r| projection.wants(r));
        c.skip = core_rr::Skip {
            ops: !wants_any(&["op_at"]),
            comment: !projection.wants("comment"),
            name: !projection.wants("name"),
            literal: !wants_any(&["str", "strprefix", "value", "static_value", "value_type", "rr_value_lit"]),
            str: !projection.wants("str"),
            literal_value: !wants_any(&["value", "static_value", "value_type", "rr_value_lit"]),
            literal_rows: !wants_any(&["str", "strprefix", "value", "static_value", "value_type", "rr_value_lit"]),
            structural: !wants_any(&["structural_before", "structural_after"]),
        };
        let core_nodes = core && (scalar_nodes || !c.skip.structural);
        // `rr_value_lit` holds the literals of the documents W reads no values of (`unvalued`, `values_of`): where no
        // other relation reads a literal's value or rows, a document W values makes none, and its walk visits only the
        // nodes whose text a rule reads. Made for every node and dropped, hogql's literals had `node rows` visit all
        // 1.3 M nodes.
        let value_lit_only = wants_any(&["rr_value_lit"]) && !wants_any(&["str", "strprefix", "value", "static_value", "value_type"]);
        let wants_long_text_nodes = projection.wants("rr_long_text_node");
        let mut long_text_nodes = Vec::new();
        c.op_nodes = projection.op_nodes.clone();
        // Asked once for the batch, not per node (a lookup in the program's set of read relations).
        let (wants_parts, wants_part_starts, wants_words) = (projection.wants("str_part"), projection.wants("str_part_start"), projection.wants("str_part_word"));
        // Only a program that reads it: every node's hash is a symbol.
        let wants_trees = core && projection.reads.as_ref().is_some_and(|r| r.contains("tree_digest"));
        let wants_units = core && projection.reads.as_ref().is_some_and(|r| r.contains("def_digest"));
        let wants_digests = wants_trees || wants_units;
        // `rx_span`'s regexes, compiled once for the run's projection (`CompiledSpans`).
        let rx_spans: Vec<core_rr::SpanPattern> = if core {
            let compiled = projection.compiled_rx.of(&projection.rx_spans)?;
            projection.rx_spans.iter().zip(compiled).map(|((within, re), (r, prefilter))| (within.clone(), re.as_str(), r, prefilter)).collect()
        } else {
            Vec::new()
        };
        // The AST patterns, compiled once per grammar; one a grammar does not parse matches nothing there.
        let mut ast_patterns: HashMap<u8, Vec<(&str, ast_grep_core::matcher::Pattern)>> = HashMap::new();
        // Every prefiltered pattern's marks over every file, in one batch of block programs (decision 0081 §3): one
        // launch of the executor (decision 0100 §13), on the cores or, under the device binding, the GPU, as its price
        // places it. `span_ends[doc][pattern]`.
        let span_ends: Vec<Vec<Option<Vector<u64>>>> = {
            use crate::code_rule_dsl::facts::span_prefilter::{RanOn, SpanEnds, launch as ends_launch};
            let filters: Vec<(usize, &Arc<dyn SpanEnds>)> = rx_spans.iter().enumerate().filter_map(|(i, (_, _, _, f))| f.as_ref().map(|f| (i, f))).collect();
            let mut out: Vec<Vec<Option<Vector<u64>>>> = vec![vec![None; rx_spans.len()]; documents.len()];
            if !filters.is_empty() && !documents.is_empty() {
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
                let (ends, ran) = ends_launch(&crate::code_rule_dsl::facts::core::executor_for(metal), &programs, &sources, &files, metal).map_err(StoreError::Invalid)?;
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
        // The batch's strings (`Shared`), held from here to the group's atoms.
        let mut shared = self.shared.lock();
        let Shared { strings, atoms, produced } = &mut *shared;
        let strings_from = strings.len();
        // Kept by the batch's groups and the batches it grows into (`ProducerRun::relations`): what a producer knows of
        // a file (its annotations) stays for its guests a later group or round adds.
        let produced = produced.get_or_insert_with(|| producers().iter().map(|producer| (crate::code_rule_dsl::language::tape_tag(producer.dialect()), producer.start(reads, markers))).collect());
        // Every string of every node passes here: Fx, not SipHash (a third of the projection's time with SipHash).
        // A batch that keeps its groups' strings (decision 0616) notes each one the group uses.
        let keeps = self.keeps_strings;
        let mut used: Vec<u32> = Vec::new();
        let mut string = |s: &str| -> usize {
            let i = strings.intern(s);
            if keeps {
                used.push(i as u32);
            }
            i
        };
        // Decision 0055: each document's role, root and tape; each guest's place, and its nodes' host nodes. A guest's
        // key is past every file's (`corpus::guests`), so its host is projected first and its lines are known.
        let mut g = GuestRows::default();
        // The documents W reads no values of (`Dialect::values`).
        let mut unvalued: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for (j, (&i, d)) in group.iter().zip(&documents).enumerate() {
            let made::MadeDoc { p, withheld, error } = self.docs.made(i);
            let (withheld, tag) = (*withheld, self.docs.tag(i));
            // The document's own dialect (`Facts::tag`): a text-only document has none, and no node.
            let dialect = if tag == TEXT { None } else { self.dialects.get(tag) };
            let guest = d.guest.as_deref().filter(|_| core);
            if core && !projection.ast_patterns.is_empty() && guest.is_none() && !withheld
                && let Some(dialect) = dialect
                && let Some(lang) = crate::code_rule_dsl::language::from_tape_tag(tag)
            {
                let compiled = ast_patterns.entry(tag).or_insert_with(|| {
                    projection
                        .ast_patterns
                        .iter()
                        .filter_map(|t| crate::code_rule_dsl::language::rule_pattern(crate::code_rule_dsl::language::pattern_source(t), lang).ok().map(|p| (t.as_str(), p)))
                        .collect()
                });
                let tape = d.tape.as_ref().map(|(t, _)| in_batch(t, self.docs.cache()));
                c.patterns(p, tape.as_ref(), dialect, lang, compiled, &mut string);
            }
            if dialect.is_some_and(|d| !d.values) {
                unvalued.insert(d.key);
            }
            let key = i64::from(d.key);
            // core.rerail: a module for Python and Rust files, "" for the others; `pkg` for Python only. T's has both
            // for every file.
            // A tape value (decision 0431) is no file's: no `doc` row, no module or package.
            let tape_value = d.guest.as_deref().is_some_and(|g| matches!(g.made, Made::Tape { .. }));
            let py_file = !tape_value && (d.path.ends_with(".py") || d.path.ends_with(".pyi"));
            let named = d.module.as_deref();
            let module = doc_module(&d.path, named, core);
            if !tape_value {
                doc.push([key, string(&d.path) as i64, string(&module) as i64]);
            }
            if !core || py_file {
                let package = match named {
                    Some(m) => package_of(&d.path, m),
                    None => package(&d.path),
                };
                pkg.push([key, string(&package) as i64]);
            }
            if py_file && d.unpackaged {
                unpackaged.push(key);
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
            if d.facts.has_syntax_error() {
                errors.push(key);
            }
// copy (0353 step 3): core.rerail's per-node rows. Tape answers become tape views, and derived values become
// rules computed on demand, written once (decision 0353).
            let nodes_span = crate::code_rule_dsl::logic::profile::span(|| "node rows".into());
            const NODE_ROWS: rerail_prof::copies::Site = rerail_prof::copies::Site::new(7, "3", "core.rerail's and the vtab's per-node rows (node rows)");
            let (copying, rows_before) = (NODE_ROWS.start(), rows.bytes() + c.bytes());
            if let Some(dialect) = dialect {
                // In core.rerail's shape `NodeRows` makes only `intval` and T's `op` (`op_t`).
                if scalar_nodes && (!core || projection.wants("intval") || projection.wants("op_t")) {
                    visited += p.len() as usize;
                    rows.document(p, dialect, core, &mut string);
                }
                if core_nodes || wants_long_text_nodes {
                    let kept = (c.skip.literal_rows, c.skip.literal_value);
                    if value_lit_only && dialect.values {
                        (c.skip.literal_rows, c.skip.literal_value) = (true, true);
                    }
                    c.for_dialect(dialect);
                    let visits = c.visits(wants_parts, wants_part_starts, wants_words);
                    for (local, id, n) in p.node_ids() {
                        if wants_long_text_nodes && (p.f().end(id) - p.f().start(id)) as usize > core_rr::TEXT_INLINE {
                            long_text_nodes.push(n);
                        }
                        if core_nodes && c.visited(visits, p, dialect, id) {
                            visited += 1;
                            c.node(p, dialect, local, id, n, &mut string, wants_parts, wants_part_starts, wants_words);
                        }
                    }
                    (c.skip.literal_rows, c.skip.literal_value) = kept;
                }
                if let Some((line, msg)) = error {
                    let line = p.extra.as_ref().map_or(*line, |x| x.file_line(*line));
                    c.withheld(p, &(line, msg.clone()), &mut string);
                }
            }
            copying.done((rows.bytes() + c.bytes()).saturating_sub(rows_before));
            drop(nodes_span);
            let document_span = crate::code_rule_dsl::logic::profile::span(|| "document rows".into());
            // A text-only document has core.rerail's text rows too (its lines), and no tokens.
            if core && (!withheld || guest.is_none()) {
                let s = crate::code_rule_dsl::logic::profile::span(|| "core document rows".into());
                c.document(p, guest.is_none(), projection);
                drop(s);
                if let Some(dialect) = dialect
                    && wants_digests
                    && !withheld
                {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "digests".into());
                    c.digests(p, dialect, &mut string, wants_trees, wants_units);
                }
            }
            if d.guest.is_none() {
                // A text document's lines split as W's `lines` regexes split any file's. A withheld document has no
                // `rx_span` matches.
                let rx = if withheld { &[][..] } else { &rx_spans[..] };
                {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "extra document rows".into());
                    extra.document(p, d, dialect, rx, &span_ends[j], &mut rows.text, &mut string)?;
                }
                let _s = crate::code_rule_dsl::logic::profile::span(|| "tree document rows".into());
                tree.document(p, dialect, python == Some(tag), &mut string);
            } else {
                // A guest's nodes have their places in its file (`Locs::place`); W's values, f-string trees and CPython's
                // verdicts are a file's, read off its tape; an embedded program keeps its own (decision 0222).
                if d.tape.is_some() {
                    extra.values(p, d, dialect, &mut string)?;
                }
                extra.nodes(p, &mut string);
                if let Some(dialect) = dialect {
                    tree.operators(p, dialect, &mut string);
                    tree.visits(p, dialect, python == Some(tag));
                }
            }
            // A guest's are read only by a producer of guests' rows (string annotations); the others' are its file's.
            for (own, run) in produced.iter_mut() {
                if d.guest.is_none() || run.reads_guests() {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| format!("produced rows: {}", run.name()));
                    run.document(p, d, dialect, *own == Some(tag), &mut string)?;
                }
            }
            drop(document_span);
        }
        // The guests' rows, once every document is projected: a guest's host may be a guest after it. Either shape:
        // a program that declares its base relations itself (T's) reads guests as core.rerail's do. A tape value's hosts
        // are the chunk's (`outer`).
        let hosts = guests::HostsOf { docs: &self.docs, outer: self.outer.as_ref().map(|b| &b.docs) };
        let wants = [projection.wants("host_node"), projection.wants("unparse_of"), projection.wants("guest_of"), projection.wants("guest_place")];
        for (&i, d) in group.iter().zip(&documents) {
            let m = self.docs.made(i);
            g.document(&m.p, d, d.guest.as_deref(), m.withheld, &hosts, wants, &mut string);
        }
        for &(p, d, ref new, withheld) in held {
            let Some(guest) = d.guest.as_deref() else { continue };
            if let Made::Tape { places } = &guest.made {
                g.places_added(p, d, &guest.grammar, &places[new.clone()], withheld, &hosts, [wants[0], wants[2], wants[3]], &mut string);
            }
        }
        let docs: Vec<&Projected> = group.iter().map(|&i| &self.docs[i]).collect();
        crate::code_rule_dsl::logic::profile::decision("input payload", || {
            let identities: usize = docs.iter().map(|p| p.ranks().bytes()).sum();
            serde_json::json!({"documents": docs.len(), "coreColumnBytes": docs.iter().map(|p| p.facts.bytes()).sum::<usize>(), "identityCapacityBytes": identities}).to_string()
        });
        crate::code_rule_dsl::logic::profile::decision("projection demand",||serde_json::json!({"coreNodes":if core_nodes {docs.iter().map(|p| p.len() as usize).sum::<usize>()}else{0},"scalarNodes":visited,"nodes":docs.iter().map(|p| p.len() as usize).sum::<usize>(),"strings":strings.len() - strings_from,"reads":projection.reads}).to_string());
        let intern = crate::code_rule_dsl::logic::profile::span(|| "intern atoms".into());
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, (strings.len() - strings_from) as u64);
        let new_atoms = self.atoms.intern_all(&(strings_from..strings.len()).map(|i| strings.get(i)).collect::<Vec<_>>())?;
        atoms.extend(new_atoms);
        drop(intern);
        let atoms = &*atoms;
        let atom = |i: i64| i64::from(atoms[i as usize]);
        let _relations_span = crate::code_rule_dsl::logic::profile::span(|| "relations".into());
        // A value alone's rows are staged: they are written once, past the chunk's relations' rows (`Grower::admit`).
        let relation: Make = if first { Relation::from_rows } else { Relation::staged };
        let pairs = |columns: &[&str], rows: &[[i64; 2]], sym: [bool; 2]| {
            relation(columns, &mut rows.iter().map(|&[a, b]| rerail_blocks::vector![if sym[0] { atom(a) } else { a }, if sym[1] { atom(b) } else { b }]))
        };
        let mut relations = vec![
            (LONG_TEXT_NODES, relation(&["n"], &mut long_text_nodes.iter().map(|&n| rerail_blocks::vector![n]))),
            ("doc", relation(&["d", "path", "module"], &mut doc.iter().map(|&[d, p, m]| rerail_blocks::vector![d, atom(p), atom(m)]))),
            ("pkg", pairs(&["d", "p"], &pkg, [false, true])),
            ("unpackaged", relation(&["d"], &mut unpackaged.iter().map(|&d| rerail_blocks::vector![d]))),
            // The tape's own verdict: tree-sitter's ERROR and MISSING nodes (`Facts::syntax_errors`).
            ("tape_error", relation(&["d"], &mut errors.iter().map(|&d| rerail_blocks::vector![d]))),
            ("path_part", pairs(&["d", "s"], &path_part, [false, true])),
            ("file_name", pairs(&["d", "s"], &file_name, [false, true])),
            ("intval", pairs(&["n", "v"], &rows.intval, [false, false])),
            ("op_t", pairs(&["n", "s"], &rows.op, [false, true])),
        ];
        relations.extend(g.relations(&atom, relation));
        if core {
            extra.literal_values(c.values_of(&unvalued));
        }
        // In core.rerail's shape `Extra`'s spans' text is their own relation (none for a node or position: the `text`
        // view's), bound after the view's rows (`places::join`).
        let span_text = core && extra.text && extra.spans();
        let (mut read, matched) = extra.relations(&atom, relation);
        if span_text {
            read.push(("text", pairs(&["n", "s"], &rows.text, [false, true])));
        }
        read.extend(tree.relations(&atom, relation));
        for (_, run) in produced.iter_mut() {
            read.extend(run.relations(&atom));
        }
        // The relations made for what the program reads are the batch's: in `core.rerail`'s shape they stand in for
        // `core_rr`'s rows and the views of the same name (W's values, the spans and places `match_all` reads,
        // `op_at`'s operators, `visit_index`'s CPython order).
        let made: BTreeSet<&'static str> = read.iter().map(|(name, _)| *name).collect();
        if core {
            relations.extend(c.relations(&atom).into_iter().filter(|(name, _)| !made.contains(name)));
        } else {
            // T's `syntax_error(d)`, or core.rerail's `syntax_error(d, l, msg)` (CPython's verdict) where the program
            // declares that one (`Projection::verdict`).
            let syntax_error = if projection.verdict {
                c.relations(&atom).into_iter().find(|(name, _)| *name == "syntax_error").map(|(_, r)| r).expect("core_rr makes syntax_error")
            } else {
                relation(&["d"], &mut errors.iter().map(|&d| rerail_blocks::vector![d]))
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
        // The strings released before the matchers run (on the pool, `make_rows`' first lines).
        drop(shared);
        // The `ast` matchers' relations over the group's tapes (decision 0106).
        let waved = match &self.wave {
            Some((matchers, binding)) if !matchers.is_empty() => {
                let held: Vector<usize> = group.iter().copied().filter(|&i| self.docs.input(i).guest.is_none() && self.docs.input(i).tape.is_some()).collect();
                let tapes: Vec<(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)> =
                    held.iter().map(|&i| self.docs.input(i)).filter_map(|d| Some((Arc::clone(&d.facts), in_batch(&d.tape.as_ref()?.0, self.docs.cache()), d.path.clone()))).collect();
                let at: Vec<Option<&Projected>> = held.iter().map(|&i| Some(&self.docs[i])).collect();
                match &matchers.0 {
                    Some(wave) => wave.matched(&tapes, &at, *binding, &self.atoms)?,
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        // What each of the batch's relations holds (0353 step 3's row 7 is these rows): its bytes of values, by name.
        if crate::code_rule_dsl::logic::profile::on() {
            for (name, r) in &relations {
                crate::code_rule_dsl::prof::count_named(format!("batch relation {name} (bytes of values)"), 8 * Source::len(r) * Source::columns(r).len());
            }
        }
        let used = keeps.then(|| {
            used.sort_unstable();
            used.dedup();
            used
        });
        Ok(made::Made { relations, matched, waved, read: made, strings_from, used })
    }

    /// A batch of `documents` that keeps its groups' strings, renewed from `held` where it can be (decision 0616), as
    /// a session's corpus batch is (`with_matchers_renewed`), with no matchers.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn renewed(store: &ContentStore, documents: Vec<impl Into<Input>>, projection: &Projection, held: Option<Arc<Batch>>) -> StoreResult<Arc<Self>> {
        let mut growth = Growth { renew: held, keeps_strings: true, ..Growth::default() };
        Self::add(&mut growth, store, inputs(documents), projection, &[], &[], false, None, &HashMap::new(), false).map(|(batch, _)| batch)
    }
    /// Its vocabulary's strings (`vocabulary`), sorted: a text test's scan reads them.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn vocabulary_strings(&self) -> Vec<String> {
        let mut v: Vec<String> = self.vocabulary().0.iter().map(str::to_owned).collect();
        v.sort();
        v
    }

    /// How many groups of documents the batch has made rows of (decision 0494).
    pub fn groups(&self) -> usize {
        self.rows.groups()
    }

    /// How many groups it has made, those set aside because another read made one of their documents first too: its
    /// groups, unless a document was made twice.
    pub fn makes(&self) -> usize {
        self.rows.makes()
    }

    /// The node view's row offsets by document: every document's projection made.
    pub fn nodes(&self) -> &[usize] {
        self.offsets(View::Node)
    }

    /// The batch's chunk block (decision 0289), made on first use.
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    pub fn chunk(&self) -> &device_chunk::Chunk {
        self.chunk.get_or_init(|| device_chunk::build(self))
    }
    /// The view's rows' offsets by document, from each document's count of them (decision 0494): made on the first
    /// read, every document's projection made for it.
    fn offsets(&self, view: View) -> &[usize] {
        // Each document's counts: those given (the documents' table's), else its made projection's.
        let counted = |count: &dyn Fn(made::ViewCounts) -> usize| {
            let mut offsets = Vector::with_capacity(self.docs.len() + 1);
            offsets.push(0);
            for i in 0..self.docs.len() {
                offsets.push(offsets.last().unwrap() + count(self.docs.counts(i)));
            }
            offsets
        };
        let o = &self.offsets;
        let [nodes, fielded, siblings, locs] = [0, 1, 2, 3];
        match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order | View::Unit => o.nodes.get_or_init(|| counted(&|c| c[nodes] as usize)),
            View::Child => o.children.get_or_init(|| counted(&|c| (c[nodes] as usize).saturating_sub(1))),
            View::Field => o.fields.get_or_init(|| counted(&|c| if self.projection.wants("field") { c[fielded] as usize } else { 0 })),
            View::Sibling => o.siblings.get_or_init(|| counted(&|c| if self.projection.wants("next_sibling") { c[siblings] as usize } else { 0 })),
            View::Loc(_) => o.locs.get_or_init(|| counted(&|c| c[locs] as usize)),
            View::Op => o.tokens.get_or_init(|| counted(&|c| c[9] as usize)),
            View::Text => o.texts.get_or_init(|| counted(&|c| c[10] as usize)),
            View::Tok(rel) => o.toks[rel as usize].get_or_init(|| {
                let at = 6 + match rel.rows() {
                    Item::Leads => 0,
                    Item::Follows => 1,
                    _ => 2,
                };
                counted(&|c| if self.projection.wants(tokens::VIEWS[rel as usize]) { c[at] as usize } else { 0 })
            }),
            View::Spread => o.spreads.get_or_init(|| counted(&|c| if self.projection.wants("spread") { c[5] as usize } else { 0 })),
            View::End => o.ends.get_or_init(|| counted(&|c| if self.projection.wants("end_point") { c[nodes] as usize } else { 0 })),
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
    /// The documents of file `file`'s programs (decision 0431 §4).
    fn programs_of(&self, file: u32) -> impl Iterator<Item = usize> + '_ {
        let at = self.programs.partition_point(|&(f, _)| f < file);
        self.programs[at..].iter().take_while(move |&&(f, _)| f == file).map(|&(_, d)| d)
    }
    /// The document of key `d`, by index (the last one found tried first, as `locate`).
    fn doc(&self, d: i64) -> Option<usize> {
        let d = u32::try_from(d).ok()?;
        let key = self.docs.addr();
        let hit = DOC.with(|h| {
            let (k, i) = h.get();
            (k == key && i < self.docs.len() && self.docs.key(i) == d).then_some(i)
        });
        if hit.is_some() {
            return hit;
        }
        let i = self.docs.position(d)?;
        DOC.with(|h| h.set((key, i)));
        Some(i)
    }
    /// `doc`, from `hint` (the document a probe before found): checked, then the next one, then searched.
    fn doc_near(&self, d: i64, hint: &mut u32) -> Option<usize> {
        let d = u32::try_from(d).ok()?;
        let h = *hint as usize;
        for i in [h, h + 1] {
            if i < self.docs.len() && self.docs.key(i) == d {
                *hint = i as u32;
                return Some(i);
            }
        }
        let i = self.docs.position(d)?;
        *hint = i as u32;
        Some(i)
    }
    /// `node`, its document found from `hint` (`doc_near`).
    fn node_near(&self, n: i64, hint: &mut u32) -> Option<(usize, u32)> {
        let doc = self.doc_near(n >> 32, hint)?;
        let local = self.docs[doc].local_of((n & 0xffff_ffff) as u32)?;
        (n >= 0 && local < self.docs[doc].len()).then_some((doc, local))
    }
    /// The document index and node of id `n`.
    pub fn node(&self, n: i64) -> Option<(usize, u32)> {
        let doc = self.doc(n >> 32)?;
        let local = self.docs[doc].local_of((n & 0xffff_ffff) as u32)?;
        (n >= 0 && local < self.docs[doc].len()).then_some((doc, local))
    }
    /// The nodes whose `kind` (the `node` view's column 2) is `kind`, in row order: `(document, local id, tape node)`.
    fn by_kind(&self, kind: i64) -> &[[u32; 3]] {
        let (Ok(tag), true) = (u8::try_from(kind / i64::from(crate::code_rule_dsl::facts::core::KIND_STRIDE)), kind >= 0) else { return &[] };
        let Ok(at) = self.kinds.binary_search_by_key(&tag, |e| e.0) else { return &[] };
        let k = self.kinds[at].1.get_or_init(|| KindRows::new(self, tag));
        k.keys.binary_search(&kind).map_or(&[][..], |i| &k.at[k.starts[i] as usize..k.starts[i + 1] as usize])
    }
    /// The document index and node or span of id `n`.
    fn loc(&self, n: i64) -> Option<(usize, u32)> {
        let doc = self.doc(n >> 32)?;
        let local = self.docs[doc].local_of((n & 0xffff_ffff) as u32)?;
        (n >= 0 && (local < self.docs[doc].len() || local < self.docs[doc].locs())).then_some((doc, local))
    }
    /// The view's row of node `local` of document `doc`, if it has one.
    fn row(&self, view: View, doc: usize, local: u32) -> Option<usize> {
        match view {
            View::Field | View::Sibling | View::Spread => self.row_of_node(view, doc, local, self.docs[doc].node(local)),
            // A token's row is found by its tape node (`row_of_node`), not its node's.
            View::Op => None,
            _ => self.row_of_node(view, doc, local, NONE),
        }
    }
    /// The atom of the text of a token of kind code `code` in a document of tape tag `tag` (`op`'s `s`): its kind's
    /// name, as `core_rr` reads a token's text, interned once for every kind of the tag.
    pub(super) fn op_atom(&self, tag: u8, code: u16) -> i64 {
        let Some(d) = self.dialects.get(tag) else { return -1 };
        let table = self.op_atoms[usize::from(tag)].get_or_init(|| {
            let names: Vec<&str> = (0..d.names.len()).map(|c| d.code_name(c as u16)).collect();
            self.atoms.intern_all(&names).unwrap_or_default()
        });
        match table.get(usize::from(code)) {
            Some(&a) => i64::from(a),
            None => self.atoms.intern_all(&[d.code_name(code)]).ok().and_then(|v| v.first().copied()).map_or(-1, i64::from),
        }
    }
    /// Each node and position with the text whose atom is `atom` (`text` read by its `s`, decision 0656 step 4): its
    /// document, local id and a node's tape node. Only the documents whose index says a node or span can have the text
    /// are made and walked (`DocIndex::may_have_text`; a text-only document's source searched), each one's rows
    /// compared by their bytes, no text interned.
    fn text_matches(&self, atom: i64, mut each: impl FnMut(usize, u32, u32)) {
        let Some(text) = u32::try_from(atom).ok().and_then(|a| self.atoms.value(a).ok().flatten()) else { return };
        let t = text.as_bytes();
        for doc in 0..self.docs.len() {
            let facts = &self.docs.input(doc).facts;
            let source = facts.source.as_bytes();
            let may = if facts.tape.is_none() { memchr::memmem::find(source, t).is_some() } else { self.docs.read(doc, |f| f.index().may_have_text(t, source)) };
            if !may {
                continue;
            }
            let p = &self.docs[doc];
            for i in 0..p.text_rows() {
                let (local, x) = p.text_local(i);
                if p.with_text(local, x, |s| s.as_bytes() == t) {
                    each(doc, local, x);
                }
            }
        }
    }
    /// `row`, the node's tape node `x` known (read only by the `field` view: its rows are the fielded nodes).
    fn row_of_node(&self, view: View, doc: usize, local: u32, x: u32) -> Option<usize> {
        let within = match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order | View::Unit | View::End | View::Loc(_) => Some(local as usize),
            View::Child => (local > 0).then(|| local as usize - 1),
            View::Field => self.docs[doc].ranks().local(Item::Field, x).map(|r| r as usize),
            View::Sibling => self.docs[doc].ranks().local(Item::Sibling, x).map(|r| r as usize),
            View::Spread => self.docs[doc].ranks().local(Item::Spread, x).map(|r| r as usize),
            View::Tok(rel) => self.docs[doc].token_row_of(rel, local).map(|r| r as usize),
            View::Op => self.docs[doc].ranks().local(Item::Op, x).map(|r| r as usize),
            View::Text => self.docs[doc].text_row_of(local, x).map(|r| r as usize),
        }?;
        Some(self.offsets(view)[doc] + within)
    }
    /// The rows of nodes with ids in `lo..=hi`.
    fn between(&self, view: View, lo: i64, hi: i64) -> rerail_blocks::vector::Vector<usize> {
        let mut rows = rerail_blocks::vector::Vector::new();
        let first = self.docs.keys().partition_point(|&k| i64::from(k) < lo >> 32);
        for doc in first..self.docs.len() {
            let base = i64::from(self.docs.key(doc)) << 32;
            if base > hi {
                break;
            }
            let p = &self.docs[doc];
            // Ids by definition are not in local order: the locals whose ids fall in the range.
            if p.defs.is_some() {
                let ids: Box<dyn Iterator<Item = (u32, i64)>> = if matches!(view, View::Loc(_) | View::Text) { Box::new(p.loc_ids()) } else { Box::new(p.node_ids().map(|(l, _, n)| (l, n))) };
                let mut locals: Vector<u32> = ids.filter(|&(_, n)| (lo..=hi).contains(&n)).map(|(l, _)| l).collect();
                locals.sort_unstable();
                rows.extend(locals.into_iter().filter_map(|local| self.row(view, doc, local)));
                continue;
            }
            let start = (lo.max(base) - base) as u32;
            let count = if matches!(view, View::Loc(_) | View::Text) { p.locs() } else { p.len() };
            let end = ((hi - base).min(i64::from(count) - 1) + 1).max(0) as u32;
            rows.extend((start..end.max(start)).filter_map(|local| self.row(view, doc, local)));
        }
        rows
    }
}

struct Viewed {
    batch: Arc<Batch>,
    view: View,
    /// The view's relation of no columns, its indexes' block, for a view the device reads in place (decision 0478 step
    /// 3): made the first time it is read there. `None` inside when no block could be had.
    home: std::sync::OnceLock<Option<crate::code_rule_dsl::facts::rows::Part>>,
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
            View::Sibling => &["a", "b"],
            View::Unit => &["n", "u"],
            View::Op | View::Text => &["n", "s"],
            View::End => &["n", "p"],
            View::Spread => &["n"],
            View::Tok(rel) => rel.columns(),
            View::Loc(Place::Line | Place::EndLine) => &["n", "l"],
            View::Loc(Place::Col) => &["n", "c"],
            View::Loc(Place::Docof) => &["n", "d"],
            View::Loc(Place::Report) => &["n", "line", "column", "end_line", "end_column"],
        }
    }
    /// The column holding the row's own node, which ranges and direct lookups address.
    fn own(&self) -> usize {
        match self.view {
            View::Node | View::Child | View::Sibling => 1,
            View::Line | View::Depth | View::Order | View::Unit | View::End | View::Spread | View::Tok(_) | View::Loc(_) | View::Text => 0,
            View::Field | View::Core => 2,
            View::Tape => 0,
            // No column is a token's own: a lookup by `n` is by its node (`lookup_docs`).
            View::Op => 2,
        }
    }
    /// The column holding the row's parent, looked up through the children lists.
    fn parent(&self) -> Option<usize> {
        match self.view {
            View::Child | View::Field => Some(0),
            View::Tape => Some(1),
            View::Node | View::Line | View::Core | View::Depth | View::Order | View::Sibling | View::Unit | View::Op | View::End | View::Spread | View::Tok(_) | View::Loc(_) | View::Text => None,
        }
    }
}
/// `node`'s column `column` over `batch`, counted without reading its rows (decision 0329): its distinct values, the
/// sum of their counts' squares, and each value's count. A kind's count is its codes' nodes, a document's its nodes;
/// `n` is the key (each value once, no counts). `None` for another column (`node_column_counted`).
///
/// Each document's nodes and codes are its index's counts where they answer for its projection (decision 0656 step
/// 4, `Docs::indexed_codes`): no document is made to count them. Else its projection's, made (`doc_codes`).
pub fn node_column_counts(batch: &Batch, column: usize) -> Option<(i64, i64, Option<Vec<(i64, i64)>>)> {
    let squares = |counts: &[(i64, i64)]| counts.iter().map(|&(_, n)| n * n).sum::<i64>();
    let docs = 0..batch.docs.len();
    match column {
        0 => {
            let counts: Vec<(i64, i64)> = docs.map(|i| (i64::from(batch.docs.key(i)), i64::from(doc_codes(batch, i, false).0))).filter(|&(_, n)| n > 0).collect();
            Some((counts.len() as i64, squares(&counts), Some(counts)))
        }
        1 => {
            let n: i64 = docs.map(|i| i64::from(doc_codes(batch, i, false).0)).sum();
            Some((n, n, None))
        }
        2 => {
            // Each code read as the column reads it (`Dialect::kind`: its canonical code).
            let mut by: HashMap<i64, i64> = HashMap::new();
            for i in docs {
                let tag = batch.docs.tag(i);
                let Some(dialect) = batch.dialects.get(tag) else { continue };
                for (code, n) in doc_codes(batch, i, true).1 {
                    *by.entry(i64::from(kind_atom(tag, dialect.canonical.get(usize::from(code)).copied().unwrap_or(code)))).or_default() += i64::from(n);
                }
            }
            let mut counts: Vec<(i64, i64)> = by.into_iter().collect();
            counts.sort_unstable();
            Some((counts.len() as i64, squares(&counts), Some(counts)))
        }
        _ => None,
    }
}

/// Document `i`'s nodes, and (`codes`) each of their codes' count as the `node` view reads the code (`Dialect::code`):
/// a made one's off its projection; else off its index where that answers for it (`Docs::indexed_codes`), else off
/// its projection, made.
fn doc_codes(batch: &Batch, i: usize, codes: bool) -> (u32, Vec<(u16, u32)>) {
    if !batch.docs.is_made(i)
        && let Some(x) = batch.docs.indexed_codes(i)
    {
        return x;
    }
    let p = batch.docs.get(i).expect("a document of the batch");
    let mut by: std::collections::BTreeMap<u16, u32> = Default::default();
    // A text-only document has no dialect and no nodes.
    if let Some(dialect) = batch.dialects.get(p.tag).filter(|_| codes) {
        for (_, id) in p.nodes() {
            *by.entry(dialect.code(p.f(), id)).or_default() += 1;
        }
    }
    (p.len(), by.into_iter().collect())
}

/// Whether `node_column_counts` counts `node`'s column `column`: known before it counts, which makes the documents
/// whose index does not answer for them.
pub fn node_column_counted(column: usize) -> bool {
    column <= 2
}

impl Source for Viewed {
    fn syntax_batch(&self)->Option<Arc<dyn std::any::Any + Send + Sync>> {Some(Arc::clone(&self.batch) as _)}
    /// `docof`'s document column: a document's rows are its own, at its offsets, each counted under its file as its
    /// input names it (`Docs::file`, as a made one's): no document is made, where walking the made documents made
    /// every one.
    fn column_counts(&self, c: usize) -> Option<Vec<(i64, u64)>> {
        if !matches!((self.view, c), (View::Loc(Place::Docof), 1)) {
            return None;
        }
        let offsets = self.batch.offsets(self.view);
        let mut by: std::collections::BTreeMap<i64, u64> = std::collections::BTreeMap::new();
        for i in 0..self.batch.docs.len() {
            let n = (offsets[i + 1] - offsets[i]) as u64;
            if n > 0 {
                *by.entry(i64::from(self.batch.docs.file(i))).or_default() += n;
            }
        }
        Some(by.into_iter().collect())
    }
    /// A view's rows are in document order (decision 0494): a document's lie at its offset, counted.
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<rerail_blocks::vector::Vector<usize>> {
        let offsets = self.batch.offsets(self.view);
        Some(docs.iter().filter_map(|&k| self.batch.docs.position(k)).flat_map(|i| offsets[i]..offsets[i + 1]).collect())
    }
    #[cfg(any(target_os = "macos", target_os = "linux", windows))]
    fn device_view(&self) -> Option<crate::code_rule_dsl::facts::vtab::DeviceView<'_>> {
        let view = device_chunk::device_view(self.view)?;
        let chunk = self.batch.chunk();
        // The views the views' reads answer are read in place where every tape (and for `line`, every document's
        // lines) is on the device.
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        let loc = matches!(self.view, View::Loc(_));
        // The directory is of the process's device's addresses, which the stages read it by (decision 0440 step C):
        // none where its buffers have none (`views_in_place`).
        let on_device = rerail_accel::gpu::join::device().is_ok();
        let read_only = matches!(view, device_chunk::VIEW_TAPE | device_chunk::VIEW_DEPTH | device_chunk::VIEW_ORDER);
        let in_place = (on_device && (loc || read_only || matches!(view, device_chunk::VIEW_NODE | device_chunk::VIEW_CHILD | device_chunk::VIEW_FIELD | device_chunk::VIEW_CORE | device_chunk::VIEW_LINE)))
            .then(|| device_chunk::views_in_place(&self.batch, chunk))
            .flatten()
            .filter(|v| (view != device_chunk::VIEW_LINE || v.lines) && (!loc || v.locs))
            .and_then(|v| {
                let mut in_place = crate::code_rule_dsl::facts::vtab::ViewInPlace { dir: v.dir?, kind: device_chunk::read_kind(view), buffers: v.buffers.clone(), cache: v.cache.clone(), failed: 0, format: v.format, keep: Arc::clone(&self.batch) as Arc<dyn std::any::Any + Send + Sync> };
                in_place.failed = in_place.cache_failed();
                Some(in_place)
            });
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        let in_place: Option<()> = None;
        // `tape`, `ast_depth` and `node_order` are read in place or not at all: no device code makes their rows.
        if read_only && in_place.is_none() {
            return None;
        }
        if matches!(self.view, View::Loc(_)) && in_place.is_none() && !device_chunk::fill_locs(&self.batch, chunk) {
            return None;
        }
        if matches!(self.view, View::Line) && in_place.is_none() && !device_chunk::fill_lines(&self.batch, chunk) {
            return None;
        }
        #[cfg(any(target_os = "macos", target_os = "linux", windows))]
        let home = in_place.as_ref().and_then(|_| self.home.get_or_init(|| {
            // Room for an index of each key the strata probe a view by, found again by every later binding: with room
            // for one, each binding probing it by another key made a fresh home of its own, never found again (133 of
            // 3.4 MB in each of pth_unpack's fixpoints, its 49 lines by kind pair; ab138).
            crate::code_rule_dsl::facts::rows::Part::for_device_indexed(0, self.len(), crate::code_rule_dsl::facts::rows::AXIS_KEYS).ok()
        }).clone());
        #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
        let home = None;
        Some(crate::code_rule_dsl::facts::vtab::DeviceView {
            block: &chunk.block,
            root: chunk.root,
            view,
            rows: self.len(),
            #[cfg(any(target_os = "macos", target_os = "linux", windows))]
            in_place,
            home,
        })
    }
    fn columns(&self) -> Vec<String> {
        Self::columns(self.view).iter().map(|c| (*c).to_owned()).collect()
    }
    fn len(&self) -> usize {
        *self.batch.offsets(self.view).last().unwrap()
    }
    /// A node's document or column found from its id (`docof`, `col`: 4 and 3.5 derived lookups), a parent's
    /// children listed (`child` 2, `field` 1.7): decision 0312. The rest about one.
    fn lookup_cost(&self, column: usize) -> f64 {
        match (self.view, column) {
            (View::Loc(Place::Docof), 0) => 4.0,
            (View::Loc(Place::Col), 0) => 3.5,
            (View::Child, 0) => 2.0,
            (View::Field, 0) => 1.7,
            _ => 1.0,
        }
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
            || matches!(self.view, View::Sibling) && column == 0
            || matches!(self.view, View::Unit) && column == 1
            || matches!(self.view, View::Op) && column == 0
            || matches!(self.view, View::Text) && column == 1
            || matches!(self.view, View::End) && column == 1
            || matches!(self.view, View::Loc(Place::Docof)) && column == 1
    }
    fn range_indexed(&self, column: usize) -> bool {
        column == self.own()
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
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
        Some(if lo > hi { rerail_blocks::vector::Vector::new() } else { self.batch.between(self.view, lo, hi) })
    }
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        self.read_matches(column, value, columns, &mut super::ProbeCursor::default(), each)
    }
    fn lookup_read_at(&self, column: usize, value: i64, columns: &[usize], cursor: &mut super::ProbeCursor, each: &mut dyn FnMut(usize, &[i64])) -> bool {
        self.read_matches(column, value, columns, cursor, each)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.lookup_docs(column, values, None)
    }
    fn keys_rows(&self, column: usize) -> bool {
        column == self.own()
    }
    fn keys_documents(&self, column: usize) -> bool {
        matches!((self.view, column), (View::Node | View::Core, 0) | (View::Loc(Place::Docof), 1))
    }
    /// A place's `docof` is its file's (a program's host's); every other view's columns are its documents' own.
    fn rows_of_own_documents(&self) -> bool {
        !matches!(self.view, View::Loc(_))
    }
    /// `lookup_docs`' own column, for one value.
    fn key_row(&self, column: usize, value: i64) -> Option<usize> {
        debug_assert_eq!(column, self.own());
        let b = &*self.batch;
        let found = if matches!(self.view, View::Loc(_) | View::Text) { b.loc(value) } else { b.node(value) };
        found.and_then(|(doc, local)| b.row(self.view, doc, local))
    }
    /// A field's children by parent and field (`field($a, #f, $t)`, `$a` bound): each child's field read off the
    /// tape as the children are walked, not its row's values found again. By child and field (`field(_, #f, $n)`,
    /// `$n` bound: whether `$n` is its parent's `#f`), the child's own field read off the tape: its one row or
    /// none. Found by child and filtered, `nameref`'s `!field(_, #f65541, $n)` took 300 ns a probe, 25 M probes of a
    /// cold PostHog run.
    fn lookup_eq(&self, column: usize, value: i64, eqs: &[(usize, i64)]) -> Option<rerail_blocks::vector::Vector<usize>> {
        let &[(1, f)] = eqs else { return None };
        if !matches!(self.view, View::Field) {
            return None;
        }
        let b = &*self.batch;
        if column == 2 {
            let Some((doc, local)) = b.node(value) else { return Some(rerail_blocks::vector::Vector::new()) };
            let p = &b.docs[doc];
            let x = p.node(local);
            let own = i64::from(field_atom(p.tag, p.f().field(x))) == f;
            return Some(if own { b.row_of_node(View::Field, doc, local, x).into_iter().collect() } else { rerail_blocks::vector::Vector::new() });
        }
        if column != 0 {
            return None;
        }
        let mut rows = rerail_blocks::vector::Vector::new();
        if let Some((doc, local)) = b.node(value) {
            let p = &b.docs[doc];
            p.each_child_node(local, |c, x| {
                if i64::from(field_atom(p.tag, p.f().field(x))) == f {
                    rows.extend(b.row_of_node(View::Field, doc, c, x));
                }
            });
        }
        Some(rows)
    }
    fn doc_column(&self) -> Option<usize> {
        matches!(self.view, View::Node | View::Core).then_some(0)
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        matches!((self.view, column), (View::Node, 2) | (View::Core, 1))
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<rerail_blocks::vector::Vector<usize>> {
        if !self.kept_to_doc(column) {
            return None;
        }
        // A document not in the batch has no rows.
        Some(match self.batch.doc(doc) {
            Some(doc) => self.lookup_docs(column, values, Some(doc))?,
            None => rerail_blocks::vector::Vector::new(),
        })
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.row_value(row, column)
    }
    /// Document by document, with no search for each row's document. (A merge had left this in `impl Viewed`,
    /// where the trait's per-row scan, a document search for each row, shadowed it.)
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vector<u32> {
        let offsets = self.batch.offsets(self.view);
        let mut out = rerail_blocks::vector::Vector::new();
        for (doc, p) in self.batch.docs.iter().enumerate() {
            let base = offsets[doc];
            let cols: Vec<usize> = eqs.iter().map(|&(c, _)| c).collect();
            self.each_row(p, offsets[doc + 1] - base, &cols, |i, value| {
                if eqs.iter().zip(value).all(|(&(_, v), &got)| got == v) {
                    out.push((base + i) as u32);
                }
            });
        }
        out
    }
    fn rows_of(&self, columns: &[usize], out: &mut Vector<i64>) {
        let offsets = self.batch.offsets(self.view);
        out.reserve(self.len() * columns.len());
        for (doc, p) in self.batch.docs.iter().enumerate() {
            self.each_row(p, offsets[doc + 1] - offsets[doc], columns, |_, value| out.extend_from_slice(value));
        }
    }
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        let offsets = self.batch.offsets(self.view);
        for (doc, p) in self.batch.docs.iter().enumerate() {
            let base = offsets[doc];
            self.each_row(p, offsets[doc + 1] - base, columns, |i, value| each(base + i, value));
        }
    }
    fn for_rows_in(&self, range: std::ops::Range<usize>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) {
        let offsets = self.batch.offsets(self.view);
        let mut go = true;
        // The documents the range holds: from the one holding its first row.
        let first = offsets.partition_point(|&o| o <= range.start).saturating_sub(1);
        for (doc, p) in self.batch.docs.iter().enumerate().skip(first) {
            let base = offsets[doc];
            if !go || base >= range.end {
                break;
            }
            self.each_row(p, offsets[doc + 1] - base, columns, |i, value| {
                if !go || !range.contains(&(base + i)) {
                    return;
                }
                go = each(base + i, value);
            });
        }
    }
    fn for_rows_keyed(&self, column: usize, keys: &rustc_hash::FxHashSet<i64>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) -> bool {
        // A row's own column (a node's or a location's id) names its document in its high half: only those documents
        // are walked.
        if column != self.own() {
            return false;
        }
        let docs: rustc_hash::FxHashSet<i64> = keys.iter().map(|k| k >> 32).collect();
        let offsets = self.batch.offsets(self.view);
        // The columns read, then the key's.
        let read: Vec<usize> = columns.iter().copied().chain([column]).collect();
        let mut go = true;
        for (doc, p) in self.batch.docs.iter().enumerate() {
            if !go {
                break;
            }
            if !docs.contains(&i64::from(p.key)) {
                continue;
            }
            let base = offsets[doc];
            self.each_row(p, offsets[doc + 1] - base, &read, |i, value| {
                if !go || !keys.contains(&value[columns.len()]) {
                    return;
                }
                go = each(base + i, &value[..columns.len()]);
            });
        }
        true
    }
    /// A column that is a row's id, or its document (`node` and `core`'s first): every row of a document has its key.
    fn documents_of(&self, column: usize, each: &mut dyn FnMut(usize, i64)) {
        let own = match self.view {
            View::Loc(_) => column == 0,
            View::Node | View::Core if column == 0 => true,
            _ => self.reads_id(column),
        };
        // A document keyed 0 has ids below `1 << 32`, each its own "document" (`memo::document_of`): read them.
        if !own || self.batch.docs.iter().any(|p| p.key == 0) {
            return self.for_rows(&[column], &mut |r, row| each(r, if row[0] >= 1 << 32 { row[0] >> 32 } else { row[0] }));
        }
        let offsets = self.batch.offsets(self.view);
        let node = matches!(self.view, View::Node) && column == 0;
        for (doc, p) in self.batch.docs.iter().enumerate() {
            let d = if node { p.file } else { p.key };
            for r in offsets[doc]..offsets[doc + 1] {
                each(r, i64::from(d));
            }
        }
    }
    /// The row's node selected once (`Projected::node`), however many columns read it.
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let (p, i) = self.batch.locate(self.view, row);
        if matches!(self.view, View::Loc(_)) {
            for (o, &c) in out.iter_mut().zip(columns) {
                *o = self.value_at(p, i, c);
            }
            return;
        }
        // The slots' ops and the inputs they need, decided once for the row (`read_op`), the inputs computed once
        // and the slots filled from them (`op_value`): no cell or closure a column.
        use read_op::*;
        let (local, x) = self.local_of_row(p, i);
        let mut ops = [0u8; 16];
        let mut needs = 0u8;
        for (k, &c) in columns.iter().enumerate().take(16) {
            let (op, need) = Self::read_op(self.view, c);
            ops[k] = op;
            needs |= need;
        }
        if columns.len() > 16 {
            for (o, &c) in out.iter_mut().zip(columns) {
                *o = self.value_at(p, i, c);
            }
            return;
        }
        let x = if x == NONE && needs & (NEED_X | NEED_UP) != 0 { p.node(local) } else { x };
        let n = if needs & NEED_ID != 0 { p.global(local) } else { 0 };
        let up = if needs & NEED_UP != 0 { Self::parent_id_of(p, local, x) } else { 0 };
        let b = &*self.batch;
        for (o, &op) in out.iter_mut().zip(&ops[..columns.len()]) {
            *o = Self::op_value(b, p, local, x, n, up, Known::default(), op);
        }
    }
}
impl Viewed {
    /// The rows whose `column` is one of `values`, in document `only` (by index) where the column spans the
    /// documents (a kind, a tape node), else in every document.
    fn lookup_docs(&self, column: usize, values: &[i64], only: Option<usize>) -> Option<rerail_blocks::vector::Vector<usize>> {
        let b = &*self.batch;
        // A lookup kept to a file keeps to its programs too (decision 0431 §4).
        let kept = |doc: &usize| only.is_none_or(|o| o == *doc || b.file(*doc) == b.docs.key(o));
        let mut rows = rerail_blocks::vector::Vector::new();
        for &v in values {
            if column == self.own() {
                let found = if matches!(self.view, View::Loc(_) | View::Tok(_) | View::Text) { b.loc(v) } else { b.node(v) };
                rows.extend(found.and_then(|(doc, local)| b.row(self.view, doc, local)));
            } else if Some(column) == self.parent() {
                if v == -1 && matches!(self.view, View::Tape) {
                    rows.extend((0..b.docs.len()).filter(|&doc| b.docs[doc].len() > 0).map(|doc| b.nodes()[doc]));
                } else if let Some((doc, local)) = b.node(v) {
                    b.docs[doc].each_child(local, |c| rows.extend(b.row(self.view, doc, c)));
                }
            } else if matches!(self.view, View::Order) && column == 1 {
                // An order key: its document, and the node with that key (`Projected::local_of_order`).
                let key = (v & 0xffff_ffff) as u32;
                rows.extend(b.doc(v >> 32).filter(|_| v >= 0).and_then(|doc| b.docs[doc].local_of_order(key).and_then(|local| b.row(self.view, doc, local))));
            } else if matches!(self.view, View::Text) && column == 1 {
                // The nodes and positions with text `v`, in the documents whose index can hold it.
                let offsets = b.offsets(View::Text);
                b.text_matches(v, |doc, local, x| rows.extend(b.docs[doc].text_row_of(local, x).map(|r| offsets[doc] + r as usize)));
            } else if matches!(self.view, View::End) && column == 1 {
                // An end point's node: its place among the end points.
                if let Some((doc, l)) = b.loc(v) {
                    let p = &b.docs[doc];
                    let from = p.extra.as_ref().map_or(u32::MAX, |x| p.len() + x.ends_from());
                    if l >= from && l - from < p.len() {
                        rows.extend(b.row(self.view, doc, l - from));
                    }
                }
            } else if matches!(self.view, View::Op) && column == 0 {
                // A node's anonymous children.
                if let Some((doc, local)) = b.node(v) {
                    let p = &b.docs[doc];
                    p.f().each_child(p.node(local), |x| rows.extend(b.row_of_node(self.view, doc, local, x)));
                }
            } else if matches!(self.view, View::Unit) && column == 1 {
                // A unit's nodes, walked on the tape (`Projected::each_of_unit`); the root's are those no unit holds.
                // Unit -1 is the roots' (the root has none): each document's root.
                if v == -1 {
                    rows.extend((0..b.docs.len()).filter(|&doc| b.docs[doc].len() > 0).filter_map(|doc| b.row(self.view, doc, 0)));
                } else if let Some((doc, local)) = b.node(v) {
                    let p = &b.docs[doc];
                    p.each_of_unit(&b.dialects[p.tag], local, |l| rows.extend(b.row(self.view, doc, l)));
                }
            } else if matches!(self.view, View::Sibling) && column == 0 {
                // The row of `a`'s next sibling: a hop along the tape's siblings.
                rows.extend(b.node(v).and_then(|(doc, local)| b.docs[doc].next_sibling(local).and_then(|s| b.row(self.view, doc, s))));
            } else if matches!(self.view, View::Core) && column == 0 {
                rows.extend(b.doc(v).map_or(0..0, |doc| b.nodes()[doc]..b.nodes()[doc + 1]));
            } else if matches!(self.view, View::Node) && column == 0 {
                // A file's rows, then its programs' (decision 0431 §4).
                let Ok(file) = u32::try_from(v) else { continue };
                for doc in b.doc(v).into_iter().chain(b.programs_of(file)) {
                    rows.extend(b.nodes()[doc]..b.nodes()[doc + 1]);
                }
            } else if matches!(self.view, View::Loc(Place::Docof)) && column == 1 {
                // A file's places, then its programs' (decision 0431 §4): at their documents' offsets, none made, where
                // an index of the view's every row made every document.
                let Ok(file) = u32::try_from(v) else { continue };
                let offsets = b.offsets(self.view);
                for doc in b.doc(v).into_iter().chain(b.programs_of(file)) {
                    rows.extend(offsets[doc]..offsets[doc + 1]);
                }
            } else if matches!(self.view, View::Core) && column == 1 {
                // A tape node of each document that has it as a node; the statement filters `d`.
                let Ok(t) = u32::try_from(v) else { continue };
                for (doc, p) in b.docs.iter().enumerate().filter(|(doc, _)| kept(doc)) {
                    if let Some(local) = p.ranks().local(Item::Node, t) {
                        rows.push(b.nodes()[doc] + local as usize);
                    }
                }
            } else if matches!(self.view, View::Node) && column == 2 {
                rows.extend(b.by_kind(v).iter().filter(|&&[doc, _, _]| kept(&(doc as usize))).map(|&[doc, local, _]| b.nodes()[doc as usize] + local as usize));
            } else {
                return None;
            }
        }
        Some(rows)
    }
    /// What column `column` of this view reads of a match, as a plan's op, and the inputs it needs (`read_matches`):
    /// `value_of`'s columns, decided once a step.
    fn read_op(view: View, column: usize) -> (u8, u8) {
        use read_op::*;
        match (view, column) {
            (View::Node, 0) => (FILE, 0),
            (View::Core, 0) => (KEY, 0),
            (View::Core, 1) => (TAPE, NEED_X),
            (View::Node, 2) => (KIND, NEED_X),
            (View::Node, 3) => (START, NEED_X),
            (View::Node, 4) => (END, NEED_X),
            (View::Line, 1) => (LINE, NEED_X),
            (View::Child | View::Field, 0) | (View::Tape, 1) => (UP, NEED_X | NEED_UP),
            (View::Child, 2) => (IDX, 0),
            (View::Field, 1) => (FIELD, NEED_X),
            (View::Tape, 2) => (SIZE, 0),
            (View::Tape, 3) | (View::Depth, 1) => (DEPTH, 0),
            (View::Order, 1) => (ORDER, 0),
            (View::Sibling, 0) => (PREV, 0),
            (View::Unit, 1) => (UNIT, 0),
            (View::Op, 1) => (OP, NEED_X),
            (View::Text, 1) => (TEXT, 0),
            (View::End, 1) => (ENDP, 0),
            (View::Tok(rel), 1) => (TOKF + rel as u8, 0),
            _ => (ID, NEED_ID),
        }
    }

    /// Slot op `op` (`read_op`) of the match at `local` of `p`: its tape node `x`, own id `n` and parent's id `up` as
    /// the plan's needs had them computed, and its place, depth and the sibling before where the walk or the probe
    /// knows them (`known`).
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn op_value(b: &Batch, p: &Projected, local: u32, x: u32, n: i64, up: i64, known: Known, op: u8) -> i64 {
        use read_op::*;
        match op {
            FILE => i64::from(p.file),
            KEY => i64::from(p.key),
            TAPE => i64::from(x),
            KIND => b.dialects[p.tag].kind(p.f(), x),
            START => i64::from(p.f().start(x)),
            END => i64::from(p.f().end(x)),
            LINE => i64::from(p.f().start_line(x)),
            UP => up,
            IDX => i64::from(known.idx.unwrap_or_else(|| p.idx(local as usize))),
            FIELD => i64::from(field_atom(p.tag, p.f().field(x))),
            SIZE => i64::from(p.size(local as usize)),
            DEPTH => i64::from(known.depth.unwrap_or_else(|| p.depth(local as usize))),
            ORDER => i64::from(p.key) << 32 | i64::from(p.order(local)),
            PREV => known.prev.unwrap_or_else(|| Self::prev_id(p, local)),
            UNIT => Self::unit_id(b, p, local),
            OP => b.op_atom(p.tag, b.dialects[p.tag].code(p.f(), x)),
            TEXT => p.text_atom(&b.atoms, local, x),
            ENDP => Self::end_id(p, local),
            TOKF..=TOKA => p.token_of([tokens::TokRel::First, tokens::TokRel::Last, tokens::TokRel::Before, tokens::TokRel::After][usize::from(op - TOKF)], local),
            _ => n,
        }
    }
    /// `Source::lookup_read`: `lookup_docs`'s matches for one value, each row's `columns` read from what found it.
    /// A lookup by a node's id knows the row's id; by its parent, the parent's id and each child's tape node; by a
    /// kind or a tape node, the tape node: no select, granule search or parent search a row for those. Views whose
    /// rows are not one a node (`field`, `loc`), and the other lookups, read none (`false`).
    ///
    /// The step's cursor (`Source::lookup_read_at`) holds where the probe before ended (`[document, granule]`, so a
    /// probe in key order finds its document and its matches' granule without a search) and the plan of the reads
    /// (`read_op`, made on the step's first probe): a match computes only the inputs the plan needs, the ones the
    /// probe did not bring, then fills each slot from its op.
    fn read_matches(&self, column: usize, value: i64, columns: &[usize], cursor: &mut super::ProbeCursor, each: &mut dyn FnMut(usize, &[i64])) -> bool {
        use read_op::*;
        // A place's row by its node or position (`line`, `col`, `docof`, by `n`): its values read off its span, into
        // an array, no vector a probe. By another column (`docof` by file), refused.
        if let View::Loc(place) = self.view {
            let b = &*self.batch;
            if column != 0 || columns.len() > 8 {
                return false;
            }
            if let Some((doc, local)) = b.loc(value)
                && let Some(row) = b.row(self.view, doc, local)
            {
                let p = &b.docs[doc];
                let mut vals = [0i64; 8];
                for (o, &c) in vals.iter_mut().zip(columns) {
                    *o = match (c, place) {
                        (0, _) => value,
                        (_, Place::Docof) => i64::from(p.file),
                        _ => Self::loc_value(p, place, 0, p.span(local), c),
                    };
                }
                each(row, &vals[..columns.len()]);
            }
            return true;
        }
        // A text's row by its node or position (`n`), or the rows of a text (`s`, `Batch::text_matches`): each match's
        // id and atom where the probe knows them, the rest read.
        if matches!(self.view, View::Text) {
            let b = &*self.batch;
            if column > 1 || columns.len() > 8 {
                return false;
            }
            let offsets = b.offsets(View::Text);
            let mut vals = [0i64; 8];
            let mut emit = |doc: usize, local: u32, x: u32| {
                let p = &b.docs[doc];
                let Some(r) = p.text_row_of(local, x) else { return };
                for (o, &c) in vals.iter_mut().zip(columns) {
                    *o = match (c, column) {
                        (0, 0) | (1, 1) => value,
                        (0, _) => p.global(local),
                        _ => p.text_atom(&b.atoms, local, x),
                    };
                }
                each(offsets[doc] + r as usize, &vals[..columns.len()]);
            };
            if column == 0 {
                if let Some((doc, local)) = b.loc(value) {
                    emit(doc, local, NONE);
                }
            } else {
                b.text_matches(value, emit);
            }
            return true;
        }
        // A field's rows are its fielded nodes: a probe's match without a field has no row (`row_of_node`), and is
        // passed over. The token order's values are no slot op of the plan.
        if matches!(self.view, View::Tok(_)) {
            return false;
        }
        if !cursor.plan.is_for(column, columns) {
            let Some(mut plan) = super::ReadPlan::new(column, columns) else { return false };
            for (k, &c) in columns.iter().enumerate() {
                let (op, needs) = Self::read_op(self.view, c);
                plan.ops[k] = op;
                plan.needs |= needs;
            }
            cursor.plan = plan;
        }
        let plan = cursor.plan;
        let (len, view, b) = (usize::from(plan.len), self.view, &*self.batch);
        let cursor_also = cursor.also;
        let [doc_hint, granule] = &mut cursor.hint;
        let mut vals = [0i64; 8];
        // A match: its document, local id, tape node (`NONE`: unknown), own id and parent's id where known, and its place
        // and depth where the probe knows them (a probe by parent: the hop's counter, one below the parent).
        let mut emit = |doc: usize, local: u32, x: u32, id: Option<i64>, up: Option<i64>, known: Known| {
            // The row by the tape node in hand (the hop's): no select for a view whose rows are an item's.
            let row = if x != NONE || matches!(view, View::Op) { b.row_of_node(view, doc, local, x) } else { b.row(view, doc, local) };
            let Some(row) = row else { return };
            let p = &b.docs[doc];
            let x = if x == NONE && plan.needs & NEED_X != 0 { p.node(local) } else { x };
            let n = match id {
                Some(n) => n,
                None if plan.needs & NEED_ID != 0 => p.global_near(local, granule),
                None => 0,
            };
            let up = match up {
                Some(up) => up,
                None if plan.needs & NEED_UP != 0 => Self::parent_id_of(p, local, x),
                None => 0,
            };
            for (o, &op) in vals[..len].iter_mut().zip(&plan.ops) {
                *o = Self::op_value(b, p, local, x, n, up, known, op);
            }
            each(row, &vals[..len]);
        };
        if column == self.own() {
            if let Some((doc, local)) = b.node_near(value, doc_hint) {
                emit(doc, local, NONE, Some(value), None, Known::default());
            }
        } else if Some(column) == self.parent() {
            if value == -1 {
                return false;
            }
            if let Some((doc, local)) = b.node_near(value, doc_hint) {
                // The children's place is the hop's counter and their depth the parent's and one, read once a probe
                // where the plan reads it. Narrowed to anchors, a node's place counts the nodes under left-out ones too
                // (`Projected::anchored`): there the place is read as before.
                let p = &b.docs[doc];
                let depth = if plan.ops[..len].contains(&DEPTH) { p.depth(local as usize) + 1 } else { 0 };
                let counted = !p.anchored;
                // A child probed by its place too (`child(p, c, i)`): that child alone, where the hop counts places.
                let at = match (view, cursor_also) {
                    (View::Child, Some((2, i))) if counted => u32::try_from(i).ok(),
                    _ => None,
                };
                // A field probed by its parent and field (`field(p, f, c)`): the children under that field alone.
                let field = match (view, cursor_also) {
                    (View::Field, Some((1, f))) => Some(f),
                    _ => None,
                };
                // By field, where no op reads a child's place: each child's field read off the tape first, and only
                // a match's local id found (a rank a child: most of a probe's walk).
                if let Some(f) = field
                    && at.is_none()
                    && !plan.ops[..len].contains(&IDX)
                {
                    let known = if counted { Known { idx: None, depth: Some(depth), prev: None } } else { Known::default() };
                    p.f().each_child_field(p.node(local), |x, code| {
                        if i64::from(field_atom(p.tag, code)) == f {
                            let c = p.local_of_node(x);
                            if c != NONE {
                                emit(doc, c, x, None, Some(value), known);
                            }
                        }
                    });
                    return true;
                }
                let mut i = 0;
                p.each_child_node(local, |c, x| {
                    if at.is_some_and(|a| a != i) || field.is_some_and(|f| i64::from(field_atom(p.tag, p.f().field(x))) != f) {
                        i += 1;
                        return;
                    }
                    emit(doc, c, x, None, Some(value), if counted { Known { idx: Some(i), depth: Some(depth), prev: None } } else { Known::default() });
                    i += 1;
                });
            }
        } else if matches!(self.view, View::Core) && column == 1 {
            let Ok(t) = u32::try_from(value) else { return true };
            for (doc, p) in b.docs.iter().enumerate() {
                if let Some(local) = p.ranks().local(Item::Node, t) {
                    emit(doc, local, t, None, None, Known::default());
                }
            }
        } else if matches!(self.view, View::Op) && column == 0 {
            // A node's anonymous children, each with its tape node; the node's id is the key.
            if let Some((doc, local)) = b.node_near(value, doc_hint) {
                let p = &b.docs[doc];
                let r = p.ranks();
                p.f().each_child(p.node(local), |x| {
                    if r.is(Item::Op, x) {
                        emit(doc, local, x, Some(value), None, Known::default());
                    }
                });
            }
        } else if matches!(self.view, View::Unit) && column == 1 {
            // A unit's nodes, walked on the tape: each match with its local id. Unit -1: each document's root.
            if value == -1 {
                for doc in (0..b.docs.len()).filter(|&doc| b.docs[doc].len() > 0) {
                    emit(doc, 0, NONE, None, None, Known::default());
                }
            } else if let Some((doc, local)) = b.node_near(value, doc_hint) {
                let p = &b.docs[doc];
                p.each_of_unit(&b.dialects[p.tag], local, |l| emit(doc, l, NONE, None, None, Known::default()));
            }
        } else if matches!(self.view, View::Node) && column == 2 {
            // The batch's index by kind (`Batch::by_kind`): each match with its local id and tape node.
            for &[doc, local, x] in b.by_kind(value) {
                emit(doc as usize, local, x, None, None, Known::default());
            }
        } else {
            return false;
        }
        true
    }
    fn row_value(&self, row: usize, column: usize) -> i64 {
        let (p, i) = self.batch.locate(self.view, row);
        self.value_at(p, i, column)
    }
}

impl Viewed {
    /// Each of document `p`'s `n` rows in this view, in order: its index and its `columns`' values. The slots' ops and
    /// the inputs they need are decided once (`read_op`); a row computes those inputs alone and fills the slots from
    /// them (`op_value`), with no reader a column. A view whose rows are nodes visits them in order
    /// (`Projected::nodes`), so a row's tape node comes with it; where the ops read the parent, a place, a depth or
    /// the sibling before, the walk carries them too, not a read a row.
    fn each_row(&self, p: &Projected, n: usize, columns: &[usize], mut row: impl FnMut(usize, &[i64])) {
        use read_op::*;
        let b = &*self.batch;
        let mut vals = vec![0i64; columns.len()];
        if let View::Loc(place) = self.view {
            for ((i, (_, span)), (_, g)) in p.spans().enumerate().zip(p.loc_ids()).take(n) {
                for (o, &c) in vals.iter_mut().zip(columns) {
                    *o = Self::loc_value(p, place, g, span, c);
                }
                row(i, &vals);
            }
            return;
        }
        let mut ops = Vec::with_capacity(columns.len());
        let mut needs = 0u8;
        for &c in columns {
            let (op, need) = Self::read_op(self.view, c);
            ops.push(op);
            needs |= need;
        }
        let mut fill = |i: usize, l: u32, x: u32, n: i64, up: i64, known: Known| {
            for (o, &op) in vals.iter_mut().zip(&ops) {
                *o = Self::op_value(b, p, l, x, n, up, known, op);
            }
            row(i, &vals);
        };
        let (only, skip) = match self.view {
            View::Child => (None, 1),
            // The nodes below the root with a field, in order (`fielded`); the nodes after a sibling; the spread nodes.
            View::Field => (Some(Item::Field), 1),
            View::Sibling => (Some(Item::Sibling), 1),
            View::Spread => (Some(Item::Spread), 0),
            View::Tok(rel) => {
                for i in 0..(p.token_rows(rel) as usize).min(n) {
                    let (l, x) = p.token_row(rel, i as u32);
                    let id = if needs & NEED_ID != 0 { p.global(l) } else { 0 };
                    fill(i, l, x, id, -1, Known::default());
                }
                return;
            }
            // The nodes with text in preorder, then the positions with text.
            View::Text => {
                for i in 0..(p.text_rows() as usize).min(n) {
                    let (l, x) = p.text_local(i as u32);
                    let id = if needs & NEED_ID != 0 { p.global(l) } else { 0 };
                    fill(i, l, x, id, -1, Known::default());
                }
                return;
            }
            // The anonymous children in preorder, each with its node.
            View::Op => {
                for (i, (_, x)) in p.ranks().iter(Item::Op).enumerate().take(n) {
                    let l = p.f().parent(x).map_or(0, |q| p.local_of_node(q));
                    let id = if needs & NEED_ID != 0 { p.global(l) } else { 0 };
                    let up = if needs & NEED_UP != 0 { Self::parent_id(p, l) } else { 0 };
                    fill(i, l, x, id, up, Known::default());
                }
                return;
            }
            _ => (None, 0),
        };
        let mut i = 0;
        let mut each = |l: u32, x: u32, n: i64, up: i64, known: Known| {
            if l < skip || only.is_some_and(|item| !p.ranks().is(item, x)) {
                return;
            }
            fill(i, l, x, n, up, known);
            i += 1;
        };
        // A parent's id, a child's place, a depth and the sibling before are carried in the walk (decision 0353 step 3)
        // where the ops read them; the sibling only where the walk's parents are the tape's (`Projected::anchored`).
        let walked = needs & NEED_UP != 0;
        let carried = ops.iter().any(|&o| matches!(o, IDX | DEPTH)) || ops.contains(&PREV) && !p.anchored;
        let ids = walked || needs & NEED_ID != 0;
        if walked || carried {
            // The open subtrees: each one's end on the tape, its id, its children so far and its last child's id.
            let mut open: Vec<(u32, i64, u32, i64)> = Vec::new();
            let mut step = |l: u32, x: u32, n: i64| {
                while open.last().is_some_and(|&(end, _, _, _)| end <= x) {
                    open.pop();
                }
                let depth = open.len() as u32;
                let (up, idx, prev) = match open.last_mut() {
                    Some(o) => {
                        o.2 += 1;
                        let prev = o.3;
                        o.3 = n;
                        (o.1, o.2 - 1, prev)
                    }
                    None => (-1, 0, -1),
                };
                open.push((p.f().subtree_end(x), n, 0, -1));
                // A walk without ids carries none: then no op reads the parent's or the sibling's.
                let known = Known { idx: Some(idx), depth: Some(depth), prev: ids.then_some(prev) };
                each(l, x, n, up, known);
            };
            if ids {
                for (l, x, n) in p.node_ids() {
                    step(l, x, n);
                }
            } else {
                for (l, x) in p.nodes() {
                    step(l, x, 0);
                }
            }
        } else if ids {
            for (l, x, n) in p.node_ids() {
                each(l, x, n, 0, Known::default());
            }
        } else {
            for (l, x) in p.nodes() {
                each(l, x, 0, 0, Known::default());
            }
        }
    }

    /// Whether column `column` of this view is the row's id (`value_of`'s last arm).
    fn reads_id(&self, column: usize) -> bool {
        !matches!(
            (self.view, column),
            (View::Node | View::Core, 0)
                | (View::Core, 1)
                | (View::Node, 2..=4)
                | (View::Line, 1)
                | (View::Child | View::Field, 0)
                | (View::Tape, 1..=3)
                | (View::Child, 2)
                | (View::Field, 1)
                | (View::Depth, 1)
                | (View::Order, 1)
                | (View::Unit, 1)
                | (View::Op, 1)
                | (View::Text, 1)
                | (View::End, 1)
        )
    }

    /// The id of node `local`'s end point (`Locs::ends`), `-1` where the document has none.
    fn end_id(p: &Projected, local: u32) -> i64 {
        p.extra.as_ref().filter(|x| local < x.ends).map_or(-1, |x| p.global(p.len() + x.ends_from() + local))
    }

    /// The id of node `local`'s definition unit (`Projected::unit_of`), `-1` for the root.
    fn unit_id(b: &Batch, p: &Projected, local: u32) -> i64 {
        p.unit_of(&b.dialects[p.tag], local).map_or(-1, |u| p.global(u))
    }

    /// The id of node `local`'s numbered sibling before it (`Projected::prev_sibling`), `-1` for none.
    fn prev_id(p: &Projected, local: u32) -> i64 {
        p.prev_sibling(local).map_or(-1, |a| p.global(a))
    }

    /// The id of node `local`'s parent (`Projected::parent`), `-1` above the root.
    fn parent_id(p: &Projected, local: u32) -> i64 {
        match p.parent(local as usize) {
            NONE => -1,
            q => p.global(q),
        }
    }
    /// `parent_id`, the node's tape node in hand (not selected again).
    fn parent_id_of(p: &Projected, local: u32, x: u32) -> i64 {
        if local == 0 {
            return -1;
        }
        match p.parent_of_node(x) {
            NONE => -1,
            q => p.global(q),
        }
    }

    /// Column `column` of a `Loc` view's row: a node's or span's id `g`, and its place from its byte span.
    fn loc_value(p: &Projected, place: Place, g: i64, span: [u32; 2], column: usize) -> i64 {
        if column == 0 {
            return g;
        }
        if matches!(place, Place::Docof) {
            return i64::from(p.file);
        }
        // `rr_report_location(n, line, column, end_line, end_column)`: each end's line and its column from 1.
        if matches!(place, Place::Report) {
            let at = span[(column - 1) / 2];
            let (line, _, col) = p.extra.as_ref().expect("core.rerail's lines").place([at, at]);
            return i64::from(if column % 2 == 1 { line } else { col + 1 });
        }
        let (line, last, col) = p.extra.as_ref().expect("core.rerail's lines").place(span);
        match place {
            Place::Line => i64::from(line),
            Place::EndLine => i64::from(last),
            Place::Col => i64::from(col),
            Place::Docof => i64::from(p.file),
            Place::Report => unreachable!(),
        }
    }

    /// Column `column` of row `i` of document `p` in this view.
    fn value_at(&self, p: &Projected, i: usize, column: usize) -> i64 {
        if let View::Loc(place) = self.view {
            let local = i as u32;
            return match (column, place) {
                (0, _) => p.global(local),
                (_, Place::Docof) => i64::from(p.file),
                _ => Self::loc_value(p, place, 0, p.span(local), column),
            };
        }
        use read_op::*;
        let (local, x) = self.local_of_row(p, i);
        let (op, needs) = Self::read_op(self.view, column);
        let x = if x == NONE && needs & (NEED_X | NEED_UP) != 0 { p.node(local) } else { x };
        let n = if needs & NEED_ID != 0 { p.global(local) } else { 0 };
        let up = if needs & NEED_UP != 0 { Self::parent_id_of(p, local, x) } else { 0 };
        Self::op_value(&self.batch, p, local, x, n, up, Known::default(), op)
    }

    /// The local id of document `p`'s row `i` in this view, and its tape node where finding the row read it (the
    /// `field` view's: a select among the fielded nodes), else `NONE`.
    fn local_of_row(&self, p: &Projected, i: usize) -> (u32, u32) {
        match self.view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order | View::Unit | View::End | View::Loc(_) => (i as u32, NONE),
            View::Child => (i as u32 + 1, NONE),
            View::Field => {
                let x = p.ranks().select(Item::Field, i as u32).expect("a field row");
                (p.ranks().rank(Item::Node, x), x)
            }
            View::Sibling => {
                let x = p.ranks().select(Item::Sibling, i as u32).expect("a sibling row");
                (p.ranks().rank(Item::Node, x), x)
            }
            View::Spread => {
                let x = p.ranks().select(Item::Spread, i as u32).expect("a spread row");
                (p.ranks().rank(Item::Node, x), x)
            }
            View::Op => {
                let x = p.ranks().select(Item::Op, i as u32).expect("an op row");
                (p.f().parent(x).map_or(0, |q| p.local_of_node(q)), x)
            }
            View::Tok(rel) => p.token_row(rel, i as u32),
            View::Text => p.text_local(i as u32),
        }
    }
}

/// The ops of a view's read plan (`Viewed::read_op`): what a slot reads of a match, and the inputs a plan needs.
mod read_op {
    pub const FILE: u8 = 0;
    pub const KEY: u8 = 1;
    pub const TAPE: u8 = 2;
    pub const KIND: u8 = 3;
    pub const START: u8 = 4;
    pub const END: u8 = 5;
    pub const LINE: u8 = 6;
    pub const UP: u8 = 7;
    pub const IDX: u8 = 8;
    pub const FIELD: u8 = 9;
    pub const SIZE: u8 = 10;
    pub const DEPTH: u8 = 11;
    pub const ORDER: u8 = 12;
    pub const PREV: u8 = 13;
    pub const ID: u8 = 14;
    pub const UNIT: u8 = 15;
    pub const OP: u8 = 16;
    pub const ENDP: u8 = 17;
    /// `tokens::TokRel`'s answers, in its order.
    pub const TOKF: u8 = 18;
    pub const TOKA: u8 = 21;
    /// `text`'s `s` (`Projected::text_atom`).
    pub const TEXT: u8 = 22;
    /// The match's tape node, own id and parent's id.
    pub const NEED_X: u8 = 1;
    pub const NEED_ID: u8 = 2;
    pub const NEED_UP: u8 = 4;
}

/// What a walk or a probe already has of a match (`Viewed::op_value`): its place among its parent's children, its depth
/// and the numbered sibling before it, by id (`next_sibling`'s `a`); read off the tape where not.
#[derive(Clone, Copy, Default)]
struct Known {
    idx: Option<u32>,
    depth: Option<u32>,
    prev: Option<i64>,
}

/// The field atom of field `code` in the dialect whose tape tag is `tag`.
pub fn field_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) << 16 | u32::from(code)
}

/// Document `key`'s `function_definition` and `class_definition` nodes by byte range (start, end), as ids of
/// `node` in the run's shape (`core`: core.rerail's, else T's; a node's id is its rank among the shape's numbered nodes,
/// so T's ids name other nodes of a run in core.rerail's shape): what W's call summaries name a definition by
/// (`calls::summary::Def`).
pub fn definitions(key: u32, facts: &Arc<Facts>, core: bool) -> StoreResult<HashMap<(u32, u32), i64>> {
    let dialect = Dialect::new(crate::code_rule_dsl::facts::core::facts_tag(facts))?;
    let p = Projected::new(key, Arc::clone(facts), Some(&dialect), if core { Shape::Core } else { Shape::T }, false, None);
    Ok(p.node_ids()
        .filter(|&(_, id, _)| matches!(dialect.name(p.f(), id), "function_definition" | "class_definition"))
        .map(|(_, id, n)| ((p.f().start(id), p.f().end(id)), n))
        .collect())
}

/// T's nine base relations over `documents`, and the `py.*` families, by table name.
pub fn sources(store: &ContentStore, documents: Vec<Document>) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    shaped(store, documents, &Projection::default())
}

/// The base relations over `documents` in `projection`'s shape, and the `py.*` families, by table name.
pub fn shaped(store: &ContentStore, documents: Vec<Document>, projection: &Projection) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    let batch = Batch::new(store, documents, projection, &[], &[], false, None, &HashMap::new())?;
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = made_sources(&batch).into_iter().filter_map(|(_, name, source)| Some((name?, source))).collect();
    out.extend(views(&batch));
    places::join(&mut out, &batch);
    Ok(out)
}

/// The views a batch of `shape` binds by name (`views`).
fn named_views(shape: Shape) -> Vec<(&'static str, View)> {
    let mut named = vec![("node", View::Node), ("child", View::Child), ("field", View::Field), ("tape", View::Tape), ("core_node", View::Core), ("node_order", View::Order)];
    // A node's depth is the tape's (`at.depth`) in either shape: the cross part reads it as a measure's key (decision
    // 0364 R6) whatever the program's shape.
    named.push(("ast_depth", View::Depth));
    named.push(("def_unit", View::Unit));
    match shape {
        Shape::T => named.push(("line", View::Line)),
        Shape::Core => named.extend([
            ("rr_report_location", View::Loc(Place::Report)),
            ("line", View::Loc(Place::Line)),
            ("end_line", View::Loc(Place::EndLine)),
            ("col", View::Loc(Place::Col)),
            // `core.rerail`'s `docof(x: loc, d)` is an input (`rr::lower`): nodes' and spans' documents.
            ("docof", View::Loc(Place::Docof)),
            ("next_sibling", View::Sibling),
            ("end_point", View::End),
            ("spread", View::Spread),
            ("op", View::Op),
            ("text", View::Text),
            ("first_token", View::Tok(tokens::TokRel::First)),
            ("last_token", View::Tok(tokens::TokRel::Last)),
            ("token_before", View::Tok(tokens::TokRel::Before)),
            ("token_after", View::Tok(tokens::TokRel::After)),
        ]),
    }
    named
}

/// Whether a batch of `shape` binds a view named `name`.
pub fn has_view(shape: Shape, name: &str) -> bool {
    named_views(shape).iter().any(|(n, _)| *n == name)
}

/// The batch's views: `node`, `child`, `field`, `tape` and the `py.*` families.
pub fn views(batch: &Arc<Batch>) -> Vec<(&'static str, Arc<dyn Source>)> {
    let mut out: Vec<(&'static str, Arc<dyn Source>)> = Vec::new();
    let named = named_views(batch.shape);
    // A relation the batch made as rows (`Batch::made`: its spans have places too) is bound in place of the view.
    for (name, view) in named.into_iter().filter(|(name, _)| !batch.made.contains(name)) {
        out.push((name, Arc::new(Viewed { batch: Arc::clone(batch), view, home: std::sync::OnceLock::new() })));
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

/// T's base relations over `documents` (`sources`) in `projection`'s shape and those of core.rerail's the program reads
/// that T had not (`Extra`), each of `tests`' relations, and each of `sets` (a relation and its set's kind atoms):
/// `rr_nearest_<set>(n, a)`, the nearest ancestor in the set, `rr_inside_<set>(n, a)`, every one (decision 0036), or
/// `rr_has_<set>(n, a)`, n in the set and a each of its strict ancestors. Each is made per looked-up node on the host
/// (`within::Axis`), or for the whole batch by the transducer's file-local FIX (`typing::file_local::nearest`, bound
/// `binding`) where the planner expects more lookups than that costs (decision 0143).
pub fn with_nearest(
    store: &ContentStore,
    documents: Vec<Document>,
    sets: &[(String, Vector<i64>)],
    tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    binding: crate::code_rule_dsl::facts::nearest::Binding,
    projection: &Projection,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    with_matchers(store, documents, sets, &Matchers::default(), tests, binding, projection, &HashMap::new())
}

/// A program's `ast` matchers (decision 0106) as one wave over a batch's tapes (decision 0107), which the engine
/// compiles from the matchers' text (`AstCompiler`): facts runs the wave where `with_matchers` binds, and keeps no
/// matcher of its own.
pub trait AstWave: Send + Sync {
    /// How many matchers the wave runs.
    fn len(&self) -> usize;
    /// Each matcher's relation over `tapes` (each a batch document's facts, tape and path, its projection `at`): its
    /// matched nodes, and the captures it binds, staged (a batch's group goes on in its block relation, decision 0494).
    fn matched(
        &self,
        tapes: &[(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)],
        at: &[Option<&Projected>],
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        atoms: &crate::content_store::atoms::Atoms,
    ) -> StoreResult<Vec<(String, Relation)>>;
    /// Each matcher's relation and its columns past `n` that hold a node of the row's own document (its node
    /// captures, not a `transform`'s text): a probe of one makes the documents its values name (decision 0494).
    fn node_columns(&self) -> Vec<(String, Vec<usize>)> {
        Vec::new()
    }
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
    /// `AstWave::node_columns`.
    pub fn node_columns(&self) -> Vec<(String, Vec<usize>)> {
        self.0.as_ref().map_or_else(Vec::new, |wave| wave.node_columns())
    }
}

/// What compiles a program's `ast` matchers (`rel`, the matcher's text, its captures) over its first `.policy`: the
/// engine's (`waves::policies`), registered at load.
pub trait AstCompiler: Send + Sync {
    /// `demand`: per matcher, the matchers whose roots hold every node its rows are read at (`None`: everywhere).
    fn compile(&self, matchers: &[(String, String, Vec<String>)], demand: &[Option<rerail_blocks::vector::Vector<usize>>], policies: &[serde_json::Value]) -> Result<Matchers, String>;
}

static AST_COMPILER: std::sync::OnceLock<&'static dyn AstCompiler> = std::sync::OnceLock::new();

/// Registers the compiler of `ast` matchers (once, at load).
pub fn install_ast_compiler(compiler: &'static dyn AstCompiler) {
    let _ = AST_COMPILER.set(compiler);
}

/// `matchers` compiled; a program with matchers and no registered compiler fails, naming the first.
pub fn compile_matchers(matchers: &[(String, String, Vec<String>)], demand: &[Option<rerail_blocks::vector::Vector<usize>>], policies: &[serde_json::Value]) -> Result<Matchers, String> {
    if matchers.is_empty() {
        return Ok(Matchers::default());
    }
    match AST_COMPILER.get() {
        Some(compiler) => compiler.compile(matchers, demand, policies),
        None => Err(format!("{}: no compiler of `ast` matchers is registered", matchers[0].0)),
    }
}

/// A batch's documents as given, made or pending (`Input`).
fn inputs(documents: Vec<impl Into<Input>>) -> Vec<Input> {
    documents.into_iter().map(Into::into).collect()
}

/// [`with_nearest`], with each of `matchers`' relations over the documents' tapes (decision 0106).
pub fn with_matchers(
    store: &ContentStore,
    documents: Vec<impl Into<Input>>,
    sets: &[(String, Vector<i64>)],
    matchers: &Matchers,
    tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    binding: crate::code_rule_dsl::facts::nearest::Binding,
    projection: &Projection,
    given: &HashMap<u32, made::ViewCounts>,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    Grower::default().sources_of(store, inputs(documents), Some(matchers), tests, binding, projection, given, false).map(|(out, batch)| finish(out, &batch, sets, projection, binding))
}

/// [`with_matchers`]'s sources of a session's corpus batch (decision 0616): renewed from `held` (its last run's
/// batch) where it can be, its groups keeping their strings for the next run's.
#[allow(clippy::too_many_arguments)]
pub fn with_matchers_renewed(
    store: &ContentStore,
    documents: Vec<impl Into<Input>>,
    sets: &[(String, Vector<i64>)],
    matchers: &Matchers,
    tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    binding: crate::code_rule_dsl::facts::nearest::Binding,
    projection: &Projection,
    given: &HashMap<u32, made::ViewCounts>,
    held: Option<Arc<Batch>>,
) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    let mut grower = Grower::default();
    grower.growth.renew = held;
    grower.growth.keeps_strings = true;
    grower.sources_of(store, inputs(documents), Some(matchers), tests, binding, projection, given, false).map(|(out, batch)| finish(out, &batch, sets, projection, binding))
}

/// Whether two projections make the same rows (decision 0616): every field but the compiled regexes' cache.
fn same_projection(a: &Projection, b: &Projection) -> bool {
    let bare = |p: &Projection| {
        let mut p = p.clone();
        p.compiled_rx = Default::default();
        format!("{p:?}")
    };
    bare(a) == bare(b)
}

/// A chunk's sources that grow by its rounds' documents (decision 0431 §6): the batch is made once, and a round's tape
/// values and programs are added to it (`Growth`), not built again with it. Each of the batch's relations is one
/// block relation (decision 0494), which the round's groups go on in (`Relation::grown`), its rows read through the
/// round's batch (`made::MadeRows`); the views and axes are made over the grown batch.
#[derive(Default)]
pub struct Grower {
    growth: Growth,
    /// Each relation with the rows of the tape values admitted since (decision 0431 §8, `admit`): the chunk's rows,
    /// then the values', in the same block relation. The chunk's relations stay its own rows.
    with_values: Vec<(String, Arc<Relation>)>,
    /// The last batch's text tests, whose probes the next round's keep: each string is tested once a run.
    tested: Vec<Arc<made::TextTest>>,
}

impl Grower {
    /// [`with_matchers`]'s sources, and the grower that adds a round's documents to them.
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        store: &ContentStore,
        documents: Vec<impl Into<Input>>,
        sets: &[(String, Vector<i64>)],
        matchers: &Matchers,
        tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
        given: &HashMap<u32, made::ViewCounts>,
    ) -> StoreResult<(Vec<(String, Arc<dyn Source>)>, Grower)> {
        let mut grower = Grower::default();
        let (out, batch) = grower.sources_of(store, inputs(documents), Some(matchers), tests, binding, projection, given, false)?;
        Ok((finish(out, &batch, sets, projection, binding), grower))
    }

    /// The sources with `documents` added: a round's, past every document the batch holds.
    pub fn grow(
        &mut self,
        store: &ContentStore,
        documents: Vec<impl Into<Input>>,
        sets: &[(String, Vector<i64>)],
        tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
        let (out, batch) = self.sources_of(store, inputs(documents), None, tests, binding, projection, &HashMap::new(), false)?;
        Ok(finish(out, &batch, sets, projection, binding))
    }

    /// The sources of tape values `documents`, admitted inside the chunk's fixpoint (decision 0431 §8): one answer's
    /// values, made together as one batch of their own, projected into the chunk's block, their hosts' lines and their
    /// strings the growth's, their rows a part of each of the chunk's sources (`values::Layered`). A document the batch
    /// holds (a value given a place more) has the rows of those places alone. Admitted one at a time, each value paid
    /// a batch's fixed work (its projection, text tests, axes and a copy of the parts' keys): on corpus0 × B-now, 13,483
    /// values made 40 GB of the run's peak and 55 s of its CPU.
    pub fn admit(
        &mut self,
        store: &ContentStore,
        documents: Vec<Document>,
        sets: &[(String, Vector<i64>)],
        tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
        let (out, batch) = self.sources_of(store, documents.into_iter().map(Input::from).collect(), None, tests, binding, projection, &HashMap::new(), true)?;
        Ok(finish(out, &batch, sets, projection, binding))
    }

    /// Tape value's relation `name`, its rows admitted (decision 0431 §8): written once, past the chunk's rows of
    /// `name` and the values' admitted before, in the same block relation (`Relation::grown`), so that the device
    /// binds the chunk's and the values' rows whole; read as the rows past those (`values::Slice`). A relation the
    /// chunk makes none of, or one read with its batch (`as_source`), is the value's own.
    fn admitted(&mut self, name: &str, rows: Relation, batch: &Arc<Batch>) -> Arc<dyn Source> {
        let chunk = self.growth.last.as_ref().map(|b| b.rows.whole()).unwrap_or_default();
        let at = self.with_values.iter().position(|(n, _)| n == name);
        let prev = at.map(|i| &self.with_values[i].1).or_else(|| chunk.get(name));
        let Some(prev) = prev.filter(|p| name != LONG_TEXT_NODES && Source::columns(p.as_ref()) == Source::columns(&rows)) else {
            return as_source(name, Arc::new(rows), batch);
        };
        let start = Source::len(prev.as_ref());
        let whole = Arc::new(prev.grown(rows));
        match at {
            Some(i) => self.with_values[i].1 = Arc::clone(&whole),
            None => self.with_values.push((name.to_owned(), Arc::clone(&whole))),
        }
        Arc::new(values::Slice::new(whole, start))
    }

    /// The batch's relations and text tests (`alone`: a tape value's, admitted), and the batch.
    #[allow(clippy::too_many_arguments)]
    fn sources_of(
        &mut self,
        store: &ContentStore,
        documents: Vec<Input>,
        matchers: Option<&Matchers>,
        tests: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
        given: &HashMap<u32, made::ViewCounts>,
        alone: bool,
    ) -> StoreResult<(Vec<(String, Arc<dyn Source>)>, Arc<Batch>)> {
        let built = crate::code_rule_dsl::logic::profile::span(|| "project and intern".into());
        let metal = matches!(binding, crate::code_rule_dsl::facts::nearest::Binding::Metal);
        if let Some(m) = matchers {
            self.growth.wave = Some((m.clone(), binding));
        }
        // A tape value alone is no file: the chunk's matchers' rows are the files'.
        let wave = if alone { None } else { self.growth.wave.clone() };
        let (batch, made) = Batch::add(&mut self.growth, store, documents, projection, &tests.matches, &tests.markers, metal, wave, given, alone)?;
        drop(built);
        let Some(made) = made else {
            // The batch's relations, the `match_all` relations (`preparation::text::TextRelations`: each regex's matches in
            // the batch's spans) and the `ast` matchers', made by group (decision 0494), but none the program derives.
            let mut out: Vec<(String, Arc<dyn Source>)> = made_sources(&batch).into_iter().map(|(name, _, source)| (name.to_owned(), source)).collect();
            // The text tests' relations (decision 0037): each a test of a string, which a probe asks of the probed
            // strings and a scan of the batch's (decision 0494), each string tested once a run.
            let answers = match self.tested.first() {
                Some(t) if t.tested().of(tests) => Arc::clone(t.tested()),
                _ => Arc::new(made::Tested::new(tests, store.atoms_handle())),
            };
            let scanned = Arc::new(std::sync::atomic::AtomicBool::new(false));
            let tested: Vec<Arc<made::TextTest>> = (0..tests.tests.len()).map(|k| Arc::new(made::TextTest::new(&batch, k, &answers, &scanned))).collect();
            out.extend(tests.tests.iter().zip(&tested).map(|((name, _), t)| (name.clone(), Arc::clone(t) as Arc<dyn Source>)));
            self.tested = tested;
            return Ok((out, batch));
        };
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, batch.nodes().last().copied().unwrap_or(0) as u64);
        // An admitted value's relations are a part of the chunk's (`admitted`), never merged into its own.
        let made::Made { relations, matched, waved: _, read: _, strings_from, used: _ } = made;
        let mut out: Vec<(String, Arc<dyn Source>)> = Vec::new();
        for (name, r) in relations.into_iter().filter(|(name, _)| !projection.derived.contains(*name)) {
            out.push((name.to_owned(), self.admitted(name, r, &batch)));
        }
        for (name, r) in matched {
            out.push((name.clone(), self.admitted(&name, r, &batch)));
        }
        // The text tests' relations (decision 0037): the value's new strings that pass each.
        let text = crate::code_rule_dsl::logic::profile::span(|| "text tests".into());
        let (strings, symbols) = {
            let shared = self.growth.shared.lock();
            (shared.strings.view(), shared.atoms.view())
        };
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, (strings.len() - strings_from) as u64);
        let tested: Vec<(String, Relation)> = tests
            .tests
            .iter()
            .zip(tests.rows(|i| strings.get(strings_from + i), &symbols[strings_from..]))
            .map(|((name, _), rows)| (name.clone(), Relation::staged(&["s"], &mut rows.into_iter().map(|s| rerail_blocks::vector![s]))))
            .collect();
        for (name, r) in tested {
            out.push((name.clone(), self.admitted(&name, r, &batch)));
        }
        drop(text);
        Ok((out, batch))
    }
}

/// `out` (the batch's relations and text tests) with the batch's views, its kind-set axes and containment's.
fn finish(mut out: Vec<(String, Arc<dyn Source>)>, batch: &Arc<Batch>, sets: &[(String, Vector<i64>)], projection: &Projection, binding: crate::code_rule_dsl::facts::nearest::Binding) -> Vec<(String, Arc<dyn Source>)> {
    let batch = Arc::clone(batch);
    let rows_named: BTreeSet<String> = out.iter().map(|(n, _)| n.clone()).collect();
    let viewing = crate::code_rule_dsl::logic::profile::span(|| "views".into());
    out.extend(views(&batch).into_iter().filter(|(name, _)| !projection.derived.contains(*name)).map(|(name, source)| (name.to_owned(), source)));
    places::join(&mut out, &batch);
    drop(viewing);
    // The kind-set axes, made per looked-up node like containment (decision 0050): a set no join reads, and the nodes
    // no join binds, cost nothing. Their statistics come from the tree.
    let _nearest = crate::code_rule_dsl::logic::profile::span(|| "nearest".into());
    use crate::code_rule_dsl::logic::front::ast::{AXIS, DESCENDANTS};
    // The axes over one set of kinds share their solves (`within::Solves`, decision 0494 step 7): each mode's is made
    // once, and an `inside` whose set nests nowhere is its `nearest`.
    let mut solves: Vec<(BTreeSet<i64>, Arc<within::Solves>)> = Vec::new();
    for (name, members) in sets {
        let axis = if name.starts_with(DESCENDANTS) {
            within::Axis::Has
        } else if name.starts_with(AXIS) {
            within::Axis::Inside
        } else {
            within::Axis::Nearest
        };
        let set: BTreeSet<i64> = members.iter().copied().collect();
        let shared = match solves.iter().find(|(s, _)| *s == set) {
            Some((_, shared)) => Arc::clone(shared),
            None => {
                let shared = Arc::new(within::Solves::default());
                solves.push((set, Arc::clone(&shared)));
                shared
            }
        };
        out.push((name.clone(), Arc::new(within::Within::axis(Arc::clone(&batch), axis, members, binding).sharing(shared))));
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
        let keys: Vector<u32> = batch.docs.keys().iter().copied().collect::<rerail_blocks::vector::Vector<_>>();
        drop(_nearest);
        stored_spike::run(&db, &read, &keys);
        return out_with_within(out, &batch, binding);
    }
    // Containment (decision 0050): made per looked-up node, so a program that never reads it pays nothing.
    out_with_within(out, &batch, binding)
}

/// `out` with containment's two relations (decision 0050).
/// Containment is solved where the run is bound (a Metal run's on the device, its batch's and its tape values' in one
/// relation, decision 0431 §8; the host's where the device cannot).
fn out_with_within(mut out: Vec<(String, Arc<dyn Source>)>, batch: &Arc<Batch>, binding: crate::code_rule_dsl::facts::nearest::Binding) -> Vec<(String, Arc<dyn Source>)> {
    out.push((WITHIN.to_owned(), Arc::new(within::Within::new(Arc::clone(batch), within::Mode::Tree).bound(binding))));
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

/// A relation's column statistics (`syntax::statistics`: rows, ndv, sq, sqd) measured over `first` rows, scaled to
/// `rows`. A key column stays a key; a column with many values (over one row in 16 distinct: node ids, strings) gains
/// values in proportion, one with few (kinds, field names) keeps them and its postings grow. `sq` keeps the postings'
/// sizes against the values, and `sqd` grows with the documents, whose postings keep their sizes.
pub fn scaled_statistics(measured: &[(String, i64, i64, i64, i64)], first: usize, rows: usize) -> Vec<(String, i64, i64, i64, i64)> {
    let r = rows as f64 / first as f64;
    let n = rows as i64;
    measured
        .iter()
        .map(|(name, m_rows, ndv, sq, sqd)| {
            if n == 0 {
                return (name.clone(), 0, 0, 0, 0);
            }
            if ndv == m_rows {
                return (name.clone(), n, n, n, n);
            }
            let ndv_scaled = if ndv * 16 > *m_rows { (*ndv as f64 * r).round() as i64 } else { *ndv }.clamp(1, n);
            let sq_scaled = (*sq as f64 * r * r * (*ndv as f64 / ndv_scaled as f64)).round() as i64;
            let sqd_scaled = (*sqd as f64 * r).round() as i64;
            (name.clone(), n, ndv_scaled, sq_scaled.max(n), sqd_scaled.max(n))
        })
        .collect()
}

/// `statistics`, measured from the rows (a source that keeps its own calls this once).
///
/// Two walks of the source (`for_rows`), no sort: the first finds the key columns (strictly increasing: every value
/// once) and whether the rows come in document order; the second counts each other column's values, over all rows
/// (ndv, and sq from each value's count) and within each document (sqd), a document's counts closed where its rows
/// end. Rows out of document order are measured by sorting (`measured_sorted`). The two give the same numbers.
pub fn measured(source: &dyn Source) -> Vec<(String, i64, i64, i64, i64)> {
    let rows = source.len();
    let names = source.columns();
    let width = names.len();
    let all: Vector<usize> = (0..width).collect();
    // The first walk: each column's strictly increasing, and the documents' order.
    let (mut increasing, mut ordered) = (vec![true; width], true);
    let mut last: Option<Vector<i64>> = None;
    source.for_rows(&all, &mut |_, row| {
        if let Some(was) = &mut last {
            ordered &= was[0] >> 32 <= row[0] >> 32;
            for c in 0..width {
                increasing[c] &= was[c] < row[c];
            }
            was.copy_from_slice(row);
        } else {
            last = Some(row.iter().copied().collect::<rerail_blocks::vector::Vector<_>>());
        }
    });
    if !ordered {
        return measured_sorted(source);
    }
    let key = (rows as i64, rows as i64, rows as i64, rows as i64);
    let counted: Vector<usize> = (0..width).filter(|&c| !increasing[c]).collect();
    // The second walk, of the other columns: every value's count, and its count in the document under way.
    struct Count {
        all: rustc_hash::FxHashMap<i64, i64>,
        doc: rustc_hash::FxHashMap<i64, i64>,
        sqd: i64,
    }
    impl Count {
        fn close(&mut self) {
            self.sqd += self.doc.drain().map(|(_, n)| n * n).sum::<i64>();
        }
    }
    let mut counts: Vec<Count> = counted.iter().map(|_| Count { all: Default::default(), doc: Default::default(), sqd: 0 }).collect();
    if !counted.is_empty() {
        let mut read: Vector<usize> = rerail_blocks::vector![0];
        read.extend(counted.iter().copied().filter(|&c| c != 0));
        let at: Vector<usize> = counted.iter().map(|c| read.iter().position(|r| r == c).expect("read")).collect();
        let mut doc = None;
        source.for_rows(&read, &mut |_, row| {
            let d = row[0] >> 32;
            if doc != Some(d) {
                doc = Some(d);
                counts.iter_mut().for_each(Count::close);
            }
            for (count, &i) in counts.iter_mut().zip(&at) {
                *count.all.entry(row[i]).or_default() += 1;
                *count.doc.entry(row[i]).or_default() += 1;
            }
        });
        counts.iter_mut().for_each(Count::close);
    }
    let shadow = crate::code_rule_dsl::logic::plan::sketch::shadowing();
    let mut counts = counts.into_iter();
    names
        .into_iter()
        .enumerate()
        .map(|(c, name)| {
            if increasing[c] {
                return (name, key.0, key.1, key.2, key.3);
            }
            let count = counts.next().expect("a count per counted column");
            let sq = count.all.values().map(|n| n * n).sum::<i64>();
            if shadow {
                let mut sketch = crate::code_rule_dsl::logic::plan::sketch::Sketch::default();
                let mut values: Vec<(i64, i64)> = count.all.iter().map(|(v, n)| (*v, *n)).collect();
                values.sort_unstable();
                for (v, n) in values {
                    sketch.add(v, n);
                }
                crate::code_rule_dsl::logic::plan::sketch::shadow("measured", &name, rows as i64, count.all.len() as i64, sq, &sketch);
            }
            (name, rows as i64, count.all.len() as i64, sq, count.sqd)
        })
        .collect()
}

/// `measured` by sorting each column's (value, document) pairs: for rows out of document order.
pub fn measured_sorted(source: &dyn Source) -> Vec<(String, i64, i64, i64, i64)> {
    let rows = source.len();
    // Each row's document (the first column's high half), read once for every column. Every read here is the
    // source's walk (`for_rows`): a view of a batch's documents reads each document's nodes once, where a read by
    // row locates the row in the batch first (0.9 s for a hogql run's two largest base relations, three such reads
    // a value).
    let mut docs: Vector<i64> = rerail_blocks::vector::Vector::with_capacity(rows);
    source.for_rows(&[0], &mut |_, v| docs.push(v[0] >> 32));
    // One sort per column by (value, document): a run of one value is its postings (ndv, and the sum of squares of
    // their lengths), and within it a run of one document is that document's (the sum of squares per document).
    source
        .columns()
        .into_iter()
        .enumerate()
        .map(|(c, name)| {
            let mut values: Vec<(i64, i64)> = Vec::with_capacity(rows);
            source.for_rows(&[c], &mut |r, v| values.push((v[0], docs[r])));
            // A key column (strictly increasing, as a node column of a relation in node order): every value once,
            // and so every (document, value) once. No sort.
            if values.windows(2).all(|w| w[0].0 < w[1].0) {
                return (name, rows as i64, rows as i64, rows as i64, rows as i64);
            }
            values.sort_unstable();
            let (mut ndv, mut sq, mut sqd) = (0i64, 0i64, 0i64);
            // The shadow's sketch, made only when a shadow is written: a sketch is a block's chunk, and one a column
            // that nothing reads took 177 MB of blocks on the data layer's 50 files (R7 against 0458).
            let mut sketch = crate::code_rule_dsl::logic::plan::sketch::shadowing().then(crate::code_rule_dsl::logic::plan::sketch::Sketch::default);
            let mut at = 0;
            while at < values.len() {
                let v = values[at].0;
                let end = at + values[at..].partition_point(|x| x.0 == v);
                if let Some(sketch) = &mut sketch {
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
            if let Some(sketch) = &sketch {
                crate::code_rule_dsl::logic::plan::sketch::shadow("measured", &name, rows as i64, ndv, sq, sketch);
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
