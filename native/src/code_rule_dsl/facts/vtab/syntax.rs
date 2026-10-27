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

use super::{Modules, Relation, Source};
use crate::code_rule_dsl::facts::Graph;
use crate::code_rule_dsl::facts::core::{Facts, KIND_STRIDE, NONE, kind_atom};
use crate::code_rule_dsl::language::{KindCatalog, from_tape_tag};
use crate::content_store::{ContentStore, StoreError, StoreResult};
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Arc;

/// One document of a batch.
pub(crate) struct Document {
    /// `d`: the document's key, its path atom in the push path.
    pub key: u32,
    /// The path relative to its root, from which `module` and `pkg` follow (T's `module`).
    pub path: String,
    /// The `#core:` name the facts were built for, and their digest (`core::digest`).
    pub core: String,
    pub digest: [u8; 32],
    pub facts: Arc<Facts>,
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
    let m = rel.trim_end_matches(".py").replace('/', ".");
    m.strip_suffix(".__init__").map(str::to_owned).unwrap_or(m)
}

/// The package relative imports resolve against (`__package__`): a package's own name, else its
/// module's parent.
pub(crate) fn package(rel: &str) -> String {
    let m = module(rel);
    if rel.ends_with("__init__.py") { m } else { m.rsplit_once('.').map(|(p, _)| p.to_owned()).unwrap_or_default() }
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
    FunctionItem,
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
            "function_item" => Self::FunctionItem,
            _ => Self::Other,
        }
    }
}

/// A dialect's kinds as T's exporter reads them. T's kinds were names, and a grammar gives some names several codes
/// (aliases): the `kind` column holds each name's canonical code (`KindCatalog::kind_id`, what a program's constant
/// lowers to), and a lookup reads the postings of every code of that name.
struct Dialect {
    tag: u8,
    catalog: KindCatalog,
    classes: Vec<Class>,
    operator: Option<u16>,
    canonical: Vec<u16>,
    codes: HashMap<u16, Vec<u16>>,
}
impl Dialect {
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
        Ok(Self { tag, catalog, classes, operator: catalog.field_id("operator"), canonical, codes })
    }
    fn code(f: &Facts, id: u32) -> u16 {
        (f.kind[id as usize] % KIND_STRIDE) as u16
    }
    fn class(&self, f: &Facts, id: u32) -> Class {
        self.classes.get(usize::from(Self::code(f, id))).copied().unwrap_or(Class::Other)
    }
    fn name(&self, f: &Facts, id: u32) -> &'static str {
        self.catalog.kind_name(Self::code(f, id)).unwrap_or("")
    }
}

