//! SPIKE (throw-away): specialized executors for the strata of the reference library that stand for W's Rust.
//!
//! `RERAIL_SPIKE_STRATA=1`: a stratum whose relations are one of the sets below is run here, not by the generic
//! joins: its rows are computed by W's algorithm (a parent walk to the nearest scope or function, the scope chain
//! walked per name, as W's `ScopeTable` and `resolve_python_index` do) from the stratum's own inputs (its lower
//! strata's relations) and the chunk's tree, and retained as the stratum's relations, so the execution shape (one
//! engine run per stratum per chunk, relations materialized between them) stays as it is. Detected by relation name
//! (a spike's shortcut).
//!
//! The chunk's tree (`Tree`) is read once off the batch's projections (`Batch::tree_arrays`: what the `kind`,
//! `child`, `field` and `text` views give, with no view read) and held in the batch's own slot (`Batch::spike`), so it
//! lives and dies with the batch. Rows are flat (`Flat`: `k` values a row), in and out: no vector a row.
//!
//! `RERAIL_SPIKE_CHECK=1` as well: the stratum is computed both ways and the rows compared (`SPIKE CHECK` lines).

use std::cell::Cell;
use std::sync::{Arc, Mutex};

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};

use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::facts::vtab::syntax::Batch;

pub fn enabled() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_STRATA").is_some())
}

pub fn check() -> bool {
    rerail_prof::code_rule_dsl::logic::profile::setting(|| std::env::var_os("RERAIL_SPIKE_CHECK").is_some())
}

/// The strata run here, by their relations (sorted).
const STRATA: [&[&str]; 8] = [&["look", "walk"], &["wstart"], &["rf_in"], &["rf_fn_in"], &["nameref"], &["bind_only"], &["rf_site"], &["rf_selfish"]];

/// Whether the stratum of `rels` is run here.
pub fn handles(rels: &[&str]) -> bool {
    let mut sorted = rels.to_vec();
    sorted.sort_unstable();
    STRATA.contains(&sorted.as_slice())
}

/// No row: a root's parent, a node with no ancestor in a set, an id not of the batch.
pub(super) const NONE: u32 = u32::MAX;
/// No value: a row with no field, no text.
pub(super) const NIL: i64 = i64::MIN;

/// Rows held flat: `k` values a row.
#[derive(Default)]
pub struct Flat {
    pub k: usize,
    pub v: Vec<i64>,
}

impl Flat {
    fn new(k: usize) -> Self {
        Flat { k, v: Vec::new() }
    }

