//! A level's facts: its relations, each a version and at most one delta of it (decision 0678's constraints: two
//! states, never more), and its documents, each a text before and after. The relations lie in one block, as
//! `blocks` lays relations out; nothing here is a second copy a reader reads instead.
//!
//! A run writes each relation's delta and leaves its version alone, so the relation's bistate (version → version and
//! delta) is the run's change, read by the next stratum and the next level. `settle` then rolls each delta into a new
//! version, with the indexes the program asks for.
//!
//! A document's texts are read through the views (`tape`): each text is given a range of view ids when it comes,
//! past every earlier one, and the ranges of texts replaced or dropped in a run are the views' record of their change.

use crate::kform::Program;
use crate::sym::Symbols;
use crate::tape::{DocPart, Names, Reads, TapeView, View as TapeKind};
use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};

use rerail_blocks::block::{Block, kind};
use rerail_blocks::relation::{self, RelW, Relation, Spec, View};
use rerail_blocks::table::Table;
use rerail_tape_format::f16::Tape16;
use rustc_hash::{FxHashMap, FxHashSet};
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

/// A relation's states: its version, and the delta a run wrote, when it wrote one. Both lie in the level's block.
pub struct Rel {
    pub arity: usize,
    spec: Spec,
    version: NonNull<Relation>,
    delta: Option<NonNull<Relation>>,
    /// The columns the program probes it by that its spec has no index on: added at `settle`.
    wanted: Vec<Vec<u16>>,
}

/// One relation's spec: 8-byte columns, unsorted, and a hashed index on every column (a row found by its values).
/// Every other probe's index is asked for by the program (`want`): a probe of a sorted relation by a set of columns
/// with its sort column among them reads that column's equal range, whatever indexes it has (`View::access`), so a
/// relation is sorted on none and each probe has its own index.
pub fn spec(arity: usize) -> Spec {
    Spec {
        widths: vec![8; arity],
        sort: None,
        indexes: vec![(0..arity as u16).collect()],
        min_log: rerail_blocks::rows::MIN_LOG,
    }
}

/// A document's text: its source, loaded from disk, and its tape, made by the rr parser.
pub struct Text {
    // copy: load (the file's bytes, which the tape's spans are read from)
    pub src: String,
    pub tape: Tape16,
}

impl Text {
    /// `src` parsed: refused when a statement does not parse (the parser writes it as an ERROR leaf, and a file the
    /// engine refuses to run still has a tape).
    pub fn parse(src: String) -> Result<Text, String> {
        let tape = rerail_rr_syntax::tape(&src)?;
        let text = Text { src, tape };
        let r = Reads::of(&text);
        if let Some(i) = (0..r.len()).find(|&i| r.node(i).0 == rerail_rr_syntax::tape::ERROR) {
            return Err(format!("a statement that does not parse: {:?}", r.text(i)));
        }
        Ok(text)
    }
}

/// A text of a document, at its range of view ids.
#[derive(Clone)]
pub struct Placed {
    pub text: Arc<Text>,
    pub base: u32,
}

/// A document: its path (a symbol) and its text at two states (`None`: absent). A changed document's two texts are
/// two tapes whose node ids say nothing of each other (L9): the file is the grain.
pub struct Doc {
    pub path: i64,
    pub before: Option<Placed>,
    pub after: Option<Placed>,
    /// Given by the level's caller, not asked for by a `load`.
    pub given: bool,
    /// The world's stamp of the text read (`World::stamp`).
    pub stamp: Option<u64>,
}

impl Doc {
    pub fn changed(&self) -> bool {
        self.before.as_ref().map(|p| p.base) != self.after.as_ref().map(|p| p.base)
    }
}

/// A level's facts.
pub struct Db {
    block: Box<Block>,
    lock: Mutex<()>,
    rels: FxHashMap<i64, Rel>,
    /// Each document by its id less one: `d` is `docs[d - 1]`.
    pub docs: Vec<Doc>,
    /// View ids given out, and given out before this run (the views' before end).
    next: u32,
    start: u32,
    /// The ids of the texts replaced or dropped in this run, ascending once `prepare_views` sorts them.
    retired: Vec<u32>,
    /// The heads whose derivation counts are kept (`{head}#counts`), so a run changes them by the rules' terms.
    pub counted: FxHashSet<i64>,
    /// The cache of the program this level runs (L5): its plans and strata at the program's state after its last
    /// run. `None`: it names no state, so the program is read again.
    pub plans: Option<Program>,
}

