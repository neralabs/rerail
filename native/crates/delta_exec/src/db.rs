//! A level's facts: its relations, each a version and at most one delta of it (decision 0678's constraints: two
//! states, never more), and its documents, each a source and its tape at two states. All of it lies in one block,
//! as `blocks` lays relations out; nothing here is a second copy a reader reads instead.
//!
//! A run writes each relation's delta and leaves its version alone, so the relation's bistate (version → version and
//! delta) is the run's change, read by the next level. `settle` then rolls each delta into a new version, the
//! roll-up of the parent page's trace (an LSM merge): the old version and delta are no longer read.

use rerail_blocks::bistate::{Bistate, ColumnIndex, Read, RowId};
use rerail_blocks::block::{Block, kind};
use rerail_blocks::relation::{self, RelW, Relation, Spec, View};
use rerail_blocks::table::Table;
use rerail_tape_format::f16::Tape16;
use rustc_hash::FxHashMap;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

/// A relation's states: its version, and the delta a run wrote, when it wrote one. Both lie in the level's block.
pub struct Rel {
    pub arity: usize,
    spec: Spec,
    version: NonNull<Relation>,
    delta: Option<NonNull<Relation>>,
}

/// One relation's spec: 8-byte columns, sorted on column 0, a hashed index on every column (a head row found by its
/// values, a count by its key).
pub fn spec(arity: usize) -> Spec {
    Spec { widths: vec![8; arity], sort: Some(0), indexes: vec![(0..arity as u16).collect()], min_log: rerail_blocks::rows::MIN_LOG }
}

/// A document's text: its source, loaded from disk, and its tape, made by the rr parser.
pub struct Text {
    // copy: load (the file's bytes, which the tape's spans are read from)
    pub src: String,
    pub tape: Tape16,
}

impl Text {
    pub fn parse(src: String) -> Result<Text, String> {
        let tape = rerail_rr_syntax::tape(&src)?;
        Ok(Text { src, tape })
    }
}

/// A document: its path (a symbol) and its text at two states (`None`: absent). A changed document's two texts are
/// two tapes whose node ids say nothing of each other (L9): the file is the grain.
pub struct Doc {
    pub path: i64,
    pub before: Option<Arc<Text>>,
    pub after: Option<Arc<Text>>,
    /// Given by the level's caller, not asked for by a `load`.
    pub given: bool,
    /// Asked for by a `load` in this run.
    pub asked: bool,
}

impl Doc {
    pub fn changed(&self) -> bool {
        match (&self.before, &self.after) {
            (Some(a), Some(b)) => !Arc::ptr_eq(a, b),
            (None, None) => false,
            _ => true,
        }
    }
}