    pub fn rows(&self) -> std::slice::ChunksExact<'_, i64> {
        self.v.chunks_exact(self.k.max(1))
    }

    fn push(&mut self, row: &[i64]) {
        self.v.extend_from_slice(row);
    }

    /// Its rows as sorted tuples of their first `N` values, for binary search.
    fn sorted<const N: usize>(&self) -> Vec<[i64; N]> {
        let mut out: Vec<[i64; N]> = self.rows().map(|r| std::array::from_fn(|i| r[i])).collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// A batch's named tree as dense arrays by its node rows (W's tape cursor, flattened): made once for the batch, at
/// its first spiked stratum, held in its slot (`Batch::spike`). Each node row's id, kind and document always; the
/// rest at its first reader: its shape (`Shape`: parents, children, fields), spans and texts.
pub struct Tree {
    pub(super) ids: Vec<i64>,
    /// Each row's kind atom (`kind`; 32 bits, `core::kind_atom`).
    pub(super) kinds: Vec<u32>,
    /// Each row's document (its place in the batch), and each document's first row and file (`node`'s `d`).
    pub(super) doc: Vec<u32>,
    pub(super) base: Vec<u32>,
    pub(super) file: Vec<i64>,
    /// Parents, children and fields (`Shape`): made at the first stratum that reads one (`Tree::shape_of`).
    shape: std::sync::OnceLock<Shape>,
    /// Each row's span (`node`'s `start` and `end`): read at its first reader (`Chunk::spans`).
    spans: std::sync::OnceLock<(Vec<u32>, Vec<u32>)>,
    pub(super) atoms: HashMap<String, i64>,
    fields: std::sync::OnceLock<HashMap<String, i64>>,
    /// Each row's nearest strict ancestor in a set of kinds, by the set's name: computed once for the batch.
    nearest: Mutex<HashMap<&'static str, Arc<Vec<u32>>>>,
    /// The same, by the set's kind atoms (`spike_eval`).
    pub(super) near_by: Mutex<HashMap<Vec<i64>, Arc<Vec<u32>>>>,
    /// The rows of each kind (`spike_eval`), made at the first enumeration by kind.
    pub(super) by_kind: std::sync::OnceLock<HashMap<i64, Vec<u32>>>,
    /// Each row's `text`, read at its first use (`Chunk::text`): `TEXT_UNREAD` until then, `NIL` for none.
    texts: std::sync::OnceLock<Vec<std::sync::atomic::AtomicI64>>,
    /// The tape values admitted into the chunk (decision 0431 §8), each a batch of its own: their rows follow the
    /// chunk's batch's, each batch's from its first row and document here (`value_rows`, `value_docs`). Held weakly:
    /// a value's batch may hold the chunk's, whose slot holds this, and the chunk's sources hold them while it runs.
    values: Vec<std::sync::Weak<Batch>>,
    value_rows: Vec<u32>,
    value_docs: Vec<u32>,
    /// The relations `spike_eval` holds for the chunk, by the run and name: kept when the tree is made again over
    /// more values (`Slot`).
    pub(super) held: Arc<Mutex<HashMap<(usize, String), Arc<super::spike_eval::HRel>>>>,
    /// The relations `spike_eval` found projections of one tree atom, by the run and name.
    pub(super) aliases: Arc<Mutex<HashMap<(usize, String), super::spike_eval::Alias>>>,
}

/// A batch's tree, held in its slot (`Batch::spike`): made again, its held relations kept, when more tape values are
/// admitted into the chunk than it was made over.
#[derive(Default)]
pub struct Slot(Mutex<Option<Arc<Tree>>>);

impl Slot {
    /// The tree of `b` and the values `values` admitted into its chunk so far.
    pub fn tree(&self, b: &Batch, values: Vec<Arc<Batch>>) -> Arc<Tree> {
        let mut at = self.0.lock().unwrap();
        if let Some(t) = at.as_ref()
            && t.values.len() == values.len()
        {
            return Arc::clone(t);
        }
        let t = Arc::new(Tree::of(b, values, at.as_deref()));
        *at = Some(Arc::clone(&t));
        t
    }
}

/// A batch's named tree's shape, by node row (`Tree::shape_of`).
pub struct Shape {
    pub(super) parent: Vec<u32>,
    /// Each row's place among its siblings (`child(p, c, idx)`).
    pub(super) idx: Vec<u32>,
    /// The field each row is in (`NIL` for none).
    pub(super) field: Vec<i64>,
    pub(super) depth: Vec<u32>,
    /// Each row's children, in order: `kids[child_at[r]..child_at[r + 1]]`.
    child_at: Vec<u32>,
    kids: Vec<u32>,
    /// Each row's subtree's end: the rows strictly below `r` are `r + 1..end[r]` (rows are preorder).
    pub(super) end: Vec<u32>,
}

impl Shape {
    fn of(b: &Batch, values: &[std::sync::Weak<Batch>], value_rows: &[u32]) -> Shape {
        let _span = super::super::profile::span(|| "spike: the batch's tree's shape".into());
        let mut a = b.tree_shape();
        for (v, &at) in values.iter().zip(value_rows) {
            let more = v.upgrade().expect("a value's batch lives while its chunk runs").tree_shape();
            a.parent.extend(more.parent.iter().map(|&p| if p == NONE { NONE } else { p + at }));
            a.idx.extend(more.idx);
            a.field.extend(more.field);
        }
        let n = a.parent.len();
        let mut child_at = vec![0u32; n + 1];
        for &p in &a.parent {
            if p != NONE {
                child_at[p as usize + 1] += 1;
            }
        }
        for r in 0..n {
            child_at[r + 1] += child_at[r];
        }
        let mut at = child_at.clone();
        let mut kids = vec![0u32; child_at[n] as usize];
        // Rows are each document's nodes in preorder: a parent's children placed in row order are in their order.
        for (c, &p) in a.parent.iter().enumerate() {
            if p != NONE {
                kids[at[p as usize] as usize] = c as u32;
                at[p as usize] += 1;
            }
        }
        let mut depth = vec![u32::MAX; n];
        let mut chain = Vec::new();
        for r in 0..n {
            let mut up = r as u32;
            while up != NONE && depth[up as usize] == u32::MAX {
                chain.push(up);
                up = a.parent[up as usize];
            }
            let base = if up == NONE { 0 } else { depth[up as usize] + 1 };
            for (d, c) in (base..).zip(chain.drain(..).rev()) {
                depth[c as usize] = d;
            }
        }
        // Preorder rows: a row's subtree ends where its last child's does.
        let mut end: Vec<u32> = (1..=n as u32).collect();
        for r in (0..n).rev() {
            let p = a.parent[r];
            if p != NONE && end[p as usize] < end[r] {
                end[p as usize] = end[r];
            }
        }
        Shape { parent: a.parent, idx: a.idx, field: a.field, depth, child_at, kids, end }
    }

    fn bytes(&self) -> usize {
        4 * (self.parent.capacity() + self.idx.capacity() + self.depth.capacity() + self.child_at.capacity() + self.kids.capacity() + self.end.capacity()) + 8 * self.field.capacity()
    }

    pub(super) fn kids(&self, r: u32) -> &[u32] {
        &self.kids[self.child_at[r as usize] as usize..self.child_at[r as usize + 1] as usize]
    }

    /// `field(_, f, r)`.
    fn in_field(&self, r: u32, field: i64) -> bool {
        self.field[r as usize] == field
    }

    /// `child(r, c, 0)`.
    fn first_child(&self, r: u32) -> Option<u32> {
        self.kids(r).iter().copied().find(|&c| self.idx[c as usize] == 0)
    }

    /// `r`'s child in `field`.
    fn field_child(&self, r: u32, field: i64) -> Option<u32> {
        self.kids(r).iter().copied().find(|&c| self.in_field(c, field))
    }

    /// Each row strictly below `r`.
    fn each_below(&self, r: u32, f: &mut impl FnMut(u32)) {
        for c in r + 1..self.end[r as usize] {
            f(c);
        }
    }

    /// `within(n, a)`: `n` strictly below `a`.
    pub(super) fn within(&self, n: u32, a: u32) -> bool {
        a < n && n < self.end[a as usize]
    }
}

/// A row's text not read yet (`Tree::texts`).
pub(super) const TEXT_UNREAD: i64 = i64::MIN + 1;

/// The tree of a batch's stratum, and its batch, which finds a node id's row (`Batch::row_of`).
pub struct Chunk<'a> {
    pub t: &'a Tree,
    pub b: &'a Batch,
    memo: Cell<u32>,
}