// SAFETY: the relations' descriptors lie in `block`, which the Db owns and never moves (an anonymous block is one
// reservation of address space); they are reached only through `&self` or `&mut self`.
unsafe impl Send for Db {}

impl Default for Db {
    fn default() -> Self {
        Self::new()
    }
}

impl Db {
    pub fn new() -> Db {
        Db {
            block: Box::new(Block::anon(kind::FACTS, 1 << 32)),
            lock: Mutex::new(()),
            rels: FxHashMap::default(),
            docs: Vec::new(),
            next: 0,
            start: 0,
            retired: Vec::new(),
            counted: FxHashSet::default(),
            plans: None,
        }
    }

    fn get(&self, p: NonNull<Relation>) -> &Relation {
        // SAFETY: see `Send`: a descriptor in this Db's block, alive while it is.
        unsafe { p.as_ref() }
    }

    // ---- relations ------------------------------------------------------------------------------------------------

    /// Relation `name` of `arity`, made empty when it has none.
    pub fn declare(&mut self, name: i64, arity: usize) {
        if !self.rels.contains_key(&name) {
            self.make(name, spec(arity));
        }
        assert_eq!(self.rels[&name].arity, arity, "relation {name}: one arity");
    }

    /// A head's derivation counts: its row and a count, indexed first on the row (`change::State`'s `by_key` 0).
    pub fn declare_counts(&mut self, name: i64, arity: usize) {
        if !self.rels.contains_key(&name) {
            let mut s = spec(arity + 1);
            s.indexes.insert(0, (0..arity as u16).collect());
            self.make(name, s);
        }
    }

    fn make(&mut self, name: i64, spec: Spec) {
        let r = Relation::create(&self.block, &spec, None);
        relation::freeze(&self.block, r, &spec, None);
        self.rels.insert(
            name,
            Rel {
                arity: spec.widths.len(),
                spec,
                version: NonNull::from(r),
                delta: None,
                wanted: Vec::new(),
            },
        );
    }

    /// The program probes relation `name` by columns `cols`: a hashed index on them, at once when it has no rows and
    /// no delta, else from `settle` on (until then the probe scans).
    pub fn want(&mut self, name: i64, cols: &[u16]) {
        let r = self.rels.get_mut(&name).expect("declared");
        let mut cols = cols.to_vec();
        cols.sort_unstable();
        cols.dedup();
        if cols.is_empty()
            || r.spec.indexes.iter().chain(&r.wanted).any(|k| {
                let mut k = k.clone();
                k.sort_unstable();
                k == cols
            })
        {
            return;
        }
        r.wanted.push(cols);
        // SAFETY: see `Send`.
        if r.delta.is_none() && unsafe { r.version.as_ref() }.rows.len() == 0 {
            let s = self.spec_of(name);
            self.rels.remove(&name);
            self.make(name, s);
        }
    }

    /// Relation `name`'s spec, as its next version's will be.
    pub fn spec_of(&self, name: i64) -> Spec {
        let r = &self.rels[&name];
        let mut s = r.spec.clone();
        s.indexes.extend(r.wanted.iter().cloned());
        s
    }

    pub fn has(&self, name: i64) -> bool {
        self.rels.contains_key(&name)
    }

    pub fn arity(&self, name: i64) -> Option<usize> {
        self.rels.get(&name).map(|r| r.arity)
    }