/// A document's named non-comment nodes, numbered in preorder.
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
}
impl Projected {
    fn new(key: u32, facts: Arc<Facts>, dialect: &Dialect) -> Self {
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
            facts: Arc::clone(&facts),
        };
        let mut next_idx: Vec<u32> = Vec::new();
        for id in 0..f.len() as u32 {
            if !f.named(id) || dialect.class(f, id) == Class::Comment {
                continue;
            }
            // T walks named children from the root: a node under one left out is not reached.
            let parent = match f.parent(id) {
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
        p
    }
    fn len(&self) -> u32 {
        self.core.len() as u32
    }
    fn global(&self, local: u32) -> i64 {
        i64::from(self.key) << 32 | i64::from(local)
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
}

/// Which view a source serves.
#[derive(Clone, Copy)]
enum View {
    Node,
    Child,
    Field,
    Tape,
}

impl Batch {
    /// Projects each document and interns the batch's strings; the documents' keys are distinct.
    /// Returns the batch and its small relations (`doc`, `pkg`, `text`, `intval`, `op`).
    fn build(store: &ContentStore, documents: Vec<Document>) -> StoreResult<(Arc<Self>, Vec<(&'static str, Relation)>)> {
        let mut documents = documents;
        documents.sort_by_key(|d| d.key);
        if documents.windows(2).any(|w| w[0].key == w[1].key) {
            return Err(StoreError::Invalid("a batch holds a document key twice".into()));
        }
        let mut dialects: HashMap<u8, Dialect> = HashMap::new();
        let mut strings: Vec<String> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        let mut string = |s: &str| -> usize {
            *at.entry(s.to_owned()).or_insert_with(|| {
                strings.push(s.to_owned());
                strings.len() - 1
            })
        };
        // Rows with string indexes; the indexes become atoms once the batch's strings are interned.
        let (mut doc, mut pkg, mut text, mut intval, mut op) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut docs = Vec::with_capacity(documents.len());
        for d in &documents {
            let tag = (d.facts.kind.first().copied().unwrap_or(0) / KIND_STRIDE) as u8;
            if !dialects.contains_key(&tag) {
                dialects.insert(tag, Dialect::new(tag)?);
            }
            let dialect = &dialects[&tag];
            let p = Projected::new(d.key, Arc::clone(&d.facts), dialect);
            let key = i64::from(d.key);
            doc.push([key, string(&d.path) as i64, string(&module(&d.path)) as i64]);
            pkg.push([key, string(&package(&d.path)) as i64]);
            let f = &*d.facts;
            for (local, &id) in p.core.iter().enumerate() {
                let n = p.global(local as u32);
                match dialect.class(f, id) {
                    Class::Text => {
                        let t = f.text(id);
                        if !t.contains(['\t', '\n', '\r', '\\']) {
                            text.push([n, string(t) as i64]);
                        }
                    }
                    Class::Integer => {
                        if let Ok(v) = f.text(id).replace('_', "").parse::<i64>() {
                            intval.push([n, v]);
                        }
                    }
                    Class::Operator => {
                        let operator = dialect.operator.filter(|&o| o != 0);
                        if let Some(&o) = f.children(id).iter().find(|&&c| operator == Some(f.field[c as usize])) {
                            op.push([n, string(dialect.name(f, o)) as i64]);
                        }
                    }
                    // `async def`: the keyword is an anonymous child, recorded as the definition's modifier.
                    Class::FunctionDefinition if f.children(id).first().is_some_and(|&k| dialect.name(f, k) == "async") => {
                        op.push([n, string("async") as i64]);
                    }
                    Class::FunctionItem
                        if f.children(id).iter().any(|&m| {
                            dialect.name(f, m) == "function_modifiers" && f.children(m).iter().any(|&k| dialect.name(f, k) == "async")
                        }) =>
                    {
                        op.push([n, string("async") as i64]);
                    }
                    _ => {}
                }
            }
            docs.push(p);
        }
        let atoms = store.intern_atoms(&strings.iter().map(String::as_str).collect::<Vec<_>>())?;
        let atom = |i: i64| i64::from(atoms[i as usize]);
        let relation = |columns: &[&str], rows: &mut dyn Iterator<Item = Vec<i64>>| {
            let mut r = Relation::new(columns);
            rows.for_each(|row| r.push(&row));
            r
        };
        let relations = vec![
            ("doc", relation(&["d", "path", "module"], &mut doc.iter().map(|&[d, p, m]| vec![d, atom(p), atom(m)]))),
            ("pkg", relation(&["d", "p"], &mut pkg.iter().map(|&[d, p]| vec![d, atom(p)]))),
            ("text", relation(&["n", "s"], &mut text.iter().map(|&[n, s]| vec![n, atom(s)]))),
            ("intval", relation(&["n", "v"], &mut intval.iter().map(|&[n, v]| vec![n, v]))),
            ("op", relation(&["n", "s"], &mut op.iter().map(|&[n, s]| vec![n, atom(s)]))),
        ];
        let offsets = |count: &dyn Fn(&Projected) -> usize| {
            let mut v = vec![0];
            docs.iter().for_each(|p| v.push(v.last().unwrap() + count(p)));
            v
        };
        let nodes = offsets(&|p| p.core.len());
        let children = offsets(&|p| p.core.len().saturating_sub(1));
        let fields = offsets(&|p| p.fielded.len());
        Ok((Arc::new(Batch { docs, dialects, nodes, children, fields }), relations))
    }

    fn offsets(&self, view: View) -> &[usize] {
        match view {
            View::Node | View::Tape => &self.nodes,
            View::Child => &self.children,
            View::Field => &self.fields,
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
        let local = (n & 0xffff_ffff) as u32;
        (n >= 0 && local < self.docs[doc].len()).then_some((doc, local))
    }
    /// The view's row of node `local` of document `doc`, if it has one.
    fn row(&self, view: View, doc: usize, local: u32) -> Option<usize> {
        let p = &self.docs[doc];
        let within = match view {
            View::Node | View::Tape => Some(local as usize),
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
            let base = p.global(0);
            if base > hi {
                break;
            }
            let start = (lo.max(base) - base) as u32;
            let end = ((hi - base).min(i64::from(p.len()) - 1) + 1).max(0) as u32;
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
            View::Child => &["p", "c", "idx"],
            View::Field => &["p", "f", "c"],
            View::Tape => &["n", "par", "size", "depth"],
        }
    }
    /// The column holding the row's own node, which ranges and direct lookups address.
    fn own(&self) -> usize {
        match self.view {
            View::Node | View::Child => 1,
            View::Field => 2,
            View::Tape => 0,
        }
    }
    /// The column holding the row's parent, looked up through the children lists.
    fn parent(&self) -> Option<usize> {
        match self.view {
            View::Child | View::Field => Some(0),
            View::Tape => Some(1),
            View::Node => None,
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
        } else if matches!(self.view, View::Node) && column == 0 {
            self.len() / self.batch.docs.len().max(1)
        } else {
            (self.len() / 64).max(1)
        };
        if batch { per * 8 } else { per }
    }
    fn indexed(&self, column: usize) -> bool {
        column == self.own() || Some(column) == self.parent() || matches!(self.view, View::Node) && matches!(column, 0 | 2)
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
        let b = &*self.batch;
        let mut rows = Vec::new();
        for &v in values {
            if column == self.own() {
                rows.extend(b.node(v).and_then(|(doc, local)| b.row(self.view, doc, local)));
            } else if Some(column) == self.parent() {
                if v == -1 && matches!(self.view, View::Tape) {
                    rows.extend((0..b.docs.len()).filter(|&doc| b.docs[doc].len() > 0).map(|doc| b.nodes[doc]));
                } else if let Some((doc, local)) = b.node(v) {
                    rows.extend(b.docs[doc].children(local).filter_map(|c| b.row(self.view, doc, c)));
                }
            } else if matches!(self.view, View::Node) && column == 0 {
                rows.extend(b.doc(v).map_or(0..0, |doc| b.nodes[doc]..b.nodes[doc + 1]));
            } else if matches!(self.view, View::Node) && column == 2 {
                let (Ok(tag), Ok(code)) = (u8::try_from(v / i64::from(KIND_STRIDE)), u16::try_from(v % i64::from(KIND_STRIDE))) else {
                    continue;
                };
                let Some(dialect) = b.dialects.get(&tag).filter(|_| v >= 0) else { continue };
                let atoms: Vec<u32> = dialect.codes.get(&code).map_or(&[][..], Vec::as_slice).iter().map(|&c| kind_atom(tag, c)).collect();
                for (doc, p) in b.docs.iter().enumerate().filter(|(_, p)| p.tag == tag) {
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
    fn value(&self, row: usize, column: usize) -> i64 {
        let (p, i) = self.batch.locate(self.view, row);
        let local = match self.view {
            View::Node | View::Tape => i as u32,
            View::Child => i as u32 + 1,
            View::Field => p.fielded[i],
        };
        let l = local as usize;
        let id = p.core[l] as usize;
        let parent = || if p.parent[l] == NONE { -1 } else { p.global(p.parent[l]) };
        match (self.view, column) {
            (View::Node, 0) => i64::from(p.key),
            (View::Node, 2) => {
                let code = (p.facts.kind[id] % KIND_STRIDE) as usize;
                i64::from(kind_atom(p.tag, self.batch.dialects[&p.tag].canonical.get(code).copied().unwrap_or(code as u16)))
            }
            (View::Node, 3) => i64::from(p.facts.start[id]),
            (View::Node, 4) => i64::from(p.facts.end[id]),
            (View::Child | View::Field, 0) | (View::Tape, 1) => parent(),
            (View::Child, 2) => i64::from(p.idx[l]),
            (View::Field, 1) => i64::from(field_atom(p.tag, p.facts.field[id])),
            (View::Tape, 2) => i64::from(p.size[l]),
            (View::Tape, 3) => i64::from(p.depth[l]),
            _ => p.global(local),
        }
    }
}

/// The field atom of field `code` in the dialect whose tape tag is `tag`.
pub(crate) fn field_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) << 16 | u32::from(code)
}

/// T's nine base relations over `documents`, by table name.
pub(crate) fn sources(store: &ContentStore, documents: Vec<Document>) -> StoreResult<Vec<(&'static str, Arc<dyn Source>)>> {
    let (batch, relations) = Batch::build(store, documents)?;
    let mut out: Vec<(&'static str, Arc<dyn Source>)> =
        relations.into_iter().map(|(name, r)| (name, Arc::new(r) as Arc<dyn Source>)).collect();
    for (name, view) in [("node", View::Node), ("child", View::Child), ("field", View::Field), ("tape", View::Tape)] {
        out.push((name, Arc::new(Viewed { batch: Arc::clone(&batch), view })));
    }
    Ok(out)
}

/// Binds T's base relations over `documents` on `db`, after checking that each document's facts are
/// those its `#core:` name binds now (§4): a stale document fails the binding.
pub(crate) fn bind(modules: &Modules, db: &Connection, graph: &(impl Graph + ?Sized), documents: Vec<Document>) -> StoreResult<()> {
    for d in &documents {
        if graph.binding(&d.core)? != Some(d.digest) {
            return Err(StoreError::Invalid(format!("stale core facts for {}", d.core)));
        }
    }
    for (name, source) in sources(graph.store(), documents)? {
        modules.bind(db, name, source).map_err(StoreError::Database)?;
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::code_rule_dsl) mod tests;