impl<'a> Chunk<'a> {
    pub fn new(t: &'a Tree, b: &'a Batch) -> Self {
        Chunk { t, b, memo: Cell::new(crate::code_rule_dsl::facts::vtab::NO_MEMO) }
    }

    /// The tree's shape, made at its first reader.
    pub(super) fn shape(&self) -> &'a Shape {
        self.t.shape.get_or_init(|| Shape::of(self.b, &self.t.values, &self.t.value_rows))
    }

    /// Each row's span (`node`'s `start` and `end`), read at its first reader.
    pub(super) fn spans(&self) -> &'a (Vec<u32>, Vec<u32>) {
        self.t.spans.get_or_init(|| {
            let (mut start, mut end) = self.b.tree_spans();
            for v in &self.t.values {
                let (s, e) = v.upgrade().expect("a value's batch lives while its chunk runs").tree_spans();
                start.extend(s);
                end.extend(e);
            }
            (start, end)
        })
    }

    /// Row `r`'s `text` atom, `NIL` where the view has none: read off the batch at its first use.
    pub(super) fn text(&self, r: u32) -> i64 {
        use std::sync::atomic::{AtomicI64, Ordering::Relaxed};
        let t = self.t;
        let texts = t.texts.get_or_init(|| (0..t.ids.len()).map(|_| AtomicI64::new(TEXT_UNREAD)).collect());
        let at = &texts[r as usize];
        let x = at.load(Relaxed);
        if x != TEXT_UNREAD {
            return x;
        }
        let doc = t.doc[r as usize];
        let local = r - t.base[doc as usize];
        let x = match t.value_docs.partition_point(|&d| d <= doc) {
            0 => self.b.text_at(doc as usize, local),
            i => t.values[i - 1].upgrade().and_then(|v| v.text_at((doc - t.value_docs[i - 1]) as usize, local)),
        };
        let x = x.unwrap_or(NIL);
        at.store(x, Relaxed);
        x
    }

    /// Node `n`'s row, `NONE` where the batch holds none.
    pub(super) fn row_of(&self, n: i64) -> u32 {
        let mut memo = self.memo.get();
        let r = self.b.row_of(n, &mut memo);
        self.memo.set(memo);
        if let Some(r) = r {
            return r;
        }
        // A tape value's node: its batch's row, past the rows before it.
        for (v, &at) in self.t.values.iter().zip(&self.t.value_rows) {
            let mut memo = crate::code_rule_dsl::facts::vtab::NO_MEMO;
            if let Some(r) = v.upgrade().and_then(|v| v.row_of(n, &mut memo)) {
                return r + at;
            }
        }
        NONE
    }

    /// The rows of `rel`'s rows' column `col`, as a mark by row.
    fn mark(&self, rows: &Flat, col: usize) -> Vec<bool> {
        let mut m = vec![false; self.t.ids.len()];
        for r in rows.rows() {
            let at = self.row_of(r[col]);
            if at != NONE {
                m[at as usize] = true;
            }
        }
        m
    }
}