    /// Relation `name` before the run: its version.
    pub fn before(&self, name: i64) -> View<'_> {
        View::full(self.get(self.rels[&name].version))
    }

    /// Relation `name` after the run: its version and its delta.
    pub fn after(&self, name: i64) -> View<'_> {
        let r = &self.rels[&name];
        match r.delta {
            Some(d) => View {
                base: Some(self.get(r.version)),
                top: self.get(d),
            },
            None => View::full(self.get(r.version)),
        }
    }

    /// Relation `name` across the run.
    pub fn bistate(&self, name: i64) -> Bistate<View<'_>> {
        Bistate::new(self.before(name), self.after(name))
    }

    /// Whether relation `name` changed in the run.
    pub fn changed(&self, name: i64) -> bool {
        self.rels[&name].delta.is_some() && !self.bistate(name).is_identity()
    }

    /// Writes relation `name`'s delta: `f` pushes rows and kills the version's rows (by id), then it is frozen. A run
    /// writes a relation's delta once.
    pub fn write(&mut self, name: i64, f: impl FnOnce(&RelW, &View<'_>)) {
        let r = &self.rels[&name];
        assert!(r.delta.is_none(), "relation {name}: one delta a run");
        let version = self.get(r.version);
        let d = Relation::create(&self.block, &r.spec, Some(version));
        f(
            &RelW {
                block: &self.block,
                rel: d,
                lock: &self.lock,
            },
            &View::full(version),
        );
        relation::freeze(&self.block, d, &r.spec, Some(version));
        let d = NonNull::from(d);
        self.rels.get_mut(&name).expect("declared").delta = Some(d);
    }

    /// Writes the deltas of two relations at once (`write`): a head's counts and the head, each change of a count
    /// written into the head where it is found.
    pub fn write_pair(
        &mut self,
        a: i64,
        b: i64,
        f: impl FnOnce(&RelW, &View<'_>, &RelW, &View<'_>),
    ) {
        assert_ne!(a, b, "two relations");
        let (ra, rb) = (&self.rels[&a], &self.rels[&b]);
        assert!(ra.delta.is_none() && rb.delta.is_none(), "one delta a run");
        let (va, vb) = (self.get(ra.version), self.get(rb.version));
        let (da, db) = (
            Relation::create(&self.block, &ra.spec, Some(va)),
            Relation::create(&self.block, &rb.spec, Some(vb)),
        );
        f(
            &RelW {
                block: &self.block,
                rel: da,
                lock: &self.lock,
            },
            &View::full(va),
            &RelW {
                block: &self.block,
                rel: db,
                lock: &self.lock,
            },
            &View::full(vb),
        );
        relation::freeze(&self.block, da, &ra.spec, Some(va));
        relation::freeze(&self.block, db, &rb.spec, Some(vb));
        let (da, db) = (NonNull::from(da), NonNull::from(db));
        self.rels.get_mut(&a).expect("declared").delta = Some(da);
        self.rels.get_mut(&b).expect("declared").delta = Some(db);
    }

    /// Relation `name`'s rows after the run: none.
    pub fn clear(&mut self, name: i64) {
        self.write(name, |w, version| {
            version.live_ids().for_each(|id| w.kill(id.get()))
        });
    }

    /// Sets relation `name`'s state after the run to the live rows of `new` (a table of its arity): each of the
    /// version's rows `new` lacks killed and each of `new`'s rows the version lacks pushed, each found by a probe.
    pub fn write_diff<T: Table>(&mut self, name: i64, new: &T) {
        let arity = self.rels[&name].arity;
        let cols: Vec<ColumnIndex> = (0..arity).map(|c| ColumnIndex::new(c as u16)).collect();
        self.write(name, |w, version| {
            let mut row = vec![0i64; arity];
            for id in version.live_ids() {
                read_row(version, id, &mut row);
                if lacks(new, &cols, &row) {
                    w.kill(id.get());
                }
            }
            for id in new.live_ids() {
                read_row(new, id, &mut row);
                if lacks(version, &cols, &row) {
                    w.push(&row);
                }
            }
        });
    }

    /// Sets relation `name`'s state after the run to the keys of counts relation `counts` after it (its columns but
    /// the last), as `write_diff`.
    pub fn write_keys(&mut self, name: i64, counts: i64) {
        let arity = self.rels[&name].arity;
        let cols: Vec<ColumnIndex> = (0..arity).map(|c| ColumnIndex::new(c as u16)).collect();
        let r = &self.rels[&counts];
        let (version, delta) = (r.version, r.delta);
        // SAFETY: see `Send`: descriptors in this Db's block, alive while it is.
        let keys = match delta {
            Some(d) => View {
                base: Some(unsafe { version.as_ref() }),
                top: unsafe { d.as_ref() },
            },
            None => View::full(unsafe { version.as_ref() }),
        };
        self.write(name, |w, version| {
            let mut row = vec![0i64; arity + 1];
            for id in version.live_ids() {
                read_row(version, id, &mut row[..arity]);
                if lacks(&keys, &cols, &row[..arity]) {
                    w.kill(id.get());
                }
            }
            for id in keys.live_ids() {
                read_row(&keys, id, &mut row);
                if lacks(version, &cols, &row[..arity]) {
                    w.push(&row[..arity]);
                }
            }
        });
    }

    /// An open relation of relation `name`'s spec in the level's block: a fixpoint's round of rows.
    pub fn scratch(&self, name: i64) -> &Relation {
        Relation::create(&self.block, &self.spec_of(name), None)
    }

    /// Adds `row` to open relation `r` unless it holds it.
    pub fn insert_into(&self, r: &Relation, row: &[i64]) -> bool {
        RelW {
            block: &self.block,
            rel: r,
            lock: &self.lock,
        }
        .insert_unique(row)
    }

    /// Freezes open relation `r` of relation `name`'s spec.
    pub fn freeze(&self, name: i64, r: &Relation) {
        relation::freeze(&self.block, r, &self.spec_of(name), None);
    }

    /// Rolls every relation's delta into a new version with the indexes wanted, and every document's text after into
    /// its text before: the state the next run starts from.
    pub fn settle(&mut self) {
        let names: Vec<i64> = self
            .rels
            .iter()
            .filter(|(_, r)| r.delta.is_some() || !r.wanted.is_empty())
            .map(|(&n, _)| n)
            .collect();
        for name in names {
            let spec = self.spec_of(name);
            let after = self.after(name);
            let v = Relation::create(&self.block, &spec, None);
            let w = RelW {
                block: &self.block,
                rel: v,
                lock: &self.lock,
            };
            let mut row = vec![0i64; spec.widths.len()];
            for id in after.live_ids() {
                read_row(&after, id, &mut row);
                // copy: derived (the roll-up: the state's rows written once into its next version; the old version
                // and delta are not read again)
                w.push(&row);
            }
            relation::freeze(&self.block, v, &spec, None);
            let r = self.rels.get_mut(&name).expect("declared");
            (r.version, r.delta, r.spec) = (NonNull::from(v), None, spec);
            r.wanted.clear();
        }
        for d in &mut self.docs {
            d.before = d.after.clone();
        }
        self.start = self.next;
        self.retired.clear();
    }

    // ---- documents ------------------------------------------------------------------------------------------------

    /// Document `path`'s text after the run: `text` (`None`: absent), given a new range of view ids when it is not the
    /// text it has. Its id.
    pub fn set_doc(
        &mut self,
        path: i64,
        text: Option<Arc<Text>>,
        given: bool,
        stamp: Option<u64>,
    ) -> i64 {
        let d = match self.doc_of(path) {
            Some(d) => d,
            None => {
                self.docs.push(Doc {
                    path,
                    before: None,
                    after: None,
                    given,
                    stamp: None,
                });
                self.docs.len() as i64
            }
        };
        let i = d as usize - 1;
        self.docs[i].given |= given;
        self.docs[i].stamp = stamp;
        let same = match (&self.docs[i].after, &text) {
            (Some(p), Some(t)) => Arc::ptr_eq(&p.text, t) || p.text.src == t.src,
            (None, None) => true,
            _ => false,
        };
        if same {
            return d;
        }
        // Its text after, retired when it is its text before (a text placed in this run and replaced in it was never
        // read before).
        if let Some(old) = self.docs[i].after.take() {
            if old.base < self.start {
                self.retired
                    .extend(old.base..old.base + Reads::of(&old.text).len() as u32);
            }
        }
        self.docs[i].after = text.map(|text| {
            let base = self.next;
            self.next = base
                .checked_add(Reads::of(&text).len() as u32)
                .expect("view ids within u32");
            Placed { text, base }
        });
        d
    }

    /// A document given by the level's caller: its id.
    pub fn give(&mut self, path: i64, text: Arc<Text>) -> i64 {
        self.set_doc(path, Some(text), true, None)
    }

    /// The document of `path`, when it has one.
    pub fn doc_of(&self, path: i64) -> Option<i64> {
        self.docs
            .iter()
            .position(|d| d.path == path)
            .map(|i| i as i64 + 1)
    }

    /// The documents that changed in the run.
    pub fn changed_docs(&self) -> impl Iterator<Item = i64> + '_ {
        self.docs
            .iter()
            .enumerate()
            .filter(|(_, d)| d.changed())
            .map(|(i, _)| i as i64 + 1)
    }

    /// The views' record of the run's change, sorted: done before views are read.
    pub fn prepare_views(&mut self) {
        self.retired.sort_unstable();
        self.retired.dedup();
    }

    /// View `v` over every document, on one side: after with its change since the before state.
    pub fn view<'a>(
        &'a self,
        v: TapeKind,
        after: bool,
        sym: &'a Symbols,
        names: &'a Names,
    ) -> TapeView<'a> {
        let parts = (0..self.docs.len())
            .filter_map(|i| self.part(i, after))
            .collect();
        if after {
            TapeView::new(
                v,
                parts,
                self.next,
                Some((self.start, &self.retired)),
                sym,
                names,
            )
        } else {
            TapeView::new(v, parts, self.start, None, sym, names)
        }
    }

    /// View `v` over document `d` alone, on one side, as a cold run reads it.
    pub fn doc_view<'a>(
        &'a self,
        v: TapeKind,
        d: i64,
        after: bool,
        sym: &'a Symbols,
        names: &'a Names,
    ) -> TapeView<'a> {
        let parts = self.part(d as usize - 1, after).into_iter().collect();
        TapeView::new(
            v,
            parts,
            if after { self.next } else { self.start },
            None,
            sym,
            names,
        )
    }

    fn part(&self, i: usize, after: bool) -> Option<DocPart<'_>> {
        let doc = &self.docs[i];
        let p = if after {
            doc.after.as_ref()
        } else {
            doc.before.as_ref()
        }?;
        Some(DocPart {
            base: p.base,
            d: i as i64 + 1,
            path: doc.path,
            reads: Reads::of(&p.text),
        })
    }

    /// The live rows of relation `name` on one side, for tests and checks.
    pub fn rows(&self, name: i64, after: bool) -> std::collections::BTreeSet<Vec<i64>> {
        if !self.has(name) {
            return Default::default();
        }
        let v = if after {
            self.after(name)
        } else {
            self.before(name)
        };
        let mut row = vec![0i64; self.rels[&name].arity];
        v.live_ids()
            .map(|id| {
                read_row(&v, id, &mut row);
                // copy: not block data (a test's or a check's set of rows)
                row.clone()
            })
            .collect()
    }
}