/// A level's facts.
pub struct Db {
    block: Box<Block>,
    lock: Mutex<()>,
    rels: FxHashMap<i64, Rel>,
    /// Each document by its id less one: `d` is `docs[d - 1]`.
    pub docs: Vec<Doc>,
    /// The heads whose derivation counts are kept (`{head}#counts`), so a run changes them by the rules' terms.
    pub counted: rustc_hash::FxHashSet<i64>,
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
        Db { block: Box::new(Block::anon(kind::FACTS, 1 << 30)), lock: Mutex::new(()), rels: FxHashMap::default(), docs: Vec::new(), counted: Default::default() }
    }

    fn get(&self, p: NonNull<Relation>) -> &Relation {
        // SAFETY: see `Send`: a descriptor in this Db's block, alive while it is.
        unsafe { p.as_ref() }
    }

    /// Relation `name` of `arity`, made empty when it has none.
    pub fn declare(&mut self, name: i64, arity: usize) -> &Rel {
        self.declare_as(name, spec(arity))
    }

    /// A head's derivation counts: its row and a count, indexed on the row (`change::State`'s `by_key` 0).
    pub fn declare_counts(&mut self, name: i64, arity: usize) -> &Rel {
        let mut s = spec(arity + 1);
        s.indexes = vec![(0..arity as u16).collect()];
        self.declare_as(name, s)
    }

    fn declare_as(&mut self, name: i64, spec: Spec) -> &Rel {
        let arity = spec.widths.len();
        if !self.rels.contains_key(&name) {
            let r = Relation::create(&self.block, &spec, None);
            relation::freeze(&self.block, r, &spec, None);
            let version = NonNull::from(r);
            self.rels.insert(name, Rel { arity, spec, version, delta: None });
        }
        let r = &self.rels[&name];
        assert_eq!(r.arity, arity, "relation {name}: one arity");
        r
    }

    pub fn has(&self, name: i64) -> bool {
        self.rels.contains_key(&name)
    }

    pub fn arity(&self, name: i64) -> Option<usize> {
        self.rels.get(&name).map(|r| r.arity)
    }

    /// Its relations' names.
    pub fn names(&self) -> impl Iterator<Item = i64> + '_ {
        self.rels.keys().copied()
    }

    /// Relation `name` before the run: its version.
    pub fn before(&self, name: i64) -> View<'_> {
        View::full(self.get(self.rels[&name].version))
    }

    /// Relation `name` after the run: its version and its delta.
    pub fn after(&self, name: i64) -> View<'_> {
        let r = &self.rels[&name];
        match r.delta {
            Some(d) => View { base: Some(self.get(r.version)), top: self.get(d) },
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
        f(&RelW { block: &self.block, rel: d, lock: &self.lock }, &View::full(version));
        relation::freeze(&self.block, d, &r.spec, Some(version));
        let d = NonNull::from(d);
        self.rels.get_mut(&name).expect("declared").delta = Some(d);
    }

    /// Sets relation `name`'s state after the run to exactly `rows`: its version's rows not among them killed, the
    /// rest pushed. A run's reference strategy (run the after side, diff with the version) ends here.
    pub fn write_set(&mut self, name: i64, rows: &rustc_hash::FxHashSet<Box<[i64]>>) {
        let arity = self.rels[&name].arity;
        self.write(name, |w, version| {
            let mut kept = rustc_hash::FxHashSet::default();
            let mut row = vec![0i64; arity];
            for id in version.live_ids() {
                read_row(version, id, &mut row);
                if rows.contains(&row[..]) {
                    kept.insert(row.clone().into_boxed_slice());
                } else {
                    w.kill(id.get());
                }
            }
            for r in rows {
                if !kept.contains(r) {
                    w.push(r);
                }
            }
        });
    }

    /// Rolls every relation's delta into a new version, and every document's text after into its text before: the
    /// state the next run starts from.
    pub fn settle(&mut self) {
        let names: Vec<i64> = self.rels.iter().filter(|(_, r)| r.delta.is_some()).map(|(&n, _)| n).collect();
        for name in names {
            let after = self.after(name);
            let r = &self.rels[&name];
            let v = Relation::create(&self.block, &r.spec, None);
            let w = RelW { block: &self.block, rel: v, lock: &self.lock };
            let mut row = vec![0i64; r.arity];
            for id in after.live_ids() {
                read_row(&after, id, &mut row);
                // copy: derived (the roll-up: the state's rows written once into its new version; the old version and
                // delta are not read again)
                w.push(&row);
            }
            relation::freeze(&self.block, v, &r.spec, None);
            let r = self.rels.get_mut(&name).expect("declared");
            r.version = NonNull::from(v);
            r.delta = None;
        }
        for d in &mut self.docs {
            d.before = d.after.clone();
            d.asked = false;
        }
    }

    /// A document given by the level's caller: its id.
    pub fn give(&mut self, path: i64, text: Arc<Text>) -> i64 {
        match self.doc_of(path) {
            Some(d) => {
                let doc = &mut self.docs[d as usize - 1];
                doc.after = Some(text);
                doc.given = true;
                d
            }
            None => {
                self.docs.push(Doc { path, before: None, after: Some(text), given: true, asked: false });
                self.docs.len() as i64
            }
        }
    }

    /// The document of `path`, when it has one.
    pub fn doc_of(&self, path: i64) -> Option<i64> {
        self.docs.iter().position(|d| d.path == path).map(|i| i as i64 + 1)
    }

    /// An open relation of `arity` in the level's block: a fixpoint's rows as it finds them, read by range.
    pub fn scratch(&self, arity: usize) -> NonNull<Relation> {
        NonNull::from(Relation::create(&self.block, &spec(arity), None))
    }

    /// An open relation `scratch` made.
    pub fn open(&self, r: NonNull<Relation>) -> &Relation {
        self.get(r)
    }

    /// Sets relation `name`'s state after the run to open relation `r`'s rows: frozen, then each of the version's
    /// rows `r` lacks killed and each of `r`'s rows the version lacks pushed, each found by a probe.
    pub fn write_diff(&mut self, name: i64, r: NonNull<Relation>) {
        let arity = self.rels[&name].arity;
        let spec = spec(arity);
        relation::freeze(&self.block, self.get(r), &spec, None);
        // SAFETY: see `Send`: a descriptor in this Db's block, alive while it is.
        let new = View::full(unsafe { r.as_ref() });
        let cols: Vec<ColumnIndex> = (0..arity).map(|c| ColumnIndex::new(c as u16)).collect();
        self.write(name, |w, version| {
            let mut row = vec![0i64; arity];
            // `probe` is false when its `each` stopped it: a row found.
            let holds = |t: &View<'_>, row: &[i64]| !Table::probe(t, &cols, row, &mut |_| false);
            for id in version.live_ids() {
                read_row(version, id, &mut row);
                if !holds(&new, &row) {
                    w.kill(id.get());
                }
            }
            for id in new.live_ids() {
                read_row(&new, id, &mut row);
                if !holds(version, &row) {
                    w.push(&row);
                }
            }
        });
    }

    /// Sets relation `name`'s state after the run to the keys of counts relation `counts` after it (its columns but
    /// the last): each found by a probe, as `write_diff`.
    pub fn write_keys(&mut self, name: i64, counts: i64) {
        let arity = self.rels[&name].arity;
        let cols: Vec<ColumnIndex> = (0..arity).map(|c| ColumnIndex::new(c as u16)).collect();
        let r = &self.rels[&counts];
        // SAFETY: see `Send`: descriptors in this Db's block, alive while it is.
        let keys = match r.delta {
            Some(d) => View { base: Some(unsafe { r.version.as_ref() }), top: unsafe { d.as_ref() } },
            None => View::full(unsafe { r.version.as_ref() }),
        };
        self.write(name, |w, version| {
            let mut row = vec![0i64; arity + 1];
            for id in version.live_ids() {
                read_row(version, id, &mut row[..arity]);
                if Table::probe(&keys, &cols, &row[..arity], &mut |_| false) {
                    w.kill(id.get());
                }
            }
            for id in keys.live_ids() {
                read_row(&keys, id, &mut row);
                if Table::probe(version, &cols, &row[..arity], &mut |_| false) {
                    w.push(&row[..arity]);
                }
            }
        });
    }

    /// Adds `row` to open relation `r` unless it holds it.
    pub fn insert_into(&self, r: &Relation, row: &[i64]) -> bool {
        RelW { block: &self.block, rel: r, lock: &self.lock }.insert_unique(row)
    }

    /// Whether any document changed in the run.
    pub fn docs_changed(&self) -> bool {
        self.docs.iter().any(Doc::changed)
    }

    /// The live rows of relation `name` on one side, for tests and checks.
    pub fn rows(&self, name: i64, after: bool) -> std::collections::BTreeSet<Vec<i64>> {
        let v = if after { self.after(name) } else { self.before(name) };
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

/// Row `id`'s values, read into `row`: a chunk the reader reuses (as `Read::fill`'s), not a second store.
pub fn read_row<R: Read>(r: &R, id: RowId, row: &mut [i64]) {
    for (c, v) in row.iter_mut().enumerate() {
        *v = r.get(id, ColumnIndex::new(c as u16));
    }
}