/// SPIKE: with `RERAIL_SPIKE_WHY`, what the chunk's tree and held relations took, when the batch lets it go.
impl Drop for Tree {
    fn drop(&mut self) {
        // A tree made again over more values hands its held relations on: the last one reports them.
        if std::env::var_os("RERAIL_SPIKE_WHY").is_none() || Arc::strong_count(&self.held) > 1 {
            return;
        }
        let arrays = 8 * (self.ids.capacity() + self.file.capacity() + self.texts.get().map_or(0, Vec::capacity))
            + 4 * (self.kinds.capacity() + self.doc.capacity() + self.base.capacity())
            + self.shape.get().map_or(0, Shape::bytes)
            + self.spans.get().map_or(0, |(a, b)| 4 * (a.capacity() + b.capacity()));
        let near = self.near_by.lock().unwrap().values().map(|v| v.capacity() * 4).sum::<usize>() + self.nearest.lock().unwrap().values().map(|v| v.capacity() * 4).sum::<usize>();
        let kinds = self.by_kind.get().map_or(0, |m| m.values().map(|v| v.capacity() * 4).sum());
        let held = self.held.lock().unwrap();
        let mut rels: Vec<(usize, &str)> = held.iter().map(|((_, r), h)| (h.bytes(), r.as_str())).collect();
        rels.sort_unstable_by(|a, b| b.cmp(a));
        let total: usize = rels.iter().map(|r| r.0).sum();
        let top: Vec<String> = rels.iter().take(6).map(|(b, r)| format!("{r} {}k", b / 1024)).collect();
        eprintln!(
            "SPIKE MEM {} rows: tree {}k, nearest {}k, by kind {}k, held {} relations {}k ({})",
            self.ids.len(),
            arrays / 1024,
            near / 1024,
            kinds / 1024,
            rels.len(),
            total / 1024,
            top.join(", ")
        );
    }
}

impl Tree {
    /// The batch's tree, read off its projections: each node's id, kind and document (its shape, spans and texts
    /// are read at their first readers).
    pub fn of(b: &Batch, values: Vec<Arc<Batch>>, before: Option<&Tree>) -> Tree {
        let _span = super::super::profile::span(|| "spike: the batch's tree".into());
        let mut a = b.tree_core();
        let (mut value_rows, mut value_docs) = (Vec::new(), Vec::new());
        for v in &values {
            let (rows, docs) = (a.ids.len() as u32, a.base.len() as u32);
            value_rows.push(rows);
            value_docs.push(docs);
            let more = v.tree_core();
            a.ids.extend(more.ids);
            a.kind.extend(more.kind);
            a.doc.extend(more.doc.iter().map(|&d| d + docs));
            a.base.extend(more.base.iter().map(|&r| r + rows));
            a.file.extend(more.file);
        }
        // A kind atom is `tag * 1024 + code` (`core::kind_atom`), its name's canonical code's, as the `kind` view
        // gives it: the same atom a rule's constant of the name is.
        let kind_name = |k: i64| {
            let k = u32::try_from(k).ok()?;
            let tag = crate::code_rule_dsl::facts::core::kind_tag(k);
            crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().kind_name(crate::code_rule_dsl::facts::core::kind_code(k)).map(str::to_owned)
        };
        let atoms = by_name(a.kind.iter().map(|&k| i64::from(k)), kind_name);
        Tree {
            ids: a.ids,
            kinds: a.kind,
            doc: a.doc,
            base: a.base,
            file: a.file,
            shape: std::sync::OnceLock::new(),
            spans: std::sync::OnceLock::new(),
            atoms,
            fields: std::sync::OnceLock::new(),
            nearest: Mutex::default(),
            near_by: Mutex::default(),
            by_kind: std::sync::OnceLock::new(),
            texts: std::sync::OnceLock::new(),
            values: values.iter().map(Arc::downgrade).collect(),
            value_rows,
            value_docs,
            held: before.map_or_else(Arc::default, |t| Arc::clone(&t.held)),
            aliases: before.map_or_else(Arc::default, |t| Arc::clone(&t.aliases)),
        }
    }

    /// The shape, made by its first reader (`Chunk::shape`).
    pub(super) fn sh(&self) -> &Shape {
        self.shape.get().expect("the tree's shape is made before it is read (`Chunk::shape`)")
    }

    fn atom(&self, kind: &str) -> i64 {
        self.atoms.get(kind).copied().unwrap_or(NIL)
    }

    /// A field's atom by its name, among the shape's fields (`Chunk::shape` made it).
    fn field_atom(&self, field: &str) -> i64 {
        let fields = self.fields.get_or_init(|| {
            // A field atom is `tag << 16 | code` (`syntax::field_atom`): named by its dialect's catalog.
            let field_name = |f: i64| {
                let (tag, code) = (u8::try_from(f >> 16).ok()?, (f & 0xffff) as u16);
                crate::code_rule_dsl::language::from_tape_tag(tag)?.kinds().field_name(code).map(str::to_owned)
            };
            by_name(self.sh().field.iter().copied().filter(|&f| f != NIL), field_name)
        });
        fields.get(field).copied().unwrap_or(NIL)
    }

    fn set(&self, kinds: &[&str]) -> HashSet<i64> {
        kinds.iter().map(|k| self.atom(k)).collect()
    }

    fn rows(&self) -> std::ops::Range<u32> {
        0..self.ids.len() as u32
    }

    fn id(&self, r: u32) -> i64 {
        self.ids[r as usize]
    }

