//! T's base relations served from a batch of core facts (decision 0008; 05 Phase 7 row 3): the
//! relations Typing-support's exporter (`spikes/datalog-engines/exporter/src/main.rs`) wrote as
//! rows, here read off `facts::core` columns when a statement filters them.
//!
//! The rows are T's: over each document's named nodes other than comments, in preorder,
//!
//! ```text
//! doc(d, path, module)   pkg(d, p)   unpackaged(d)   node(d, n, kind, start, end_)
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

use super::{Modules, Relation, Source};
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::core::{Facts, NONE, TEXT, kind_atom, kind_code};
use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::Connection;
use crate::code_rule_dsl::logic::front::ast::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use cells::{Block, Cells};

pub mod within;
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
pub mod ranks;
mod cells;
/// A projection's numbering's reads, written once in CLIF (`rerail_clif::id_reads`, decision 0353 step 2).
#[rustfmt::skip]
pub mod id_reads;
#[cfg(any(target_os = "macos", target_os = "linux"))]
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
        Projection { shape: Shape::Core, reads: None, rx_spans: Vec::new(), ast_patterns: Vec::new(), text_spans: None, token_text: true, node_anchors: None, long_text: BTreeSet::new(), long_text_any: false, text_nodes: None, partial: BTreeSet::new(), verdict: false, derived: BTreeSet::new(), compiled_rx: CompiledSpans::default() }
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
        Projection { shape, reads: Some(reads), rx_spans, ast_patterns, text_spans: Some(spans), token_text, node_anchors: None, long_text: BTreeSet::new(), long_text_any: false, text_nodes: None, partial, verdict, derived, compiled_rx: CompiledSpans::default() }
    }

    /// This projection, with `text` made at any length for the node kinds whose text the program's rules read
    /// (`demand`), and its phase's node anchors (`anchors`): found by its caller over its version
    /// (`logic::plan::text_demand::projection`).
    pub fn with_long_text_parts(mut self, demand: TextDemand, anchors: Result<BTreeSet<String>, String>) -> Self {
        let TextDemand { long, any, read, wide } = demand;
        self.long_text = long;
        self.long_text_any = !any.is_empty();
        crate::code_rule_dsl::logic::profile::decision("text past the inline bound", || {
            if self.long_text_any { format!("every node ({} rules read the text of a node of any kind)", any.len()) } else { format!("{} kinds: {}", self.long_text.len(), self.long_text.iter().take(12).cloned().collect::<Vec<_>>().join(", ")) }
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
    /// `id` is the node's tape node (`Projected::node`), which the caller's walk holds.
    pub(crate) fn reads(&self, p: &Projected, id: u32, dialect: &Dialect) -> bool {
        let f = &*p.facts;
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

/// What the rules read of `text` (`logic::plan::text_demand::text_kind_demand`).
pub struct TextDemand {
    /// The kinds whose text is read at any length.
    pub long: BTreeSet<String>,
    /// The text atoms (rule, body position) of a node of any kind, no twin reading its long text.
    pub any: Vec<(usize, usize)>,
    /// The nodes whose text (within the bound) is read; `None` every node.
    pub read: Option<TextNodes>,
    /// Where `read` is every node: the first rule that reads the text of a node it does not narrow.
    pub wide: Option<String>,
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
    nodes: Arc<Relation>,
    batch: Arc<Batch>,
    /// Each document key's place in `batch.docs`.
    docs: HashMap<u32, usize>,
}

impl LongTexts {
    fn new(nodes: Arc<Relation>, batch: &Arc<Batch>) -> Self {
        let docs = batch.docs.iter().enumerate().map(|(i, p)| (p.key, i)).collect();
        Self { nodes, batch: Arc::clone(batch), docs }
    }

    /// Node `n`'s text when it is past `text`'s inline bound (`core_rr::TEXT_INLINE`); none for a shorter node
    /// (whose text `text` holds), a span, or a node of a document not in the batch. Only the nodes a rule binds are
    /// read and interned (decision 0108 §1).
    pub fn long_text(&self, n: i64) -> Option<String> {
        let (key, low) = ((n >> 32) as u32, n as u32);
        let p = &self.batch.docs[*self.docs.get(&key)?];
        let local = p.local_of(low)?;
        let core = p.ranks().select(Item::Node, local)?;
        let text = p.facts.text(core);
        (text.len() > core_rr::TEXT_INLINE).then(|| text.to_owned())
    }

    /// Node `n`'s exact source text at any length, or a comment span's (a comment no node is: `Extra`'s `node_text`
    /// rows); none for another span or a node of a document not in the batch. It answers `node_text`'s requests
    /// (decision 0174): only the nodes a rule asks for are read and interned.
    pub fn text_any(&self, n: i64) -> Option<String> {
        let (key, low) = ((n >> 32) as u32, n as u32);
        let p = &self.batch.docs[*self.docs.get(&key)?];
        let f = &p.facts;
        if let Some(k) = granules::comment_of(low) {
            // The k-th comment kind's node not inside another that no node is, as `Extra::nodes` numbers them.
            use crate::code_rule_dsl::language::is_comment_kind;
            let dialect = self.batch.dialects.get(p.tag)?;
            let mut j = 0;
            for (id, up) in f.with_parents(0, f.len() as u32) {
                if !f.named(id) || !is_comment_kind(dialect.name(f, id)) || up.is_some_and(|q| is_comment_kind(dialect.name(f, q))) {
                    continue;
                }
                if p.local_of_node(id as u32) == NONE {
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
            if let Some(reader) = crate::code_rule_dsl::facts::core::value_reader() {
                reader.values(&valued.tape, valued.file.as_ref(), true, false, &mut |followed, ordinal, v, _| {
                    let local = p.local_of_node(ordinal as u32);
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
        let f = &*p.facts;
        match self.batch.shape {
            Shape::Core => core_rr::node_str(f, dialect, x),
            Shape::T => match dialect.class(f, x) {
                Class::String => dialect.string(f, x).and_then(|(_, content)| content.map(str::to_owned)),
                _ => None,
            },
        }
    }

    /// Node `n`'s text or value as function `name` reads it (`NODE_READS`) through `source`, a binding of
    /// `rr_long_text_node`; none for another source or function.
    pub fn of(source: &dyn Source, name: &str, n: i64) -> Option<String> {
        // A chunk's layered binding (decision 0431 §8): a tape value's node is read through its own batch.
        if let Some(layered) = source.as_any()?.downcast_ref::<values::Layered>() {
            return Self::of(&*layered.source_of(n), name, n);
        }
        let texts = source.as_any()?.downcast_ref::<Self>()?;
        match name {
            "rr_node_text" => texts.long_text(n),
            "rr_node_text_any" => texts.text_any(n),
            "rr_node_value" => texts.value_any(n),
            "rr_node_str" => texts.str_any(n),
            _ => None,
        }
    }
}

impl Source for LongTexts {
    fn python_batch(&self) -> Option<Arc<Batch>> {
        Some(Arc::clone(&self.batch))
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
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.nodes.estimated_matches(column, batch)
    }
    fn indexed(&self, column: usize) -> bool {
        self.nodes.indexed(column)
    }
    fn range_indexed(&self, column: usize) -> bool {
        self.nodes.range_indexed(column)
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        self.nodes.range_lookup(column, lower, upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.nodes.lookup(column, values)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.nodes.value(row, column)
    }
}

/// The batch's relation `name` as a source: `rr_long_text_node` with the batch (`LongTexts`), any other as rows.
/// How a batch's relations are made of their rows: written into a block (`Relation::from_rows`), or staged
/// (`Relation::staged`).
pub(super) type Make = fn(&[&str], &mut dyn Iterator<Item = Vec<i64>>) -> Relation;

fn as_source(name: &str, rows: Arc<Relation>, batch: &Arc<Batch>) -> Arc<dyn Source> {
    if name == LONG_TEXT_NODES { Arc::new(LongTexts::new(rows, batch)) } else { rows }
}

/// The relation of the nodes whose text `@rr_node_text` reads, bound with the batch every node read goes through
/// (`LongTexts`).
pub const LONG_TEXT_NODES: &str = "rr_long_text_node";

/// The functions that read a node's text or value from the run's batch (`LongTexts::of`): their values are the run's,
/// so `functions::eval` gives none.
pub const NODE_READS: [&str; 4] = ["rr_node_text", "rr_node_text_any", "rr_node_value", "rr_node_str"];

/// The request relation of `value` (decision 0174): a batch keeps its documents' tapes for W's values where a program
/// derives it.
pub const VALUE_REQ: &str = "rr_value_req";

/// A document's tape and the values W reads off it (`ValueReader`), read at the first request.
pub(crate) struct Valued {
    tape: crate::code_rule_dsl::tape::TapeDoc,
    file: Arc<dyn crate::code_rule_dsl::facts::core::SourceFile>,
    values: OnceLock<HashMap<u32, String>>,
}

/// The relation a program's guest settings are facts of (`.guests`, `.on guest_error`; decision 0055):
/// `guest_policy("library", "on" | "off")`, `guest_policy("errors" | "errors:GRAMMAR", "skip" | "partial")`.
pub const GUEST_POLICY: &str = "guest_policy";

/// The first guest document's key: files take `1..GUEST_KEYS` (`corpus::memo::doc_keys`), code guests in the run's
/// language `GUEST_KEYS..ANNOTATION_KEYS`, annotations (type expressions, Python's in a Python run)
/// `ANNOTATION_KEYS..HIDDEN_KEYS`, and every other document (a guest in another grammar, a value) `HIDDEN_KEYS..2^31`
/// (decision 0058 §1: what a rule sees is a bound on these keys). A rule that does not name a guest sees files and code
/// guests; a relation an annotation's rows reach (`corpus::preparation::combining::FoldOwn`) sees annotations too.
pub const GUEST_KEYS: u32 = 1 << 30;
pub const ANNOTATION_KEYS: u32 = (1 << 30) + (1 << 28);
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
    /// A Python file no regular package holds: one of the root, or whose folder has no `__init__.py` (core.rerail's
    /// `unpackaged`, `corpus::package_root_modules`).
    pub unpackaged: bool,
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
    PRODUCERS.get().map_or(&[], Vec::as_slice)
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
        Some((l, id, dialect.name(&p.facts, id)))
    }
    /// Local id `local`'s core id and kind name.
    pub fn node(&self, local: u32) -> Option<(u32, &'static str)> {
        let (p, dialect) = self.p.as_ref()?;
        (local < p.len()).then(|| p.node(local)).map(|id| (id, dialect.name(&p.facts, id)))
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
        p.ranks().is(Item::Node, id).then(|| dialect.name(&p.facts, id))
    }
}

pub fn numbering(doc: &Document, shape: Shape) -> StoreResult<Numbering> {
    // A document read as text only has no nodes.
    if doc.facts.len() == 0 {
        return Ok(Numbering { p: None });
    }
    let tag = crate::code_rule_dsl::facts::core::facts_tag(&doc.facts);
    let dialect = Dialect::kept(tag)?;
    // In `core.rerail`'s shape a guest's nodes are its root's subtree; in T's, its whole tree (`Batch::build`).
    let guest = doc.guest.as_deref().filter(|_| shape == Shape::Core).map(|g| (g, Arc::new(core_rr::Lines::new("", false))));
    let p = Projected::new(doc.key, Arc::clone(&doc.facts), Some(&*dialect), shape, false, guest);
    Ok(Numbering { p: Some((p, dialect)) })
}

/// Node `id`'s kind name in its facts' grammar (`""` where the facts' tag names no dialect).
pub fn kind_name(f: &Facts, id: u32) -> &'static str {
    let tag = crate::code_rule_dsl::facts::core::facts_tag(f);
    Dialect::kept(tag).map_or("", |d| d.name(f, id))
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
    pub canonical: Vec<u16>,
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
        let canonical: Vec<u16> = (0..catalog.kind_count() as u16)
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
    pub(super) fn code(&self, f: &Facts, id: u32) -> u16 {
        kind_code(f.kind(id))
    }
    fn class(&self, f: &Facts, id: u32) -> Class {
        self.classes.get(usize::from(self.code(f, id))).copied().unwrap_or(Class::Other)
    }
    pub fn name(&self, f: &Facts, id: u32) -> &'static str {
        self.code_name(self.code(f, id))
    }
    /// Kind code `code`'s name; one past the grammar's kinds (ERROR's) is the catalog's.
    fn code_name(&self, code: u16) -> &'static str {
        self.names.get(usize::from(code)).copied().unwrap_or_else(|| self.catalog.kind_name(code).unwrap_or(""))
    }
    /// Node `id`'s kind atom as the `kind` column holds it: its name's canonical code's.
    fn kind(&self, f: &Facts, id: u32) -> i64 {
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
    fn field_child(&self, f: &Facts, id: u32, field: &str) -> Option<u32> {
        let field = self.catalog.field_id(field)?;
        f.children(id).iter().copied().find(|&c| f.field(c) == field)
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
/// (`core.rerail`'s).
pub struct Projected {
    pub module: Option<crate::code_rule_dsl::module_path::ModulePath>,
    pub key: u32,
    /// The key of the file it is part of (decision 0431 §4): its own, or a program's file's (`Guest::file`). Its
    /// `docof`; its nodes keep its own key, which hiding reads.
    pub file: u32,
    pub tag: u8,
    pub facts: Arc<Facts>,
    /// The tape nodes it numbers, its nodes and tokens, as a rank index (`ranks::Ranks`, decision 0353 step 2): a
    /// node's local id is its rank among the nodes. In its batch's block (decision 0289 §3), which the device reads.
    ranks: ranks::Ranks,
    /// Each node's place among its parent's children, derived on the first point read (`idx`).
    idx: OnceLock<Cells<u32>>,
    /// Walked by anchors: an outermost anchor hangs from the root, which is not its tape parent, so a walk's parent
    /// is not always the tape's (a sibling is then read off the tape, `prev_sibling`).
    anchored: bool,
    /// `core.rerail`'s lines and spans; `None` in T's shape.
    pub extra: Option<Box<Locs>>,
    /// Node ids numbered per definition (`DefIds`), when the run asks for them; else a node's id is its preorder.
    defs: Option<DefIds>,
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
    rerail_prof::code_rule_dsl::logic::profile::setting(|| !std::env::var("RERAIL_NODE_IDS").is_ok_and(|v| v == "preorder"))
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
        DefIds { table: granules::number(p, dialect, tokens, None) }
    }
}

/// A document's lines, and its spans past its nodes (`Shape::Core`'s): from local id `len()`, its anonymous tokens
/// (`tokens` of them, in preorder), then its whole text, then each line. Not `Extra`'s spans (`SPAN`), which the
/// relations core.rerail's programs read beyond T's are made over.
pub struct Locs {
    lines: Arc<core_rr::Lines>,
    pub tokens: u32,
    /// Each node's end point (`end_point`, where a program reads it): a zero-width span at its end offset, after the
    /// lines, one per node in local order. 0 where no program reads them.
    pub ends: u32,
    /// The whole source and its lines follow the tokens, implicit (`Projected::span`): a guest has none of its own.
    own_text: bool,
    /// A guest's host lines and placement: its places are the host's (decision 0055). A guest has no source-text
    /// spans of its own (`source`, `src_line`): those are its file's.
    host: Option<(Arc<core_rr::Lines>, Placement)>,
}

impl Locs {
    /// The document's spans past its nodes: its tokens, then (its own text) the whole source and each line.
    fn positions(&self) -> u32 { self.ends_from() + self.ends }
    /// The first end point's position: past the tokens, the whole source and the lines.
    fn ends_from(&self) -> u32 { self.tokens + if self.own_text { 1 + self.lines.count() as u32 } else { 0 } }
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
    /// node). A `withheld` document (`core.rerail`: one that does not parse) has no nodes and no spans: no facts. A guest's
    /// nodes are its root's subtree, placed in its host's lines (`host`).
    /// Its arrays are in a block of its own; a batch's documents' are in the batch's (`narrowed`).
    pub fn new(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape, withheld: bool, guest: Option<(&Guest, Arc<core_rr::Lines>)>) -> Self {
        let block = cells::block(Self::bytes(&facts));
        Self::narrowed(key, facts, dialect, shape, withheld, guest, None, None, &block)
    }
    /// `new`, narrowed to `anchors` (`narrowed`): the root and each outermost anchor's subtree.
    pub fn anchored(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape, anchors: &BTreeSet<String>) -> Self {
        let block = cells::block(Self::bytes(&facts));
        Self::narrowed(key, facts, dialect, shape, false, None, None, Some(anchors), &block)
    }

    /// What a projection's arrays can take in a block: `core`, `local`, `up`, `low`, the ordinals and the fielded
    /// nodes, none longer than the tape, with room for alignment.
    pub fn bytes(facts: &Facts) -> usize {
        // The views' entry and its codes' table too (`view_docs::ViewEntry`).
        6 * 4 * facts.len() + 6 * 16 + 8 * facts.code_count() + 256
    }

    /// As `new`, but with `anchors` (`Projection::node_anchors`) a file's nodes are its root and the subtrees of the
    /// nodes of those kinds, found by the kind postings: no other node is walked. An anchor below no other is its
    /// root's child here; the program reads no node outside the subtrees, nor an anchor's parent or place among its
    /// siblings, and its node ids stay inside its phase.
    #[allow(clippy::too_many_arguments)]
    /// `known`: the document's own lines where a guest of it was projected first and built them (decision 0470 step 2).
    pub fn narrowed(key: u32, facts: Arc<Facts>, dialect: Option<&Dialect>, shape: Shape, withheld: bool, guest: Option<(&Guest, Arc<core_rr::Lines>)>, known: Option<Arc<core_rr::Lines>>, anchors: Option<&BTreeSet<String>>, block: &Arc<Block>) -> Self {
        let identity_span = crate::code_rule_dsl::logic::profile::span(|| "identity maps".into());
        let f = &*facts;
        let (first, last) = match guest {
            Some((g, _)) => (g.root, if (g.root as usize) < f.len() { f.subtree_end(g.root) } else { g.root + 1 }.max(g.root + 1)),
            None => (0, f.len() as u32),
        };
        let end = if withheld { first } else { last.min(f.len() as u32) };
        // The id ranges walked: the document's, or (`anchors`) its root's and each outermost anchor's subtree.
        let ranges: Vec<(u32, u32)> = match (anchors, dialect) {
            (Some(kinds), Some(dialect)) if guest.is_none() && first < end => {
                let mut starts: Vec<u32> = f.kinds().into_iter().filter(|&k| kinds.contains(dialect.atom_name(k))).flat_map(|k| f.of_kind(k)).collect();
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
                    if !f.named(c) && f.end(c) > f.start(c) && outside(c) {
                        marks.mark(Item::Token, c);
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
                    if core_tokens && f.end(id) > f.start(id) && above.is_some_and(|q| marks.has(Item::Node, q)) {
                        marks.mark(Item::Token, id);
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
                if n != 0 && f.field(id) != 0 {
                    marks.mark(Item::Field, id);
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
        });
        let tokens = ranks.count(Item::Token);
        let file = guest.as_ref().map(|(g, _)| *g).filter(|g| g.made == Made::Host && g.file != 0).map_or(key, |g| g.file);
        let mut p = Projected {
            module: None,
            key,
            file,
            tag: f.tag,
            ranks,
            idx: OnceLock::new(),
            anchored: ranges.len() > 1,
            extra: None,
            defs: None,
            facts: Arc::clone(&facts),
        };
        drop(identity_span);
        if shape == Shape::Core {
            let lines_span = crate::code_rule_dsl::logic::profile::span(|| "line index".into());
            let lines = known.unwrap_or_else(|| Arc::new(core_rr::Lines::shared(Arc::clone(&f.source), dialect.is_some_and(|d| d.family == Family::Python), p.block())));
            drop(lines_span);
            let _ids_span = crate::code_rule_dsl::logic::profile::span(|| "definition ids".into());
            if definition_ids() && let Some(dialect) = dialect {
                // Each token's node, in preorder of the tokens (`token_order`).
                let owners: Vec<u32> = p.token_order().into_iter().map(|(_, l)| l).collect();
                debug_assert_eq!(owners.len(), tokens as usize);
                p.defs = Some(DefIds::new(&p, dialect, &owners));
            }
            // Raw file text remains valid when its tree is withheld. Guests have no own text spans.
            let own_text = guest.is_none();
            let host = guest.filter(|(g, _)| g.made == Made::Host).map(|(g, lines)| (lines, g.in_file.clone()));
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
            open.push((l, x, self.facts.subtree_end(x)));
            (l, x, parent)
        })
    }
    /// Every node's and span's byte span, in local order (`span`): the nodes in one pass of the index, then the spans
    /// past them (tokens, the text, lines, end points). No select a node.
    pub fn spans(&self) -> impl Iterator<Item = (u32, [u32; 2])> + '_ {
        let f = &*self.facts;
        self.nodes().map(|(l, x)| (l, [f.start(x), f.end(x)])).chain((self.len()..self.locs()).map(|l| (l, self.span(l))))
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
        match self.facts.parent_within(x, anchor).map_or(NONE, |q| self.local_of_node(q)) {
            NONE => 0,
            p => p,
        }
    }
    /// Local id `local`'s place among its parent's children (`child`'s `i`). The tape holds no sibling's place, and
    /// hops along the siblings are as many as they are (a module's statements: a probe for each of them would be
    /// quadratic), so a point read reads a derived column: the walk's places, written once into the block on the
    /// projection's first point read (decision 0353 step 3). A scan carries the place in its walk and never makes it.
    #[inline]
    pub fn idx(&self, local: usize) -> u32 {
        self.places_column()[local]
    }
    /// Every numbered node's place (`idx`), the derived column, made on its first read.
    pub fn places_column(&self) -> &Cells<u32> {
        self.idx.get_or_init(|| {
            let _span = crate::code_rule_dsl::logic::profile::span(|| "projection places (derived, on a point read)".into());
            // copy: derived (each node's place, computed once here from the walk; no block holds it)
            Cells::build(self.block(), self.len() as usize, |out| {
                for (l, idx, _) in self.places() {
                    out[l as usize] = idx;
                }
                out.len()
            })
            .expect("a batch's block holds its projection")
        })
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
            open.push((self.facts.subtree_end(x), 0));
            (l, idx, depth)
        })
    }
    /// The nodes under local id `local`: those of its tape subtree, by rank.
    #[inline]
    pub fn size(&self, local: usize) -> u32 {
        let id = self.node(local as u32);
        self.ranks.rank(Item::Node, self.facts.subtree_end(id)) - local as u32 - 1
    }
    /// Local id `local`'s numbered sibling before it (`next_sibling(a, local)`'s `a`): the tape node before it is in
    /// its previous tape sibling's subtree, climbed to that sibling, and past siblings left out. Only for a node after
    /// a sibling (`Item::Sibling`), which has one.
    pub fn prev_sibling(&self, local: u32) -> Option<u32> {
        let x = self.node(local);
        if !self.ranks.is(Item::Sibling, x) {
            return None;
        }
        let q = self.facts.parent(x)?;
        let mut c = x - 1;
        loop {
            while let Some(up) = self.facts.parent(c)
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
        let q = self.facts.parent(x).filter(|&q| self.ranks.is(Item::Node, q))?;
        let end = self.facts.subtree_end(q);
        let mut s = self.facts.subtree_end(x);
        while s < end {
            if self.ranks.is(Item::Node, s) {
                return Some(self.ranks.rank(Item::Node, s));
            }
            s = self.facts.subtree_end(s);
        }
        None
    }
    /// The nodes after a sibling (`next_sibling`'s rows).
    pub fn after_siblings(&self) -> u32 {
        self.ranks.count(Item::Sibling)
    }
    /// Local id `local`'s depth, the root's 0: its numbered tape ancestors, by a climb (decision 0353 step 3). A scan
    /// carries it in its walk.
    pub fn depth(&self, local: usize) -> u32 {
        let mut x = self.node(local as u32);
        let mut d = 0;
        while let Some(q) = self.facts.parent(x) {
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
        let f = &self.facts;
        self.ranks.iter(Item::Token).map(|(_, x)| (x, f.parent(x).map_or(0, |q| self.local_of_node(q)))).collect()
    }
    /// The granule table, where ids are by definition (decision 0353 step 2).
    pub fn table(&self) -> Option<&granules::Table> {
        self.defs.as_ref().map(|d| &d.table)
    }
    /// The ids' low bits (nodes, then tokens) and the order keys as arrays, numbered again: the reference the table is
    /// checked and measured against, not a reader's path.
    pub fn reference(&self, dialect: &Dialect) -> granules::Reference {
        let owners: Vec<u32> = if self.extra.is_some() { self.token_order().into_iter().map(|(_, l)| l).collect() } else { Vec::new() };
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
                [self.facts.start(id), self.facts.end(id)]
            }
            Some(i) => self.extra.as_ref().map_or([0, 0], |x| {
                if i < x.tokens {
                    // The token's tape node, a select on the rank index: its span is the tape's.
                    let c = self.ranks.select(Item::Token, i).expect("a numbered token");
                    [self.facts.start(c), self.facts.end(c)]
                } else if i >= x.ends_from() {
                    let id = self.node(i - x.ends_from());
                    [self.facts.end(id), self.facts.end(id)]
                } else if i == x.tokens {
                    [0, self.facts.source.len() as u32]
                } else {
                    x.lines.span((i - x.tokens) as usize)
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
        self.facts.children(x).into_iter().map(move |c| self.local_of_node(c)).filter(|&c| c != NONE)
    }
    /// `children`, each handed to `f`: a parent's probe makes no list of them.
    pub fn each_child(&self, local: u32, mut f: impl FnMut(u32)) {
        self.each_child_node(local, |c, _| f(c));
    }
    /// `each_child`, with each child's tape node: `f(local id, tape node)`.
    pub fn each_child_node(&self, local: u32, mut f: impl FnMut(u32, u32)) {
        self.facts.each_child(self.node(local), |x| {
            let c = self.local_of_node(x);
            if c != NONE {
                f(c, x);
            }
        });
    }
}

/// One wave's documents, sorted by key, and the row offsets of the views.
pub struct Batch {
    /// Each document's projection, shared with the batches it grows into (`Growth`, decision 0431 §6).
    pub docs: Vec<Arc<Projected>>,
    /// The documents' keys, in their order (ascending): the index a node's document is searched in (`doc`). Searched
    /// in `docs` itself, each step read a different projection's line (Q220: `Batch::node` was a fifth of a join's
    /// view reads).
    keys: Vec<u32>,
    /// The programs among the documents (decision 0431 §4), by file: `(file, document)`, ascending; a lookup of `node`
    /// by its document reads a file's programs' rows with its own.
    programs: Vec<(u32, usize)>,
    /// Each document's tape, for W's values answered on request (`LongTexts::value_any`, decision 0174), and its values,
    /// read off it at the first request; empty where no program asks for values.
    valued: Vec<Option<Arc<Valued>>>,
    pub dialects: Dialects,
    pub nodes: Vec<usize>,
    children: Vec<usize>,
    fields: Vec<usize>,
    siblings: Vec<usize>,
    /// Nodes and spans (`Projected::locs`).
    locs: OnceLock<Vec<usize>>,
    /// The `node` view's rows by kind (`by_kind`), made at the first lookup by kind.
    kinds: OnceLock<KindRows>,
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    chunk: OnceLock<device_chunk::Chunk>,
}

/// What a chunk's batch keeps to grow by a round's documents (decision 0431 §6): its block, which the later documents
/// are projected into; its documents' projections, shared by each batch it grows into; its strings and their atoms;
/// the files' lines their guests are placed by. A round's documents are projected once, and the rows made are theirs.
#[derive(Default)]
pub struct Growth {
    docs: Vec<Arc<Projected>>,
    keys: Vec<u32>,
    valued: Vec<Option<Arc<Valued>>>,
    dialects: Dialects,
    strings: Vec<String>,
    at: rustc_hash::FxHashMap<String, usize>,
    atoms: Vec<u32>,
    lines_of: HashMap<u32, Arc<core_rr::Lines>>,
    host: HashMap<u32, u32>,
    made: BTreeSet<&'static str>,
    block: Option<Arc<Block>>,
    /// The producers of its relations, kept: a later round's guests are read with what they know of the files.
    produced: Option<Vec<(Option<u8>, Box<dyn ProducerRun>)>>,
    /// Each tape value's places the batch has rows of, and whether it is withheld (no root): a round that gives it
    /// more adds their rows alone (`GuestRows::places_added`).
    places: HashMap<u32, (usize, bool)>,
}

/// The `node` view's rows by kind: an index of the view (decision 0353: an index in its relation's place, not a copy of
/// its rows), made in one walk of each document's numbered nodes (`Projected::nodes`, no select) at the batch's first
/// lookup by kind. Each match keeps its document, local id and tape node, so a lookup reads no rank and no select.
/// Q220: a probe `kind($m, $k)` scanned every document's tape for the kind's codes (`Facts::of_kind`), 55 k probes
/// of a cold run at 39 µs each, where t41's `kind`, a relation of its own, had an index.
struct KindRows {
    /// The kinds, ascending.
    keys: Vec<i64>,
    /// Where each kind's matches start in `at`, and one past the last.
    starts: Vec<u32>,
    /// Each kind's nodes in row order: `(document, local id, tape node)`.
    at: Vec<(u32, u32, u32)>,
}

impl KindRows {
    fn new(b: &Batch) -> Self {
        let mut all: Vec<(i64, u32, u32, u32)> = Vec::with_capacity(b.nodes.last().copied().unwrap_or(0));
        for (doc, p) in b.docs.iter().enumerate() {
            let Some(dialect) = b.dialects.get(p.tag) else { continue };
            all.extend(p.nodes().map(|(l, x)| (dialect.kind(&p.facts, x), doc as u32, l, x)));
        }
        // Stable: within a kind, the rows stay in order.
        all.sort_by_key(|e| e.0);
        let (mut keys, mut starts) = (Vec::new(), Vec::new());
        for (i, e) in all.iter().enumerate() {
            if keys.last() != Some(&e.0) {
                keys.push(e.0);
                starts.push(i as u32);
            }
        }
        starts.push(all.len() as u32);
        KindRows { keys, starts, at: all.into_iter().map(|(_, d, l, x)| (d, l, x)).collect() }
    }
}

/// A span's id (core.rerail's `span`: a piece of a document's text): `d << 32 | granules::extra_low(kind, i)`, in slots of
/// its kind's past every node of its document, so no node view has a row of it (`Batch::node`). `ExtraSpan::Line` `0`
/// is the whole source (`source`), `Line` `l` physical line `l` (`src_line`), `Lf` `l` the `\n`-line `l`
/// (`src_line_lf`), `Tape` `l` the tape's line `l` (`tape_line`), `Comment` `k` the document's `k`-th comment from 1,
/// in source order, of those no node is (`comment`: T's nodes leave out a grammar's `comment` kind, as Python's, but
/// keep Rust's `line_comment`, which is then the comment's id) and `Match` `i` the document's `i`th match or group of
/// a `match_all`, `match_group` or `rx_span` regex. A document with more of a kind than its capacity fails, named.
fn extra_span(key: i64, kind: granules::ExtraSpan, i: u32) -> StoreResult<i64> {
    granules::extra_low(kind, i).map(|low| key << 32 | i64::from(low)).ok_or_else(|| {
        StoreError::Invalid(format!("document {key} has more than {} {kind:?} spans (core.rerail's span ids)", kind.capacity()))
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

pub fn families() -> &'static [&'static dyn Families] {
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
    /// `RERAIL_NODE_IDS=preorder`, its preorder index. Rules that order nodes read it (`lowering_tail::OrderNodes`).
    Order,
    /// `next_sibling(a, b)`: `b` is the numbered direct child of `a`'s parent after `a`, those left out skipped. A
    /// row a node after a sibling (`Item::Sibling`), as `field`'s are its fielded nodes.
    Sibling,
    /// `core.rerail`'s place of a node or span (`loc`): `line(n, l)`, `end_line(n, l)`, `col(n, c)`, `docof(n, d)`,
    /// read from the document's lines (`core_rr::Lines`), rows laid out over `Projected::locs`. A batch that makes
    /// one of them as rows (`Extra`, whose spans have places too) binds those rows instead (`Batch::made`).
    Loc(Place),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Place {
    Line,
    EndLine,
    Col,
    Docof,
}

/// The relations of core.rerail a batch makes only for a program that reads them (its `reads`), T's exporter having
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
/// In `core.rerail`'s shape (`Shape::Core`) a relation made here is the batch's, in place of `core_rr`'s rows or the `Loc`
/// view of that name (`Batch::build`), and its node rows cover that shape's anonymous tokens too (`Locs`); `comment`
/// is `core_rr`'s there (comments are nodes), so it is not made here.
///
///
/// And each `matchall_<k>(sp, m)` read (`preparation::text::TextRelations`, core.rerail's `match_all`): `m` each match of
/// regex `k` in span `sp`'s text, leftmost-first and non-overlapping (Rust's `find_iter`, as W's `regex` reads a
/// file), over every span above. A match is a span (`MATCH`) with rows in `text` (what it matched), `line`,
/// `end_line` and `col`, as W's finding of that regex is placed: in a line span, that line and the byte column in it
/// (W's `lines` regexes); in the source, the lines and column of the tape's line table (W's content regex).
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
    src_line_lf: bool,
    tape_line: bool,
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
    value: bool,
    /// `rr_value_lit`: the literal values of documents W reads no values of, where `value` is answered on request
    /// (decision 0174): kept eager, as they are few.
    value_lit: bool,
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
    /// The document's file's key (`Projected::file`), the `docof` of its spans: set per document.
    file: i64,
    values: Vec<[i64; 2]>,
    value_lits: Vec<[i64; 2]>,
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
            value_lit: read("rr_value_lit"),
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
        let f = &*p.facts;
        let ends = p.extra.as_ref().map_or(0..0, |x| p.len() + x.ends_from()..p.len() + x.ends_from() + x.ends);
        let guest = p.extra.as_deref().filter(|x| x.host.is_some());
        if self.line_rows() || self.end_line || self.col || self.node_text {
            for (_, id, n) in p.node_ids() {
                let (start, end) = (f.start(id) as usize, f.end(id) as usize);
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
            for (local, _, n) in p.token_ids() {
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
            // Each node's end point (`end_point`, where a program reads it): placed where the node ends.
            for local in ends.clone() {
                let n = p.global(local);
                let at = p.span(local)[0];
                let (line, col) = match guest {
                    Some(x) => {
                        let (first, _, col) = x.place([at, at]);
                        (i64::from(first), i64::from(col))
                    }
                    None => {
                        let l = f.lines.line(at as usize);
                        (l as i64 + 1, (at as usize - f.lines.start(l)) as i64)
                    }
                };
                if self.line_rows() {
                    self.lines.push([n, line]);
                }
                if self.end_line {
                    self.end_lines.push([n, line]);
                }
                if self.col {
                    self.cols.push([n, col]);
                }
            }
        }
        if self.docof {
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
                let local = p.local_of_node(ordinal as u32);
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
    /// `splitlines`: its lines split as Python's `str.splitlines()` does; `rx`: `rx_span`'s patterns, the spans each
    /// runs over and its block program, and `ends` (by pattern) the bytes of the document's source a match can end
    /// after, where the batch ran the pattern's block program (decision 0081 §3).
    #[allow(clippy::too_many_arguments)]
    fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, splitlines: bool, rx: &[core_rr::SpanPattern<'_>], ends: &[Option<Vec<u64>>], text: &mut Vec<[i64; 2]>, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<()> {
        let f = &*p.facts;
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
                let c = match p.local_of_node(id as u32) {
                    NONE => {
                        k += 1;
                        let c = span(ExtraSpan::Comment, k)?;
                        let (start, end) = (f.start(id) as usize, f.end(id) as usize);
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
                Some(dialect) => p
                    .node_ids()
                    .filter(|&(_, x, _)| test(dialect.name(f, x)))
                    .map(|(_, x, n)| (n, f.start(x) as usize..f.end(x) as usize, None))
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
                    .map(|((_, x, _), (_, _, n))| (n, f.start(x) as usize..f.end(x) as usize, None))
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
                    SpanIn::ChildOf(k) => (children_of(k.as_str()), true),
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
    fn relations(self, atom: &dyn Fn(i64) -> i64, relation: Make) -> (Vec<(&'static str, Relation)>, Vec<(String, Relation)>) {
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
        if self.value_lit {
            out.push(("rr_value_lit", relation(&["n", "v"], &mut self.value_lits.iter().map(|&[n, v]| vec![n, atom(v)]))));
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
/// lowercased). In `core.rerail`'s shape only `intval` and T's `op`: `core_rr` makes the others as `core.rerail` states them.
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
    /// The bytes of its rows so far (a copy's size, decision 0353's row 7).
    fn bytes(&self) -> usize {
        std::mem::size_of_val(self.text.as_slice()) + std::mem::size_of_val(self.intval.as_slice()) + std::mem::size_of_val(self.op.as_slice()) + std::mem::size_of_val(self.name.as_slice()) + std::mem::size_of_val(self.strs.as_slice()) + std::mem::size_of_val(self.strprefix.as_slice())
    }
}

impl NodeRows {
    /// Document `p`'s rows (`dialect` its grammar's kinds; `core`: in `core.rerail`'s shape).
    fn document(&mut self, p: &Projected, dialect: &Dialect, core: bool, string: &mut dyn FnMut(&str) -> usize) {
        let f = &*p.facts;
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
    /// Projects each document and interns the batch's strings; the documents' keys are distinct.
    /// Returns the batch and its small relations (`doc`, `pkg`, `syntax_error`, `text`, `intval`, `op`), and those
    /// of core.rerail's the program `reads` that T had not (`Extra`), the `match_all` relations among them (of `matches`)
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
        let mut grown = Growth::default();
        let (batch, relations, matched, _) = Self::add(&mut grown, store, documents, projection, matches, markers, metal, false)?;
        Ok((batch, relations, matched, (grown.strings, grown.atoms)))
    }

    /// The batch `grown` holds with `documents` added (decision 0431 §6): they are projected into its block, and the
    /// rows made are theirs alone, past its keys; the batch made is every document's, the earlier ones' projections
    /// shared. Its strings from the one given are the new ones.
    fn add(
        grown: &mut Growth,
        store: &ContentStore,
        documents: Vec<Document>,
        projection: &Projection,
        matches: &[Arc<regex::Regex>],
        markers: &[String],
        metal: bool,
        alone: bool,
    ) -> StoreResult<(Arc<Self>, Vec<(&'static str, Relation)>, Vec<(String, Relation)>, usize)> {
        let shape = projection.shape;
        let core = shape == Shape::Core;
        // What the program reads, for the relations made only for a program that reads them (`Extra` and those beside
        // it); a projection that names none (T's, `Projection::core`'s) reads none of them.
        let none = BTreeSet::new();
        let reads = projection.reads.as_ref().unwrap_or(&none);
        // A document the batch holds is a tape value given more places (decision 0431 §6): the rows of those alone.
        let (held, mut documents): (Vec<Document>, Vec<Document>) = documents.into_iter().partition(|d| grown.keys.binary_search(&d.key).is_ok());
        documents.sort_by_key(|d| d.key);
        if documents.windows(2).any(|w| w[0].key == w[1].key) {
            return Err(StoreError::Invalid("a batch holds a document key twice".into()));
        }
        let strings_from = grown.strings.len();
        let Growth { strings, at, dialects, .. } = grown;
        // Every string of every node passes here: Fx, not SipHash (a third of the projection's time with SipHash).
        let mut string = |s: &str| -> usize {
            if let Some(&i) = at.get(s) {
                return i;
            }
            strings.push(s.to_owned());
            at.insert(s.to_owned(), strings.len() - 1);
            strings.len() - 1
        };
        // Rows with string indexes; the indexes become atoms once the batch's strings are interned.
        let (mut doc, mut pkg, mut unpackaged) = (Vec::new(), Vec::new(), Vec::new());
        // `path_part(d, s)`: each directory of a document's path and its file name; `file_name(d, s)`: the last.
        let (mut path_part, mut file_name): (Vec<[i64; 2]>, Vec<[i64; 2]>) = (Vec::new(), Vec::new());
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
        // Kept by a batch that grows (`ProducerRun::relations`).
        let produced = grown.produced.get_or_insert_with(|| producers().iter().map(|producer| (crate::code_rule_dsl::language::tape_tag(producer.dialect()), producer.start(reads, markers))).collect());
        let python = crate::code_rule_dsl::language::tape_tag(crate::code_rule_dsl::language::GrammarDialect::Python);
        // `core.rerail`'s (decision 0046).
        let mut c = core_rr::CoreRows::default();
        // Node identity/ancestry and line views do not require scalar rows or text interning.
        let scalar_nodes=!core || ["text","name","str","strprefix","value","value_type","static_value","comment","spread","op","op_at","op_t","intval","str_part","str_part_start","str_part_word"]
            .into_iter().any(|r|projection.wants(r));
        let wants_any = |rels: &[&str]| rels.iter().any(|r| projection.wants(r));
        c.skip = core_rr::Skip {
            ops: !wants_any(&["op", "op_at"]),
            comment: !projection.wants("comment"),
            spread: !projection.wants("spread"),
            name: !projection.wants("name"),
            literal: !wants_any(&["str", "strprefix", "value", "static_value", "value_type", "rr_value_lit", "text"]),
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
        c.long_text = projection.long_text.clone();
        c.long_text_any = projection.long_text_any;
        c.text_nodes = projection.text_nodes.clone();
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
        let mut docs: Vec<Projected> = Vec::with_capacity(documents.len());
        // The batch's block: what the projections can take, the chunk's copies of the facts' arrays (4 bytes a node for
        // each of `parent`, `start`, `end`, `kind`, `core.rerail`'s places; `field`'s 2), its tables, and the starts of the
        // axes the device solves (4 bytes a node; their rows are relations of their own, decision 0353 step 5). The
        // block reserves address space, not memory.
        // A batch that grows (decision 0431 §6) projects its later documents into the same block: the gigabyte past
        // its first documents' needs is address space for them.
        let block = Arc::clone(grown.block.get_or_insert_with(|| cells::block((documents.iter().map(|d| Projected::bytes(&d.facts) + 22 * d.facts.len() + 4096).sum::<usize>() + (1 << 20) + (1 << 30)).next_power_of_two())));
        // Decision 0055: each document's role, root and tape; each guest's place, and its nodes' host nodes. A guest's
        // key is past every file's (`corpus::guests`), so its host is projected first and its lines are known.
        let mut g = GuestRows::default();
        let lines_of = &mut grown.lines_of;
        // A file's lines built ahead of it, for a guest projected before it (below): taken by the file's projection.
        let mut ahead: HashMap<u32, Arc<core_rr::Lines>> = HashMap::new();
        // Each guest's file: its host's, through guests of guests. Files' keys are below every guest's, so a file
        // is projected (and its lines known) before its guests; a guest's guests may come before it.
        grown.host.extend(documents.iter().filter_map(|d| d.guest.as_ref().map(|g| (d.key, g.host))));
        let host = &grown.host;
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
                Some(dialects.get_or_new(tag)?)
            };
            // `core.rerail`: a document that does not parse is withheld from the policy (no facts) but its `doc`, path
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
            // A guest's lines are its file's. A file that a run reads for its programs (decision 0470 step 2) has a hidden
            // key, past its programs', so a program may come first: its file's lines are built then, from the file's
            // document, and the file's projection takes them.
            let placed = match guest {
                Some(g) => {
                    let file = file_of(g.host);
                    let lines = match lines_of.get(&file) {
                        Some(lines) => Arc::clone(lines),
                        None => match documents.iter().find(|h| h.key == file && h.guest.is_none()) {
                            Some(h) => {
                                let tag = crate::code_rule_dsl::facts::core::facts_tag(&h.facts);
                                let python = tag != TEXT && Family::of(crate::code_rule_dsl::language::from_tape_tag(tag).and_then(crate::code_rule_dsl::language::label)) == Family::Python;
                                let lines = Arc::new(core_rr::Lines::shared(Arc::clone(&h.facts.source), python, &block));
                                ahead.insert(file, Arc::clone(&lines));
                                lines_of.insert(file, Arc::clone(&lines));
                                lines
                            }
                            None => Arc::new(core_rr::Lines::shared(Arc::clone(&d.facts.source), python_lines, &block)),
                        },
                    };
                    Some((g, lines))
                }
                None => None,
            };
            let known = if guest.is_none() && core { ahead.remove(&d.key) } else { None };
            let new_span = crate::code_rule_dsl::logic::profile::span(|| "projected".into());
            let mut p = Projected::narrowed(d.key, Arc::clone(&d.facts), dialect, shape, withheld, placed, known, projection.node_anchors.as_ref().filter(|_| core), &block);
            drop(new_span);
            // A program that reads `end_point` has each node's end point among the positions, after the lines.
            let nodes = p.len();
            if core && projection.wants("end_point")
                && let Some(x) = p.extra.as_mut()
            {
                x.ends = nodes;
            }
            p.module = crate::code_rule_dsl::module_path::ModulePath::dotted(d.module.as_deref().unwrap_or(&module(&d.path)));
            // Its positions (tokens, text, lines) in the top slots (`granules::span_low`): past them ids would clash.
            if p.defs.is_some() && u64::from(p.locs() - p.len()) > granules::POSITIONS {
                return Err(StoreError::Invalid(format!("{} has more than {} positions (tokens and lines: core.rerail's span ids)", d.path, granules::POSITIONS)));
            }
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
            // core.rerail: a module for Python and Rust files, "" for the others; `pkg` for Python only. T's has both
            // for every file.
            // A tape value (decision 0431) is no file's: no `doc` row, no module or package.
            let tape_value = d.guest.as_deref().is_some_and(|g| matches!(g.made, Made::Tape { .. }));
            let py_file = !tape_value && (d.path.ends_with(".py") || d.path.ends_with(".pyi"));
            let dotted = py_file || d.path.ends_with(".rs");
            let named = d.module.as_deref();
            let module = match named {
                Some(m) => m.to_owned(),
                None if core && !dotted => String::new(),
                None => module(&d.path),
            };
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
            if d.facts.syntax_errors() {
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
                    rows.document(&p, dialect, core, &mut string);
                }
                if core_nodes || wants_long_text_nodes {
                    let kept = (c.skip.literal_rows, c.skip.literal_value);
                    if value_lit_only && dialect.values {
                        (c.skip.literal_rows, c.skip.literal_value) = (true, true);
                    }
                    let visits = c.visits(wants_text, wants_parts, wants_part_starts, wants_words);
                    for (local, id, n) in p.node_ids() {
                        if wants_long_text_nodes && (p.facts.end(id) - p.facts.start(id)) as usize > core_rr::TEXT_INLINE {
                            long_text_nodes.push(n);
                        }
                        if core_nodes && c.visited(visits, &p, dialect, id) {
                            visited += 1;
                            c.node(&p, dialect, local, id, n, &mut string, wants_text, wants_parts, wants_part_starts, wants_words);
                        }
                    }
                    (c.skip.literal_rows, c.skip.literal_value) = kept;
                }
                if let Some((line, msg)) = &error {
                    let line = p.extra.as_ref().map_or(*line, |x| x.file_line(*line));
                    c.withheld(&p, &(line, msg.clone()), &mut string);
                }
            }
            copying.done((rows.bytes() + c.bytes()).saturating_sub(rows_before));
            drop(nodes_span);
            let document_span = crate::code_rule_dsl::logic::profile::span(|| "document rows".into());
            // A text-only document has core.rerail's text rows too (its lines, `split_line`), and no tokens.
            if core && (!withheld || guest.is_none()) {
                let s = crate::code_rule_dsl::logic::profile::span(|| "core document rows".into());
                c.document(&p, dialect, &mut string, wants_text, wants_tokens && !withheld, wants_split_lines, wants_lf_lines, wants_source_text, wants_line_text, guest.is_none(), projection);
                drop(s);
                if let Some(dialect) = dialect
                    && wants_digests
                    && !withheld
                {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "digests".into());
                    c.digests(&p, dialect, &mut string, wants_trees, wants_units);
                }
            }
            if d.guest.is_none() {
                // A text document's lines split as W's `lines` regexes split any file's. A withheld document has no
                // `rx_span` matches.
                let rx = if withheld { &[][..] } else { &rx_spans[..] };
                {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "extra document rows".into());
                    extra.document(&p, d, dialect, python == Some(tag) || tag == TEXT, rx, &span_ends[j], &mut rows.text, &mut string)?;
                }
                let _s = crate::code_rule_dsl::logic::profile::span(|| "tree document rows".into());
                tree.document(&p, dialect, python == Some(tag), &mut string);
            } else {
                // A guest's nodes have their places in its file (`Locs::place`); W's values, f-string trees and CPython's
                // verdicts are a file's, read off its tape; an embedded program keeps its own (decision 0222).
                if d.tape.is_some() {
                    extra.values(&p, d, dialect, &mut string)?;
                }
                extra.nodes(&p, &mut string);
                if let Some(dialect) = dialect {
                    tree.operators(&p, dialect, &mut string);
                }
            }
            // A guest's are read only by a producer of guests' rows (string annotations); the others' are its file's.
            for (own, run) in produced.iter_mut() {
                if d.guest.is_none() || run.reads_guests() {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| format!("produced rows: {}", run.name()));
                    run.document(&p, d, dialect, *own == Some(tag), &mut string)?;
                }
            }
            drop(document_span);
            docs.push(p);
        }
        // The guests' rows, once every document is projected: a guest's host may be a guest after it. Either shape:
        // a program that declares its base relations itself (T's) reads guests as core.rerail's do.
        // The batch's documents in key order (its lookups search them by key and key range): the new ones merged
        // among the projections it holds, shared, with their tapes for W's values. Guests' keys (0391's) do not
        // ascend with the rounds.
        let added: Vec<Arc<Projected>> = docs.into_iter().map(Arc::new).collect();
        let added_valued: Vec<Option<Arc<Valued>>> = if projection.derived.contains(VALUE_REQ) {
            documents
                .iter()
                .zip(&added)
                .map(|(d, p)| d.tape.clone().filter(|_| dialects.get(p.tag).is_some_and(|x| x.values)).map(|(tape, file)| Arc::new(Valued { tape, file, values: OnceLock::new() })))
                .collect()
        } else {
            Vec::new()
        };
        {
            let (held_docs, held_valued) = (std::mem::take(&mut grown.docs), std::mem::take(&mut grown.valued));
            let valued = !held_valued.is_empty() || !added_valued.is_empty();
            let (mut i, mut j) = (0, 0);
            while i < held_docs.len() || j < added.len() {
                if j == added.len() || (i < held_docs.len() && held_docs[i].key < added[j].key) {
                    grown.docs.push(Arc::clone(&held_docs[i]));
                    if valued {
                        grown.valued.push(held_valued.get(i).cloned().flatten());
                    }
                    i += 1;
                } else {
                    grown.docs.push(Arc::clone(&added[j]));
                    if valued {
                        grown.valued.push(added_valued.get(j).cloned().flatten());
                    }
                    j += 1;
                }
            }
            grown.keys = grown.docs.iter().map(|p| p.key).collect();
        }
        // A value admitted inside the fixpoint (decision 0431 §8) is a batch of its own: its views and relations are a
        // part of the chunk's (`values::Layered`), its hosts found among every document the growth holds.
        let held_all = grown.docs.clone();
        let (docs, all): (Vec<Arc<Projected>>, &[Arc<Projected>]) = if alone { (added.clone(), &held_all) } else { (grown.docs.clone(), &held_all) };
        let docs = &docs;
        for ((p, d), &withheld) in added.iter().zip(&documents).zip(&withheld_at) {
            g.document(p, d, d.guest.as_deref(), withheld, all, [projection.wants("host_node"), projection.wants("unparse_of"), projection.wants("guest_of"), projection.wants("guest_place")], &mut string);
            if let Some(Made::Tape { places }) = d.guest.as_deref().map(|g| &g.made) {
                grown.places.insert(d.key, (places.len(), withheld));
            }
        }
        for d in &held {
            let (Some(guest), Ok(at)) = (d.guest.as_deref(), grown.keys.binary_search(&d.key)) else { continue };
            let (Made::Tape { places }, Some((had, withheld))) = (&guest.made, grown.places.get_mut(&d.key)) else {
                return Err(StoreError::Invalid(format!("{}: a document the batch holds, given again, is not a tape value", d.path)));
            };
            if places.len() > *had {
                g.places_added(&all[at], d, &guest.grammar, &places[*had..], *withheld, all, [projection.wants("host_node"), projection.wants("guest_of"), projection.wants("guest_place")], &mut string);
                *had = places.len();
            }
        }
        // Each lazy family's strings, where the batch has documents of its dialect.
        let vocabulary: Vec<Option<(u8, Vec<i64>)>> = families()
            .iter()
            .map(|f| {
                crate::code_rule_dsl::language::tape_tag(f.dialect())
                    .filter(|&tag| dialects.get(tag).is_some())
                    .map(|tag| (tag, f.strings().iter().map(|name| string(name) as i64).collect()))
            })
            .collect();
        crate::code_rule_dsl::logic::profile::decision("input payload", || {
            let identities: usize = docs.iter().map(|p| p.ranks().bytes() + p.idx.get().map_or(0, |v| v.len() * 4)).sum();
            serde_json::json!({"documents": docs.len(), "coreColumnBytes": docs.iter().map(|p| p.facts.bytes()).sum::<usize>(), "identityCapacityBytes": identities}).to_string()
        });
        crate::code_rule_dsl::logic::profile::decision("projection demand",||serde_json::json!({"coreNodes":if core_nodes {docs.iter().map(|p| p.len() as usize).sum::<usize>()}else{0},"scalarNodes":visited,"nodes":docs.iter().map(|p| p.len() as usize).sum::<usize>(),"strings":strings.len(),"reads":projection.reads}).to_string());
        let intern = crate::code_rule_dsl::logic::profile::span(|| "intern atoms".into());
        crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, strings.len() as u64);
        let new_atoms = store.intern_atoms(&strings[strings_from..].iter().map(String::as_str).collect::<Vec<_>>())?;
        grown.atoms.extend(new_atoms);
        drop(intern);
        let atoms = &grown.atoms;
        let atom = |i: i64| i64::from(atoms[i as usize]);
        let bound: Vec<Option<Box<dyn BoundFamilies>>> = families()
            .iter()
            .zip(vocabulary)
            .map(|(f, v)| v.map(|(tag, strings)| f.bind(tag, &dialects[tag].catalog, &strings.into_iter().map(atom).collect::<Vec<_>>(), docs.len())))
            .collect();
        let _relations_span = crate::code_rule_dsl::logic::profile::span(|| "relations".into());
        // A value alone's rows are staged: they are written once, past the chunk's relations' rows (`Grower::admit`).
        let relation: Make = if alone { Relation::staged } else { Relation::from_rows };
        let pairs = |columns: &[&str], rows: &[[i64; 2]], sym: [bool; 2]| {
            relation(columns, &mut rows.iter().map(|&[a, b]| vec![if sym[0] { atom(a) } else { a }, if sym[1] { atom(b) } else { b }]))
        };
        let mut relations = vec![
            (LONG_TEXT_NODES, relation(&["n"], &mut long_text_nodes.iter().map(|&n| vec![n]))),
            ("doc", relation(&["d", "path", "module"], &mut doc.iter().map(|&[d, p, m]| vec![d, atom(p), atom(m)]))),
            ("pkg", pairs(&["d", "p"], &pkg, [false, true])),
            ("unpackaged", relation(&["d"], &mut unpackaged.iter().map(|&d| vec![d]))),
            ("path_part", pairs(&["d", "s"], &path_part, [false, true])),
            ("file_name", pairs(&["d", "s"], &file_name, [false, true])),
            ("intval", pairs(&["n", "v"], &rows.intval, [false, false])),
            ("op_t", pairs(&["n", "s"], &rows.op, [false, true])),
        ];
        relations.extend(g.relations(&atom, relation));
        if core {
            extra.literal_values(c.values_of(&unvalued));
        }
        let (mut read, matched) = extra.relations(&atom, relation);
        read.extend(tree.relations(&atom, relation));
        for (_, run) in produced.iter_mut() {
            read.extend(run.relations(&atom));
        }
        // The relations made for what the program reads are the batch's: in `core.rerail`'s shape they stand in for
        // `core_rr`'s rows and the views of the same name (W's values, the spans and places `match_all` reads,
        // `op_at`'s operators, `visit_index`'s CPython order).
        let made: BTreeSet<&'static str> = read.iter().map(|(name, _)| *name).collect();
        if core {
            // `core_rr`'s `text` and the text of `Extra`'s spans.
            c.extend_text(&rows.text);
            relations.extend(c.relations(&atom).into_iter().filter(|(name, _)| !made.contains(name)));
        } else {
            // T's `syntax_error(d)`, or core.rerail's `syntax_error(d, l, msg)` (CPython's verdict) where the program
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
        let nodes = offsets(&|p| p.len() as usize);
        let children = offsets(&|p| (p.len() as usize).saturating_sub(1));
        let fields = offsets(&|p| if projection.wants("field") { p.fielded() as usize } else { 0 });
        let siblings = offsets(&|p| if projection.wants("next_sibling") { p.after_siblings() as usize } else { 0 });
        let locs = OnceLock::new();
        // W's values on request (decision 0174): the tapes of the documents W reads values of.
        debug_assert!(grown.keys.is_sorted(), "a batch's documents ascend by key");
        grown.made.extend(made.iter().copied());
        let batch = Arc::new(Batch {
            docs: docs.clone(),
            keys: if alone { docs.iter().map(|p| p.key).collect() } else { grown.keys.clone() },
            programs: {
                let mut programs: Vec<(u32, usize)> = docs.iter().enumerate().filter(|(_, p)| p.file != p.key).map(|(d, p)| (p.file, d)).collect();
                programs.sort_unstable();
                programs
            },
            valued: if alone { added_valued.clone() } else { grown.valued.clone() },
            dialects: dialects.clone(),
            nodes,
            children,
            fields,
            siblings,
            locs,
            kinds: OnceLock::new(),
            shape,
            families: bound,
            made: grown.made.clone(),
            block,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            chunk: OnceLock::new(),
        });
        #[cfg(any(test, feature = "test-hooks"))]
        BATCHES.lock().push((batch.docs.iter().map(|p| p.key).collect(), Arc::downgrade(&batch)));
        Ok((batch, relations, matched, strings_from))
    }

    /// The batch's chunk block (decision 0289), made on first use.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub fn chunk(&self) -> &device_chunk::Chunk {
        self.chunk.get_or_init(|| device_chunk::build(self))
    }
    fn offsets(&self, view: View) -> &[usize] {
        match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order => &self.nodes,
            View::Child => &self.children,
            View::Field => &self.fields,
            View::Sibling => &self.siblings,
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
    /// The documents of file `file`'s programs (decision 0431 §4).
    fn programs_of(&self, file: u32) -> impl Iterator<Item = usize> + '_ {
        let at = self.programs.partition_point(|&(f, _)| f < file);
        self.programs[at..].iter().take_while(move |&&(f, _)| f == file).map(|&(_, d)| d)
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
        let i = self.keys.binary_search(&d).ok()?;
        DOC.with(|h| h.set((key, i)));
        Some(i)
    }
    /// The document index and node of id `n`.
    pub fn node(&self, n: i64) -> Option<(usize, u32)> {
        let doc = self.doc(n >> 32)?;
        let local = self.docs[doc].local_of((n & 0xffff_ffff) as u32)?;
        (n >= 0 && local < self.docs[doc].len()).then_some((doc, local))
    }
    /// The nodes whose `kind` (the `node` view's column 2) is `kind`, in row order: `(document, local id, tape node)`.
    fn by_kind(&self, kind: i64) -> &[(u32, u32, u32)] {
        let k = self.kinds.get_or_init(|| KindRows::new(self));
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
            View::Field | View::Sibling => self.row_of_node(view, doc, local, self.docs[doc].node(local)),
            _ => self.row_of_node(view, doc, local, NONE),
        }
    }
    /// `row`, the node's tape node `x` known (read only by the `field` view: its rows are the fielded nodes).
    fn row_of_node(&self, view: View, doc: usize, local: u32, x: u32) -> Option<usize> {
        let within = match view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order | View::Loc(_) => Some(local as usize),
            View::Child => (local > 0).then(|| local as usize - 1),
            View::Field => self.docs[doc].ranks().local(Item::Field, x).map(|r| r as usize),
            View::Sibling => self.docs[doc].ranks().local(Item::Sibling, x).map(|r| r as usize),
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
                let ids: Box<dyn Iterator<Item = (u32, i64)>> = if matches!(view, View::Loc(_)) { Box::new(p.loc_ids()) } else { Box::new(p.node_ids().map(|(l, _, n)| (l, n))) };
                let mut locals: Vec<u32> = ids.filter(|&(_, n)| (lo..=hi).contains(&n)).map(|(l, _)| l).collect();
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
            View::Loc(Place::Line | Place::EndLine) => &["n", "l"],
            View::Loc(Place::Col) => &["n", "c"],
            View::Loc(Place::Docof) => &["n", "d"],
        }
    }
    /// The column holding the row's own node, which ranges and direct lookups address.
    fn own(&self) -> usize {
        match self.view {
            View::Node | View::Child | View::Sibling => 1,
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
            View::Node | View::Line | View::Core | View::Depth | View::Order | View::Sibling | View::Loc(_) => None,
        }
    }
}
/// `node`'s column `column` over `batch`, counted without reading its rows (decision 0329): its distinct values, the
/// sum of their counts' squares, and each value's count. A kind's count is its postings' (`of_kind`), a document's its
/// nodes; `n` is the key (each value once, no counts). `None` for another column.
pub fn node_column_counts(batch: &Batch, column: usize) -> Option<(i64, i64, Option<Vec<(i64, i64)>>)> {
    let squares = |counts: &[(i64, i64)]| counts.iter().map(|&(_, n)| n * n).sum::<i64>();
    match column {
        0 => {
            let counts: Vec<(i64, i64)> = batch.docs.iter().map(|p| (i64::from(p.key), i64::from(p.len()))).filter(|&(_, n)| n > 0).collect();
            Some((counts.len() as i64, squares(&counts), Some(counts)))
        }
        1 => {
            let n: i64 = batch.docs.iter().map(|p| i64::from(p.len())).sum();
            Some((n, n, None))
        }
        2 => {
            // The batch's nodes (a document's tape holds tokens and nodes it leaves out), each node's code read once into
            // its dialect's counts by code (`of_kind`, a scan of the tape a kind, took 12 s of a cold PostHog run's
            // guests), each code read as the column reads it (`Dialect::kind`: its canonical code).
            let mut by_code: std::collections::BTreeMap<u8, Vec<i64>> = Default::default();
            for p in &batch.docs {
                // A text-only document has no dialect and no nodes.
                let Some(dialect) = batch.dialects.get(p.tag) else { continue };
                let counts = by_code.entry(p.tag).or_default();
                for (_, id) in p.nodes() {
                    let code = usize::from(dialect.code(&p.facts, id));
                    if counts.len() <= code {
                        counts.resize(code + 1, 0);
                    }
                    counts[code] += 1;
                }
            }
            let mut by: HashMap<i64, i64> = HashMap::new();
            for (tag, counts) in by_code {
                let dialect = &batch.dialects[tag];
                for (code, &n) in counts.iter().enumerate().filter(|(_, n)| **n > 0) {
                    let code = code as u16;
                    *by.entry(i64::from(kind_atom(tag, dialect.canonical.get(usize::from(code)).copied().unwrap_or(code)))).or_default() += n;
                }
            }
            let mut counts: Vec<(i64, i64)> = by.into_iter().collect();
            counts.sort_unstable();
            Some((counts.len() as i64, squares(&counts), Some(counts)))
        }
        _ => None,
    }
}

impl Source for Viewed {
    fn python_batch(&self)->Option<Arc<Batch>> {Some(Arc::clone(&self.batch))}
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn device_view(&self) -> Option<crate::code_rule_dsl::facts::vtab::DeviceView<'_>> {
        let view = device_chunk::device_view(self.view)?;
        let chunk = self.batch.chunk();
        // The views the views' reads answer are read in place where every tape (and for `line`, every document's
        // lines) is on the device.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let loc = matches!(self.view, View::Loc(_));
        // The directory is of the process's device's addresses, which the stages read it by (decision 0440 step C):
        // none where its buffers have none (`views_in_place`).
        let on_device = rerail_accel::gpu::join::device().is_ok();
        let in_place = (on_device && (loc || matches!(view, device_chunk::VIEW_NODE | device_chunk::VIEW_CHILD | device_chunk::VIEW_FIELD | device_chunk::VIEW_CORE | device_chunk::VIEW_LINE)))
            .then(|| device_chunk::views_in_place(&self.batch, chunk))
            .flatten()
            .filter(|v| (view != device_chunk::VIEW_LINE || v.lines) && (!loc || v.locs))
            .map(|v| {
                let mut in_place = crate::code_rule_dsl::facts::vtab::ViewInPlace { dir: v.dir, kind: device_chunk::read_kind(view), buffers: v.buffers.clone(), cache: v.cache.clone(), failed: 0, keep: Arc::clone(&self.batch) as Arc<dyn std::any::Any + Send + Sync> };
                in_place.failed = in_place.cache_failed();
                in_place
            });
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let in_place: Option<()> = None;
        if matches!(self.view, View::Loc(_)) && in_place.is_none() && !device_chunk::fill_locs(&self.batch, chunk) {
            return None;
        }
        if matches!(self.view, View::Line) && in_place.is_none() && !device_chunk::fill_lines(&self.batch, chunk) {
            return None;
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        let home = in_place.as_ref().and_then(|_| self.home.get_or_init(|| crate::code_rule_dsl::facts::rows::Part::for_device(0, self.len()).ok()).clone());
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        let home = None;
        Some(crate::code_rule_dsl::facts::vtab::DeviceView {
            block: &chunk.block,
            root: chunk.root,
            view,
            rows: self.len(),
            #[cfg(any(target_os = "macos", target_os = "linux"))]
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
    fn lookup_read(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        self.read_matches(column, value, columns, each)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.lookup_docs(column, values, None)
    }
    fn keys_rows(&self, column: usize) -> bool {
        column == self.own()
    }
    /// `lookup_docs`' own column, for one value.
    fn key_row(&self, column: usize, value: i64) -> Option<usize> {
        debug_assert_eq!(column, self.own());
        let b = &*self.batch;
        let found = if matches!(self.view, View::Loc(_)) { b.loc(value) } else { b.node(value) };
        found.and_then(|(doc, local)| b.row(self.view, doc, local))
    }
    /// A field's children by parent and field (`field($a, #f, $t)`, `$a` bound): each child's field read off the
    /// tape as the children are walked, not its row's values found again. By child and field (`field(_, #f, $n)`,
    /// `$n` bound: whether `$n` is its parent's `#f`), the child's own field read off the tape: its one row or
    /// none. Found by child and filtered, `nameref`'s `!field(_, #f65541, $n)` took 300 ns a probe, 25 M probes of a
    /// cold PostHog run.
    fn lookup_eq(&self, column: usize, value: i64, eqs: &[(usize, i64)]) -> Option<Vec<usize>> {
        let &[(1, f)] = eqs else { return None };
        if !matches!(self.view, View::Field) {
            return None;
        }
        let b = &*self.batch;
        if column == 2 {
            let Some((doc, local)) = b.node(value) else { return Some(Vec::new()) };
            let p = &b.docs[doc];
            let x = p.node(local);
            let own = i64::from(field_atom(p.tag, p.facts.field(x))) == f;
            return Some(if own { b.row_of_node(View::Field, doc, local, x).into_iter().collect() } else { Vec::new() });
        }
        if column != 0 {
            return None;
        }
        let mut rows = Vec::new();
        if let Some((doc, local)) = b.node(value) {
            let p = &b.docs[doc];
            p.each_child_node(local, |c, x| {
                if i64::from(field_atom(p.tag, p.facts.field(x))) == f {
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
    /// Document by document, with no search for each row's document. (A merge had left this in `impl Viewed`,
    /// where the trait's per-row scan, a document search for each row, shadowed it.)
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        let offsets = self.batch.offsets(self.view);
        let mut out = Vec::new();
        for (doc, p) in self.batch.docs.iter().enumerate() {
            let base = offsets[doc];
            self.each_row(p, offsets[doc + 1] - base, eqs.iter().map(|&(c, _)| c), |i, value| {
                if eqs.iter().all(|&(c, v)| value(c) == v) {
                    out.push((base + i) as u32);
                }
            });
        }
        out
    }
    fn rows_of(&self, columns: &[usize], out: &mut Vec<i64>) {
        let offsets = self.batch.offsets(self.view);
        out.reserve(self.len() * columns.len());
        for (doc, p) in self.batch.docs.iter().enumerate() {
            self.each_row(p, offsets[doc + 1] - offsets[doc], columns.iter().copied(), |_, value| out.extend(columns.iter().map(|&c| value(c))));
        }
    }
    fn for_rows(&self, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) {
        let offsets = self.batch.offsets(self.view);
        let mut row = vec![0i64; columns.len()];
        for (doc, p) in self.batch.docs.iter().enumerate() {
            let base = offsets[doc];
            self.each_row(p, offsets[doc + 1] - base, columns.iter().copied(), |i, value| {
                for (o, &c) in row.iter_mut().zip(columns) {
                    *o = value(c);
                }
                each(base + i, &row);
            });
        }
    }
    fn for_rows_in(&self, range: std::ops::Range<usize>, columns: &[usize], each: &mut dyn FnMut(usize, &[i64]) -> bool) {
        let offsets = self.batch.offsets(self.view);
        let mut row = vec![0i64; columns.len()];
        let mut go = true;
        // The documents the range holds: from the one holding its first row.
        let first = offsets.partition_point(|&o| o <= range.start).saturating_sub(1);
        for (doc, p) in self.batch.docs.iter().enumerate().skip(first) {
            let base = offsets[doc];
            if !go || base >= range.end {
                break;
            }
            self.each_row(p, offsets[doc + 1] - base, columns.iter().copied(), |i, value| {
                if !go || !range.contains(&(base + i)) {
                    return;
                }
                for (o, &c) in row.iter_mut().zip(columns) {
                    *o = value(c);
                }
                go = each(base + i, &row);
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
        let mut row = vec![0i64; columns.len()];
        let mut go = true;
        for (doc, p) in self.batch.docs.iter().enumerate() {
            if !go {
                break;
            }
            if !docs.contains(&i64::from(p.key)) {
                continue;
            }
            let base = offsets[doc];
            self.each_row(p, offsets[doc + 1] - base, columns.iter().copied().chain([column]), |i, value| {
                if !go || !keys.contains(&value(column)) {
                    return;
                }
                for (o, &c) in row.iter_mut().zip(columns) {
                    *o = value(c);
                }
                go = each(base + i, &row);
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
        let (local, x) = self.local_of_row(p, i);
        let node = std::cell::OnceCell::new();
        if x != NONE {
            let _ = node.set(x);
        }
        let (n, up) = (std::cell::OnceCell::new(), std::cell::OnceCell::new());
        let x = || *node.get_or_init(|| p.node(local));
        for (o, &c) in out.iter_mut().zip(columns) {
            let at = At {
                node: &x,
                n: &|| *n.get_or_init(|| p.global(local)),
                up: &|| *up.get_or_init(|| Self::parent_id_of(p, local, x())),
                idx: &|| p.idx(local as usize),
                depth: &|| p.depth(local as usize),
                prev: &|| Self::prev_id(p, local),
            };
            *o = self.value_of(p, local, &at, c);
        }
    }
}
impl Viewed {
    /// The rows whose `column` is one of `values`, in document `only` (by index) where the column spans the
    /// documents (a kind, a tape node), else in every document.
    fn lookup_docs(&self, column: usize, values: &[i64], only: Option<usize>) -> Option<Vec<usize>> {
        let b = &*self.batch;
        // A lookup kept to a file keeps to its programs too (decision 0431 §4).
        let kept = |doc: &usize| only.is_none_or(|o| o == *doc || b.docs[*doc].file == b.docs[o].key);
        let mut rows = Vec::new();
        for &v in values {
            if column == self.own() {
                let found = if matches!(self.view, View::Loc(_)) { b.loc(v) } else { b.node(v) };
                rows.extend(found.and_then(|(doc, local)| b.row(self.view, doc, local)));
            } else if Some(column) == self.parent() {
                if v == -1 && matches!(self.view, View::Tape) {
                    rows.extend((0..b.docs.len()).filter(|&doc| b.docs[doc].len() > 0).map(|doc| b.nodes[doc]));
                } else if let Some((doc, local)) = b.node(v) {
                    b.docs[doc].each_child(local, |c| rows.extend(b.row(self.view, doc, c)));
                }
            } else if matches!(self.view, View::Order) && column == 1 {
                // An order key: its document, and the node with that key (`Projected::local_of_order`).
                let key = (v & 0xffff_ffff) as u32;
                rows.extend(b.doc(v >> 32).filter(|_| v >= 0).and_then(|doc| b.docs[doc].local_of_order(key).and_then(|local| b.row(self.view, doc, local))));
            } else if matches!(self.view, View::Sibling) && column == 0 {
                // The row of `a`'s next sibling: a hop along the tape's siblings.
                rows.extend(b.node(v).and_then(|(doc, local)| b.docs[doc].next_sibling(local).and_then(|s| b.row(self.view, doc, s))));
            } else if matches!(self.view, View::Core) && column == 0 {
                rows.extend(b.doc(v).map_or(0..0, |doc| b.nodes[doc]..b.nodes[doc + 1]));
            } else if matches!(self.view, View::Node) && column == 0 {
                // A file's rows, then its programs' (decision 0431 §4).
                let Ok(file) = u32::try_from(v) else { continue };
                for doc in b.doc(v).into_iter().chain(b.programs_of(file)) {
                    rows.extend(b.nodes[doc]..b.nodes[doc + 1]);
                }
            } else if matches!(self.view, View::Core) && column == 1 {
                // A tape node of each document that has it as a node; the statement filters `d`.
                let Ok(t) = u32::try_from(v) else { continue };
                for (doc, p) in b.docs.iter().enumerate().filter(|(doc, _)| kept(doc)) {
                    if let Some(local) = p.ranks().local(Item::Node, t) {
                        rows.push(b.nodes[doc] + local as usize);
                    }
                }
            } else if matches!(self.view, View::Node) && column == 2 {
                rows.extend(b.by_kind(v).iter().filter(|&&(doc, _, _)| kept(&(doc as usize))).map(|&(doc, local, _)| b.nodes[doc as usize] + local as usize));
            } else {
                return None;
            }
        }
        Some(rows)
    }
    /// `Source::lookup_read`: `lookup_docs`'s matches for one value, each row's `columns` read from what found it.
    /// A lookup by a node's id knows the row's id; by its parent, the parent's id and each child's tape node; by a
    /// kind or a tape node, the tape node: no select, granule search or parent search a row for those. Views whose
    /// rows are not one a node (`field`, `loc`), and the other lookups, read none (`false`).
    fn read_matches(&self, column: usize, value: i64, columns: &[usize], each: &mut dyn FnMut(usize, &[i64])) -> bool {
        if columns.len() > 8 || matches!(self.view, View::Field | View::Loc(_)) {
            return false;
        }
        let b = &*self.batch;
        let mut vals = [0i64; 8];
        // A match: its document, local id, tape node (`NONE`: unknown), own id and parent's id where known.
        let mut emit = |doc: usize, local: u32, x: u32, id: Option<i64>, up: Option<i64>| {
            let Some(row) = b.row(self.view, doc, local) else { return };
            let p = &b.docs[doc];
            let node = std::cell::OnceCell::new();
            if x != NONE {
                let _ = node.set(x);
            }
            let (n, parent) = (std::cell::OnceCell::new(), std::cell::OnceCell::new());
            if let Some(id) = id {
                let _ = n.set(id);
            }
            if let Some(up) = up {
                let _ = parent.set(up);
            }
            let x = || *node.get_or_init(|| p.node(local));
            for (o, &c) in vals.iter_mut().zip(columns) {
                let at = At {
                    node: &x,
                    n: &|| *n.get_or_init(|| p.global(local)),
                    up: &|| *parent.get_or_init(|| Self::parent_id_of(p, local, x())),
                    idx: &|| p.idx(local as usize),
                    depth: &|| p.depth(local as usize),
                    prev: &|| Self::prev_id(p, local),
                };
                *o = self.value_of(p, local, &at, c);
            }
            each(row, &vals[..columns.len()]);
        };
        if column == self.own() {
            if let Some((doc, local)) = b.node(value) {
                emit(doc, local, NONE, Some(value), None);
            }
        } else if Some(column) == self.parent() {
            if value == -1 {
                return false;
            }
            if let Some((doc, local)) = b.node(value) {
                b.docs[doc].each_child_node(local, |c, x| emit(doc, c, x, None, Some(value)));
            }
        } else if matches!(self.view, View::Core) && column == 1 {
            let Ok(t) = u32::try_from(value) else { return true };
            for (doc, p) in b.docs.iter().enumerate() {
                if let Some(local) = p.ranks().local(Item::Node, t) {
                    emit(doc, local, t, None, None);
                }
            }
        } else if matches!(self.view, View::Node) && column == 2 {
            // The batch's index by kind (`Batch::by_kind`): each match with its local id and tape node.
            for &(doc, local, x) in b.by_kind(value) {
                emit(doc as usize, local, x, None, None);
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
    /// Each of document `p`'s `n` rows in this view, in order: its index and its columns' reader. A view whose rows are
    /// nodes visits them in order (`Projected::nodes`), so a row's tape node comes with it, not from a select a row;
    /// where `columns` read the parent, the walk (`Projected::walk`) brings it too, not a tape parent read a row.
    fn each_row(&self, p: &Projected, n: usize, columns: impl Iterator<Item = usize>, mut row: impl FnMut(usize, &dyn Fn(usize) -> i64)) {
        let columns: Vec<usize> = columns.collect();
        let walked = self.parent().is_some_and(|q| columns.contains(&q));
        // A child's place and a node's depth are carried in the walk (decision 0353 step 3): no per-node array.
        // So is a sibling before a node, where the walk's parents are the tape's (`Projected::anchored`).
        let carried = columns.iter().any(|&c| matches!((self.view, c), (View::Child, 2) | (View::Tape, 3) | (View::Depth, 1)) || matches!((self.view, c), (View::Sibling, 0)) && !p.anchored);
        // The ids' walk (`node_ids`) only when a column reads a row's or its parent's id.
        let ids = walked || columns.iter().any(|&c| self.reads_id(c));
        // `only`: the item a row's node must be (the `field` view's rows are the fielded nodes).
        let visit = |row: &mut dyn FnMut(usize, &dyn Fn(usize) -> i64), skip: u32, only: Option<Item>| {
            let mut i = 0;
            let mut each = |l: u32, x: u32, at: &At<'_>| {
                if l < skip || only.is_some_and(|item| !p.ranks().is(item, x)) {
                    return;
                }
                row(i, &|c| self.value_of(p, l, at, c));
                i += 1;
            };
            if walked || carried {
                // The open subtrees: each one's end on the tape, its id, its children so far and its last child's id. A
                // parent's id, a child's place, a depth and the sibling before come from the walk, not a point read.
                let mut open: Vec<(u32, i64, u32, i64)> = Vec::new();
                let mut step = |l: u32, x: u32, n: i64| {
                    while open.last().is_some_and(|&(end, _, _, _)| end <= x) {
                        open.pop();
                    }
                    let depth = open.len() as u32;
                    // A walk without ids carries none (no column reads one): `n` is 0 then.
                    let (up, idx, prev) = match open.last_mut() {
                        Some(o) => {
                            o.2 += 1;
                            let prev = o.3;
                            o.3 = n;
                            (o.1, o.2 - 1, prev)
                        }
                        None => (-1, 0, -1),
                    };
                    open.push((p.facts.subtree_end(x), n, 0, -1));
                    let n = || if ids { n } else { p.global(l) };
                    each(l, x, &At { node: &|| x, n: &n, up: &|| up, idx: &|| idx, depth: &|| depth, prev: &|| prev });
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
                    each(l, x, &At { node: &|| x, n: &|| n, up: &|| Self::parent_id(p, l), idx: &|| p.idx(l as usize), depth: &|| p.depth(l as usize), prev: &|| Self::prev_id(p, l) });
                }
            } else {
                for (l, x) in p.nodes() {
                    each(l, x, &At { node: &|| x, n: &|| p.global(l), up: &|| Self::parent_id(p, l), idx: &|| p.idx(l as usize), depth: &|| p.depth(l as usize), prev: &|| Self::prev_id(p, l) });
                }
            }
        };
        match self.view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order => visit(&mut row, 0, None),
            View::Child => visit(&mut row, 1, None),
            // The nodes below the root with a field, in order (`fielded`).
            View::Field => visit(&mut row, 1, Some(Item::Field)),
            // The nodes after a sibling.
            View::Sibling => visit(&mut row, 1, Some(Item::Sibling)),
            View::Loc(place) => {
                for ((i, (_, span)), (_, g)) in p.spans().enumerate().zip(p.loc_ids()).take(n) {
                    row(i, &|c| Self::loc_value(p, place, g, span, c));
                }
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
        )
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
        let (line, last, col) = p.extra.as_ref().expect("core.rerail's lines").place(span);
        match place {
            Place::Line => i64::from(line),
            Place::EndLine => i64::from(last),
            Place::Col => i64::from(col),
            Place::Docof => i64::from(p.file),
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
        let (local, x) = self.local_of_row(p, i);
        let node = || if x != NONE { x } else { p.node(local) };
        let at = At { node: &node, n: &|| p.global(local), up: &|| Self::parent_id(p, local), idx: &|| p.idx(local as usize), depth: &|| p.depth(local as usize), prev: &|| Self::prev_id(p, local) };
        self.value_of(p, local, &at, column)
    }

    /// The local id of document `p`'s row `i` in this view, and its tape node where finding the row read it (the
    /// `field` view's: a select among the fielded nodes), else `NONE`.
    fn local_of_row(&self, p: &Projected, i: usize) -> (u32, u32) {
        match self.view {
            View::Node | View::Line | View::Tape | View::Core | View::Depth | View::Order | View::Loc(_) => (i as u32, NONE),
            View::Child => (i as u32 + 1, NONE),
            View::Field => {
                let x = p.ranks().select(Item::Field, i as u32).expect("a field row");
                (p.ranks().rank(Item::Node, x), x)
            }
            View::Sibling => {
                let x = p.ranks().select(Item::Sibling, i as u32).expect("a sibling row");
                (p.ranks().rank(Item::Node, x), x)
            }
        }
    }

    /// Column `column` of node `local`'s row in this view, what the column needs read through `at`: its tape node,
    /// its id, its parent's id (`-1` above the root), its place among its parent's children and its depth. A scan's
    /// walk carries the last three; a point read reads them off the tape.
    fn value_of(&self, p: &Projected, local: u32, at: &At<'_>, column: usize) -> i64 {
        let l = local as usize;
        let id = || (at.node)() as usize;
        let parent = at.up;
        match (self.view, column) {
            // A program's nodes are its file's (decision 0431 §4); `core` keeps to the document underneath.
            (View::Node, 0) => i64::from(p.file),
            (View::Core, 0) => i64::from(p.key),
            (View::Core, 1) => id() as i64,
            (View::Node, 2) => self.batch.dialects[p.tag].kind(&p.facts, id() as u32),
            (View::Node, 3) => i64::from(p.facts.start(id() as u32)),
            (View::Node, 4) => i64::from(p.facts.end(id() as u32)),
            (View::Line, 1) => i64::from(p.facts.start_line(id() as u32)),
            (View::Child | View::Field, 0) | (View::Tape, 1) => parent(),
            (View::Child, 2) => i64::from((at.idx)()),
            (View::Field, 1) => i64::from(field_atom(p.tag, p.facts.field(id() as u32))),
            (View::Tape, 2) => i64::from(p.size(l)),
            (View::Tape, 3) | (View::Depth, 1) => i64::from((at.depth)()),
            (View::Order, 1) => i64::from(p.key) << 32 | i64::from(p.order(local)),
            (View::Sibling, 0) => (at.prev)(),
            _ => (at.n)(),
        }
    }
}

/// What a view's column reads of a node's row (`Viewed::value_of`), each read when the column asks.
struct At<'a> {
    node: &'a dyn Fn() -> u32,
    n: &'a dyn Fn() -> i64,
    up: &'a dyn Fn() -> i64,
    idx: &'a dyn Fn() -> u32,
    depth: &'a dyn Fn() -> u32,
    /// The numbered sibling before it, by id (`next_sibling`'s `a`).
    prev: &'a dyn Fn() -> i64,
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
        .filter(|&(_, id, _)| matches!(dialect.name(facts, id), "function_definition" | "class_definition"))
        .map(|(_, id, n)| ((facts.start(id), facts.end(id)), n))
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
        relations.into_iter().map(|(name, r)| (name, as_source(name, Arc::new(r), &batch))).collect();
    out.extend(views(&batch));
    Ok(out)
}

/// The views a batch of `shape` binds by name (`views`).
fn named_views(shape: Shape) -> Vec<(&'static str, View)> {
    let mut named = vec![("node", View::Node), ("child", View::Child), ("field", View::Field), ("tape", View::Tape), ("core_node", View::Core), ("node_order", View::Order)];
    match shape {
        Shape::T => named.push(("line", View::Line)),
        Shape::Core => named.extend([
            ("line", View::Loc(Place::Line)),
            ("end_line", View::Loc(Place::EndLine)),
            ("col", View::Loc(Place::Col)),
            // `core.rerail`'s `docof(x: loc, d)` is an input (`rr::lower`): nodes' and spans' documents.
            ("docof", View::Loc(Place::Docof)),
            ("ast_depth", View::Depth),
            ("next_sibling", View::Sibling),
        ]),
    }
    named
}

/// Whether a batch of `shape` binds a view named `name`.
pub fn has_view(shape: Shape, name: &str) -> bool {
    named_views(shape).iter().any(|(n, _)| *n == name)
}

/// The batch's views: `node`, `child`, `field`, `tape` and the `py.*` families.
fn views(batch: &Arc<Batch>) -> Vec<(&'static str, Arc<dyn Source>)> {
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
    /// `demand`: per matcher, the matchers whose roots hold every node its rows are read at (`None`: everywhere).
    fn compile(&self, matchers: &[(String, String, Vec<String>)], demand: &[Option<Vec<usize>>], policies: &[serde_json::Value]) -> Result<Matchers, String>;
}

static AST_COMPILER: std::sync::OnceLock<&'static dyn AstCompiler> = std::sync::OnceLock::new();

/// Registers the compiler of `ast` matchers (once, at load).
pub fn install_ast_compiler(compiler: &'static dyn AstCompiler) {
    let _ = AST_COMPILER.set(compiler);
}

/// `matchers` compiled; a program with matchers and no registered compiler fails, naming the first.
pub fn compile_matchers(matchers: &[(String, String, Vec<String>)], demand: &[Option<Vec<usize>>], policies: &[serde_json::Value]) -> Result<Matchers, String> {
    if matchers.is_empty() {
        return Ok(Matchers::default());
    }
    match AST_COMPILER.get() {
        Some(compiler) => compiler.compile(matchers, demand, policies),
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
    Grower::default().sources(store, documents, sets, Some(matchers), tests, binding, projection, false)
}

/// A chunk's sources that grow by its rounds' documents (decision 0431 §6): the batch is built once, and a round's tape
/// values and programs are added to it (`Growth`), not built again with it. The batch's relations, the `match_all`
/// ones and the text tests' are grown by the round's rows (`Relation::grown`: in the same block relations); the ast
/// matchers' relations are the files' (a round adds none), kept; the views and axes are made over the grown batch.
#[derive(Default)]
pub struct Grower {
    growth: Growth,
    /// The relations of rows, by name in the order first made, each the last batch's.
    rels: Vec<(String, Arc<Relation>)>,
    /// The ast matchers' relations, made with the files.
    matched: Vec<(String, Arc<dyn Source>)>,
    /// The batches made: the first's relations are taken as they come (a name twice is bound twice, as before).
    built: usize,
    /// Each relation with the rows of the tape values admitted since (decision 0431 §8, `admit`): the chunk's rows,
    /// then the values', in the same block relation. The chunk's relations (`rels`) stay its own rows.
    with_values: Vec<(String, Arc<Relation>)>,
}

impl Grower {
    /// [`with_matchers`]'s sources, and the grower that adds a round's documents to them.
    pub fn start(
        store: &ContentStore,
        documents: Vec<Document>,
        sets: &[(String, Vec<i64>)],
        matchers: &Matchers,
        tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
    ) -> StoreResult<(Vec<(String, Arc<dyn Source>)>, Grower)> {
        let mut grower = Grower::default();
        let out = grower.sources(store, documents, sets, Some(matchers), tests, binding, projection, true)?;
        Ok((out, grower))
    }

    /// The sources with `documents` added: a round's, past every document the batch holds.
    pub fn grow(
        &mut self,
        store: &ContentStore,
        documents: Vec<Document>,
        sets: &[(String, Vec<i64>)],
        tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
        self.sources(store, documents, sets, None, tests, binding, projection, true)
    }

    /// The sources of tape value `document` alone, admitted inside the chunk's fixpoint (decision 0431 §8): projected
    /// into the chunk's block, its hosts' lines and its strings the growth's, its rows its own, a part of each of the
    /// chunk's sources (`values::Layered`). A document the batch holds (a value given a place more) has the rows of
    /// those places alone.
    pub fn admit(
        &mut self,
        store: &ContentStore,
        document: Document,
        sets: &[(String, Vec<i64>)],
        tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
        self.sources_of(store, vec![document], sets, None, tests, binding, projection, true, true)
    }

    /// Tape value's relation `name`, its rows admitted (decision 0431 §8): written once, past the chunk's rows of
    /// `name` and the values' admitted before, in the same block relation (`Relation::grown`), so that the device
    /// binds the chunk's and the values' rows whole; read as the rows past those (`values::Slice`). A relation the
    /// chunk makes none of, or one read with its batch (`as_source`), is the value's own.
    fn admitted(&mut self, name: &str, rows: Relation, batch: &Arc<Batch>) -> Arc<dyn Source> {
        let at = self.with_values.iter().position(|(n, _)| n == name);
        let prev = at.map(|i| &self.with_values[i].1).or_else(|| self.rels.iter().find(|(n, _)| n == name).map(|(_, r)| r));
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

    /// The relation `name` with `rows` after its own (the first rows of a name are its own), as one that grows.
    fn merge(&mut self, name: String, rows: Relation, growing: bool) {
        match self.rels.iter_mut().find(|(n, _)| *n == name).filter(|_| self.built > 0) {
            Some((_, r)) => *r = Arc::new(r.grown(rows)),
            None => self.rels.push((name, Arc::new(if growing { rows.growable() } else { rows }))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn sources(
        &mut self,
        store: &ContentStore,
        documents: Vec<Document>,
        sets: &[(String, Vec<i64>)],
        matchers: Option<&Matchers>,
        tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
        growing: bool,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
        self.sources_of(store, documents, sets, matchers, tests, binding, projection, growing, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn sources_of(
        &mut self,
        store: &ContentStore,
        documents: Vec<Document>,
        sets: &[(String, Vec<i64>)],
        matchers: Option<&Matchers>,
        tests: &crate::code_rule_dsl::logic::strings::Vocabulary,
        binding: crate::code_rule_dsl::facts::nearest::Binding,
        projection: &Projection,
        growing: bool,
        alone: bool,
    ) -> StoreResult<Vec<(String, Arc<dyn Source>)>> {
    let built = crate::code_rule_dsl::logic::profile::span(|| "project and intern".into());
    let metal = matches!(binding, crate::code_rule_dsl::facts::nearest::Binding::Metal);
    let tapes: Vec<(Arc<Facts>, crate::code_rule_dsl::tape::TapeDoc, String)> = match matchers {
        Some(m) if !m.is_empty() => documents.iter().filter(|d| d.guest.is_none()).filter_map(|d| Some((Arc::clone(&d.facts), d.tape.as_ref()?.0.clone(), d.path.clone()))).collect(),
        _ => Vec::new(),
    };
    let (batch, relations, match_all, from) = Batch::add(&mut self.growth, store, documents, projection, &tests.matches, &tests.markers, metal, alone)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, batch.nodes.last().copied().unwrap_or(0) as u64);
    drop(built);
    // The batch's relations, but none the program derives (`Projection::derived`); the `match_all` relations
    // (`preparation::text::TextRelations`): each regex's matches in the batch's spans.
    // An admitted value's relations are a part of the chunk's (`admitted`), never merged into its own.
    let mut own: Vec<(String, Arc<dyn Source>)> = Vec::new();
    for (name, r) in relations.into_iter().filter(|(name, _)| !projection.derived.contains(*name)) {
        if alone { own.push((name.to_owned(), self.admitted(name, r, &batch))) } else { self.merge(name.to_owned(), r, growing) }
    }
    for (name, r) in match_all {
        if alone { own.push((name.clone(), self.admitted(&name, r, &batch))) } else { self.merge(name, r, growing) }
    }
    if let Some(Matchers(Some(wave))) = matchers {
        self.matched = wave.matched(&batch, &tapes, binding, &store.atoms_handle())?;
    }
    // The text tests' relations (decision 0037): the batch's strings that pass each, each string tested once a run
    // (a round's, its new strings).
    let text = crate::code_rule_dsl::logic::profile::span(|| "text tests".into());
    let (strings, symbols) = (&self.growth.strings[from..], &self.growth.atoms[from..]);
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Value, strings.len() as u64);
    let tested: Vec<(String, Relation)> = tests
        .tests
        .iter()
        .zip(tests.rows(strings, symbols))
        .map(|((name, _), rows)| {
            let mut r = Relation::new(&["s"]);
            rows.into_iter().for_each(|s| r.push(&[s]));
            (name.clone(), r)
        })
        .collect();
    for (name, r) in tested {
        if alone { own.push((name.clone(), self.admitted(&name, r, &batch))) } else { self.merge(name, r, growing) }
    }
    drop(text);
    let mut out: Vec<(String, Arc<dyn Source>)> = if alone {
        own
    } else {
        self.built += 1;
        let mut out: Vec<(String, Arc<dyn Source>)> = self.rels.iter().map(|(name, r)| (name.clone(), as_source(name, Arc::clone(r), &batch))).collect();
        out.extend(self.matched.iter().map(|(n, s)| (n.clone(), Arc::clone(s))));
        out
    };
    let rows_named: BTreeSet<String> = out.iter().map(|(n, _)| n.clone()).collect();
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
        return Ok(out_with_within(out, &batch, binding));
    }
    // Containment (decision 0050): made per looked-up node, so a program that never reads it pays nothing.
    Ok(out_with_within(out, &batch, binding))
    }
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
    let all: Vec<usize> = (0..width).collect();
    // The first walk: each column's strictly increasing, and the documents' order.
    let (mut increasing, mut ordered) = (vec![true; width], true);
    let mut last: Option<Vec<i64>> = None;
    source.for_rows(&all, &mut |_, row| {
        if let Some(was) = &mut last {
            ordered &= was[0] >> 32 <= row[0] >> 32;
            for c in 0..width {
                increasing[c] &= was[c] < row[c];
            }
            was.copy_from_slice(row);
        } else {
            last = Some(row.to_vec());
        }
    });
    if !ordered {
        return measured_sorted(source);
    }
    let key = (rows as i64, rows as i64, rows as i64, rows as i64);
    let counted: Vec<usize> = (0..width).filter(|&c| !increasing[c]).collect();
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
        let mut read: Vec<usize> = vec![0];
        read.extend(counted.iter().copied().filter(|&c| c != 0));
        let at: Vec<usize> = counted.iter().map(|c| read.iter().position(|r| r == c).expect("read")).collect();
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
    let mut docs: Vec<i64> = Vec::with_capacity(rows);
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