/// Whether `t` has no live row whose `cols` are `key` (`Table::probe` is false when its `each` stopped it).
fn lacks<T: Table>(t: &T, cols: &[ColumnIndex], key: &[i64]) -> bool {
    Table::probe(t, cols, key, &mut |_| false)
}

/// Row `id`'s values, read into `row`: a chunk the reader reuses (as `Read::fill`'s), not a second store.
pub fn read_row<R: Read + ?Sized>(r: &R, id: RowId, row: &mut [i64]) {
    for (c, v) in row.iter_mut().enumerate() {
        *v = r.get(id, ColumnIndex::new(c as u16));
    }
}

/// Every live row of `t` that agrees with `bound`: by a probe on the bound columns, else every live row.
pub fn each_row<T: Table>(t: &T, bound: &[Option<i64>], each: &mut dyn FnMut(&[i64])) {
    let mut row = vec![0i64; bound.len()];
    let (cols, key): (Vec<ColumnIndex>, Vec<i64>) = bound
        .iter()
        .enumerate()
        .filter_map(|(c, b)| b.map(|b| (ColumnIndex::new(c as u16), b)))
        .unzip();
    if cols.is_empty() {
        for id in t.live_ids() {
            read_row(t, id, &mut row);
            each(&row);
        }
        return;
    }
    t.probe(&cols, &key, &mut |id| {
        read_row(t, id, &mut row);
        each(&row);
        true
    });
}