    #[inline]
    pub(super) fn kind(&self, r: u32) -> i64 {
        if r == NONE { NIL } else { i64::from(self.kinds[r as usize]) }
    }

    pub(super) fn kids(&self, r: u32) -> &[u32] {
        self.sh().kids(r)
    }

    fn in_field(&self, r: u32, field: i64) -> bool {
        self.sh().in_field(r, field)
    }

    fn first_child(&self, r: u32) -> Option<u32> {
        self.sh().first_child(r)
    }

    fn field_child(&self, r: u32, field: i64) -> Option<u32> {
        self.sh().field_child(r, field)
    }

    fn each_below(&self, r: u32, f: &mut impl FnMut(u32)) {
        self.sh().each_below(r, f)
    }

    pub(super) fn within(&self, n: u32, a: u32) -> bool {
        self.sh().within(n, a)
    }

    /// Each row's nearest strict ancestor of a kind in `kinds` (the set named `name`), computed once for the batch
    /// in one pass, parents before children (W's enclosing scope, kept as it walks).
    fn nearest(&self, name: &'static str, kinds: &[&str]) -> Arc<Vec<u32>> {
        if let Some(n) = self.nearest.lock().unwrap().get(name) {
            return Arc::clone(n);
        }
        let set = self.set(kinds);
        let n = self.ids.len();
        let mut near = vec![NONE; n];
        let mut done = vec![false; n];
        let mut chain = Vec::new();
        for r in 0..n as u32 {
            let mut at = r;
            while at != NONE && !done[at as usize] {
                chain.push(at);
                at = self.sh().parent[at as usize];
            }
            for c in chain.drain(..).rev() {
                let p = self.sh().parent[c as usize];
                near[c as usize] = if p == NONE { NONE } else if set.contains(&self.kind(p)) { p } else { near[p as usize] };
                done[c as usize] = true;
            }
        }
        let near = Arc::new(near);
        self.nearest.lock().unwrap().insert(name, Arc::clone(&near));
        near
    }

    /// `n`'s kind and each ancestor's, with the field it is in (check mode's account of a row).
    pub fn describe(&self, c: &Chunk<'_>, n: i64) -> String {
        let name = |k: i64| self.atoms.iter().find(|&(_, &a)| a == k).map_or("?".to_owned(), |(s, _)| s.clone());
        let _ = c.shape();
        let _ = self.field_atom("");
        let fname = |f: i64| self.fields.get().into_iter().flatten().find(|&(_, &a)| a == f).map_or("-".to_owned(), |(s, _)| s.clone());
        let mut out = Vec::new();
        let mut at = c.row_of(n);
        if at == NONE {
            let total = c.b.nodes().total();
            return format!("not the batch's (tree {} rows, batch {total})", self.ids.len());
        }
        let sh = c.shape();
        while at != NONE && out.len() < 5 {
            out.push(format!("{}[{}]", name(self.kind(at)), fname(sh.field[at as usize])));
            at = sh.parent[at as usize];
        }
        out.join(" < ")
    }
}

/// Each name of `values`' atoms, to the first atom of it.
fn by_name(values: impl Iterator<Item = i64>, name: impl Fn(i64) -> Option<String>) -> HashMap<String, i64> {
    let mut distinct: Vec<i64> = values.collect::<HashSet<i64>>().into_iter().collect();
    distinct.sort_unstable();
    let mut out = HashMap::default();
    for a in distinct {
        if let Some(n) = name(a) {
            out.entry(n).or_insert(a);
        }
    }
    out
}

/// Every row of `source`, flat.
pub fn rows(source: &dyn Source) -> Flat {
    let k = source.columns().len();
    let all: Vec<usize> = (0..k).collect();
    let mut out = Flat { k, v: Vec::with_capacity(source.len() * k) };
    source.for_rows(&all, &mut |_, row| out.v.extend_from_slice(row));
    out
}

/// A stratum's inputs, read by name: its rows, or none where the relation is not held (no rows).
pub trait Inputs {
    fn read(&self, rel: &str) -> Flat;
    /// The texts of symbol atoms.
    fn texts(&self, atoms: &[i64]) -> Vec<String>;
}

const SCOPE: [&str; 4] = ["module", "function_definition", "class_definition", "lambda"];
const COMP: [&str; 4] = ["list_comprehension", "set_comprehension", "dictionary_comprehension", "generator_expression"];

/// The values paired with `k` in sorted pairs.
fn paired(pairs: &[[i64; 2]], k: i64) -> impl Iterator<Item = i64> + '_ {
    let from = pairs.partition_point(|p| p[0] < k);
    pairs[from..].iter().take_while(move |p| p[0] == k).map(|p| p[1])
}

/// `nameref`: identifiers that are not an attribute's or a definition's name, a keyword pattern's key, nor a dotted
/// pattern's tail; with their text.
fn nameref(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let id = t.atom("identifier");
    let (kw, tail) = (c.mark(&inp.read("kwpat_key"), 0), c.mark(&inp.read("pat_tail"), 0));
    let (attribute, name) = (t.field_atom("attribute"), t.field_atom("name"));
    let mut out = Flat::new(2);
    for r in t.rows() {
        let u = r as usize;
        if t.kind(r) == id && c.text(r) != NIL && !kw[u] && !tail[u] && !t.in_field(r, attribute) && !t.in_field(r, name) {
            out.push(&[t.id(r), c.text(r)]);
        }
    }
    out
}

/// `wstart`: where a name's lookup starts (the five rules of `names.rerail`).
fn wstart(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let lscope_kinds: Vec<&str> = SCOPE.iter().chain(COMP.iter()).copied().collect();
    let near = t.nearest("lscope", &lscope_kinds);
    let (scope, comp) = (t.set(&SCOPE), t.set(&COMP));
    let def_outside = inp.read("def_outside");
    let in_first_iter = inp.read("in_first_iter");
    let outside = c.mark(&def_outside, 0);
    let first: Vec<[i64; 2]> = in_first_iter.sorted();
    let host: Vec<[i64; 2]> = inp.read("host_node").sorted();
    let ann: Vec<[i64; 2]> = inp.read("ann_start").sorted();
    let nameref = inp.read("nameref");
    let mut out = Flat::new(2);
    for r in nameref.rows() {
        let n = r[0];
        let at = c.row_of(n);
        if at != NONE && near[at as usize] != NONE {
            let s = near[at as usize];
            let k = t.kind(s);
            if scope.contains(&k) && !outside[at as usize] {
                out.push(&[n, t.id(s)]);
            }
            if comp.contains(&k) && first.binary_search(&[n, t.id(s)]).is_err() {
                out.push(&[n, t.id(s)]);
            }
        }
        for h in paired(&host, n) {
            for s in paired(&ann, h) {
                out.push(&[n, s]);
            }
        }
    }
    for r in def_outside.rows().chain(in_first_iter.rows()) {
        let f = c.row_of(r[1]);
        if f != NONE && near[f as usize] != NONE {
            out.push(&[r[0], t.id(near[f as usize])]);
        }
    }
    out
}

/// `look` (and `walk`, which only `look` reads, in this stratum: not made): each name's scope chain walked from its
/// starts until a scope binds it (W's resolve). Per name, the scopes walked are a few: kept in a list for the name.
fn look_walk(c: &Chunk<'_>, inp: &dyn Inputs) -> Vec<(String, Flat)> {
    let t = c.t;
    let lscope = t.set(&SCOPE.iter().chain(COMP.iter()).copied().collect::<Vec<_>>());
    let lf_kinds: Vec<&str> = ["module", "function_definition", "lambda"].iter().chain(COMP.iter()).copied().collect();
    let lf = t.nearest("lfscope", &lf_kinds);
    // `scope_up(s, p) :- kind(s, k), lscope(k), nearest(s, p, lfscope)` (folded into its reader: no relation of its own).
    let scope_up = |s: i64| {
        let r = c.row_of(s);
        (r != NONE && lscope.contains(&t.kind(r)) && lf[r as usize] != NONE).then(|| t.id(lf[r as usize]))
    };
    let nameref: Vec<[i64; 2]> = inp.read("nameref").sorted();
    let tp_ref: Vec<[i64; 2]> = inp.read("tp_ref").sorted();
    // `bound_in(s, x)`, `gbefore(n, x, s)`, `cls_late(n, x, s)`: sorted, searched.
    let bound: Vec<[i64; 2]> = inp.read("bound_in").sorted();
    let gbefore: Vec<[i64; 3]> = inp.read("gbefore").sorted();
    let late: Vec<[i64; 3]> = inp.read("cls_late").sorted();
    let mut starts: Vec<[i64; 2]> = inp.read("wstart").sorted();
    starts.dedup();
    let mut look = Flat::new(3);
    let mut walked: Vec<(i64, i64)> = Vec::new();
    let mut work: Vec<(i64, i64)> = Vec::new();
    let mut i = 0;
    while i < starts.len() {
        let n = starts[i][0];
        let j = i + starts[i..].partition_point(|s| s[0] == n);
        walked.clear();
        for x in paired(&nameref, n) {
            if tp_ref.binary_search(&[n, x]).is_ok() {
                continue;
            }
            for s in &starts[i..j] {
                if !walked.contains(&(x, s[1])) {
                    walked.push((x, s[1]));
                    work.push((x, s[1]));
                }
            }
        }
        while let Some((x, s)) = work.pop() {
            let b = bound.binary_search(&[s, x]).is_ok();
            let l = late.binary_search(&[n, x, s]).is_ok();
            let g = gbefore.binary_search(&[n, x, s]).is_ok();
            if (b && !l) || (!b && g) {
                look.push(&[n, x, s]);
            }
            if ((!b && !g) || l)
                && let Some(p) = scope_up(s)
                && !walked.contains(&(x, p))
            {
                walked.push((x, p));
                work.push((x, p));
            }
        }
        i = j;
    }
    vec![("look".into(), look)]
}

/// The function whose body holds row `n`, through the functions of `fn_in` where `n` is in a definition's head.
/// `rf_body(g, b) :- kind(g, "function_definition"), field(g, "body", b)` is the tree's (no relation of its own).
fn enclosing(t: &Tree, n: u32, near: &[u32], body: i64, fn_in: &HashMap<u32, Vec<u32>>, out: &mut Vec<u32>) {
    let g = near[n as usize];
    if g == NONE {
        return;
    }
    if let Some(b) = t.field_child(g, body) {
        if t.within(n, b) {
            out.push(g);
        } else {
            out.extend(fn_in.get(&g).into_iter().flatten().copied());
        }
    }
}

/// `rf_fn_in` by row: each function's enclosing functions, outermost first (so a nested one finds its parent's).
fn fn_in_rows(c: &Chunk<'_>, inp: &dyn Inputs, near: &[u32], body: i64) -> HashMap<u32, Vec<u32>> {
    let t = c.t;
    let fdef = t.atom("function_definition");
    let mut fns: Vec<u32> = inp.read("rf_def").rows().map(|r| c.row_of(r[0])).filter(|&g| g != NONE && t.kind(g) == fdef).collect();
    fns.sort_unstable_by_key(|&g| (t.sh().depth[g as usize], g));
    fns.dedup();
    let mut fn_in: HashMap<u32, Vec<u32>> = HashMap::default();
    let mut found = Vec::new();
    for g in fns {
        found.clear();
        enclosing(t, g, near, body, &fn_in, &mut found);
        found.sort_unstable();
        found.dedup();
        if !found.is_empty() {
            fn_in.insert(g, found.clone());
        }
    }
    fn_in
}

/// `rf_fn_in`: each function's enclosing function.
fn rf_fn_in(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let near = t.nearest("fdef", &["function_definition"]);
    let fn_in = fn_in_rows(c, inp, &near, t.field_atom("body"));
    let mut out = Flat::new(2);
    for (g, hs) in fn_in {
        for h in hs {
            out.push(&[t.id(g), t.id(h)]);
        }
    }
    out
}

/// `rf_in`: each site's enclosing function.
fn rf_in(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let near = t.nearest("fdef", &["function_definition"]);
    let body = t.field_atom("body");
    let mut fn_in: HashMap<u32, Vec<u32>> = HashMap::default();
    for r in inp.read("rf_fn_in").rows() {
        let (g, h) = (c.row_of(r[0]), c.row_of(r[1]));
        if g != NONE && h != NONE {
            fn_in.entry(g).or_default().push(h);
        }
    }
    let sites = c.mark(&inp.read("rf_site"), 0);
    let mut out = Flat::new(2);
    let mut found = Vec::new();
    for n in t.rows().filter(|&r| sites[r as usize]) {
        found.clear();
        enclosing(t, n, &near, body, &fn_in, &mut found);
        found.sort_unstable();
        found.dedup();
        for &g in &found {
            out.push(&[t.id(n), t.id(g)]);
        }
    }
    out
}

/// `bind_only`: names a statement binds without reading them (the eight rules of `names.rerail`).
fn bind_only(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let id = t.atom("identifier");
    let is_id = |r: u32| t.kind(r) == id;
    let mut out = vec![false; t.ids.len()];
    let aug = c.mark(&inp.read("aug_target"), 0);
    for r in inp.read("tgt").rows() {
        let at = c.row_of(r[0]);
        if at != NONE && is_id(at) && !aug[at as usize] {
            out[at as usize] = true;
        }
    }
    for r in inp.read("tp_site").rows() {
        let at = c.row_of(r[1]);
        if at != NONE {
            out[at as usize] = true;
        }
    }
    let alias_read = c.mark(&inp.read("tp_alias_read"), 0);
    let (assignment, alias, typed) = (t.atom("assignment"), t.atom("type_alias_statement"), t.atom("typed_parameter"));
    let (left, parameters) = (t.field_atom("left"), t.field_atom("parameters"));
    let imports = t.set(&["import_statement", "import_from_statement", "future_import_statement"]);
    let globals = t.set(&["global_statement", "nonlocal_statement"]);
    for r in t.rows() {
        let p = t.sh().parent[r as usize];
        if p != NONE && t.in_field(r, left) {
            if t.kind(p) == assignment && is_id(r) {
                out[r as usize] = true;
            }
            if t.kind(p) == alias {
                t.each_below(r, &mut |i| {
                    if is_id(i) && !alias_read[i as usize] {
                        out[i as usize] = true;
                    }
                });
            }
        }
        if p != NONE && t.in_field(r, parameters) {
            for &q in t.kids(r) {
                if is_id(q) {
                    out[q as usize] = true;
                }
                if let Some(first) = t.first_child(q) {
                    if is_id(first) {
                        out[first as usize] = true;
                    }
                    if t.kind(q) == typed
                        && let Some(id0) = t.first_child(first)
                        && is_id(id0)
                    {
                        out[id0 as usize] = true;
                    }
                }
            }
        }
        let k = t.kind(r);
        if imports.contains(&k) {
            t.each_below(r, &mut |i| {
                if is_id(i) {
                    out[i as usize] = true;
                }
            });
        }
        if globals.contains(&k) {
            for &g in t.kids(r) {
                if is_id(g) {
                    out[g as usize] = true;
                }
            }
        }
    }
    Flat { k: 1, v: t.rows().filter(|&r| out[r as usize]).map(|r| t.id(r)).collect() }
}

/// `rf_site`: calls, attributes of a name, and read names, where none is a call's function.
fn rf_site(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let (id, call, attribute) = (t.atom("identifier"), t.atom("call"), t.atom("attribute"));
    let (function, object) = (t.field_atom("function"), t.field_atom("object"));
    // `rf_read(i) :- nameref(i, _), !bind_only(i), !rf_deleted(i), !guest_of(i, _)` (no relation of its own).
    let names = c.mark(&inp.read("nameref"), 0);
    let bound = c.mark(&inp.read("bind_only"), 0);
    // `rf_deleted(n) :- del_unbinds(_, n)` (one rule: folded into its readers, not stored).
    let deleted = c.mark(&inp.read("del_unbinds"), 1);
    let guest = c.mark(&inp.read("guest_of"), 0);
    let objects = c.mark(&inp.read("rf_object"), 0);
    let v = t
        .rows()
        .filter(|&n| {
            let (k, u) = (t.kind(n), n as usize);
            if k == call {
                true
            } else if k == attribute {
                !t.in_field(n, function) && t.field_child(n, object).is_some_and(|o| t.kind(o) == id)
            } else if k == id {
                !t.in_field(n, function) && names[u] && !bound[u] && !deleted[u] && !guest[u] && !objects[u]
            } else {
                false
            }
        })
        .map(|n| t.id(n))
        .collect();
    Flat { k: 1, v }
}

/// `rf_selfish`: the names `self` and `cls`.
fn rf_selfish(c: &Chunk<'_>, inp: &dyn Inputs) -> Flat {
    let t = c.t;
    let id = t.atom("identifier");
    let atoms: Vec<i64> = t.rows().filter(|&r| t.kind(r) == id && c.text(r) != NIL).map(|r| c.text(r)).collect::<HashSet<_>>().into_iter().collect();
    let strings = inp.texts(&atoms);
    let selfish: HashSet<i64> = atoms.iter().zip(&strings).filter(|(_, s)| *s == "self" || *s == "cls").map(|(a, _)| *a).collect();
    Flat { k: 1, v: t.rows().filter(|&r| t.kind(r) == id && selfish.contains(&c.text(r))).map(|r| t.id(r)).collect() }
}

/// The stratum's rows by relation, by W's algorithm.
pub fn run(rels: &[&str], c: &Chunk<'_>, inp: &dyn Inputs) -> Vec<(String, Flat)> {
    let _ = c.shape();
    let mut sorted = rels.to_vec();
    sorted.sort_unstable();
    let one = |rel: &str, rows: Flat| vec![(rel.to_owned(), rows)];
    match sorted.as_slice() {
        ["look", "walk"] => look_walk(c, inp),
        ["wstart"] => one("wstart", wstart(c, inp)),
        ["rf_in"] => one("rf_in", rf_in(c, inp)),
        ["rf_fn_in"] => one("rf_fn_in", rf_fn_in(c, inp)),
        ["nameref"] => one("nameref", nameref(c, inp)),
        ["bind_only"] => one("bind_only", bind_only(c, inp)),
        ["rf_site"] => one("rf_site", rf_site(c, inp)),
        ["rf_selfish"] => one("rf_selfish", rf_selfish(c, inp)),
        _ => Vec::new(),
    }
}

/// `SPIKE CHECK` lines: the spike's rows against the generic joins'.
pub fn compare(rel: &str, spike: &Flat, generic: &Flat, chunk: Option<&Chunk<'_>>) {
    let a: HashSet<&[i64]> = spike.rows().collect();
    let b: HashSet<&[i64]> = generic.rows().collect();
    let only_spike: Vec<_> = a.difference(&b).take(3).collect();
    let only_generic: Vec<_> = b.difference(&a).take(3).collect();
    let (ns, ng) = (a.difference(&b).count(), b.difference(&a).count());
    if let Some(c) = chunk {
        for r in only_spike.iter().chain(&only_generic) {
            eprintln!("SPIKE WHY {rel} {r:?}: {}", c.t.describe(c, r[0]));
        }
    }
    eprintln!("SPIKE CHECK {rel}: spike {} generic {} only-spike {ns} {only_spike:?} only-generic {ng} {only_generic:?}", a.len(), b.len());
}
