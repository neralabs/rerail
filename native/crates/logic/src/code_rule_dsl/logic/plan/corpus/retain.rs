//! A session's cross-file part, kept from run to run (decision 0077, stage 1): each stratum's relations as the last
//! run derived them, and a digest of every input the part read. A run after an edit evaluates only the strata whose
//! inputs changed (spec §21.8: "only rules whose recorded read set intersects a changed (fact, key) pair re-run"),
//! with the other strata's relations bound as inputs from what was kept.
//!
//! ```text
//! run 1:  inputs ──► every stratum ──► outputs        keep: input digests, every stratum's rows (to the budget)
//! run 2:  inputs ──► digests differ for {x}
//!         strata reading x, or reading a stratum that does, run; the others' rows are bound as inputs
//! ```
//!
//! A stratum is clean when none of its reads changed (an input whose digest differs, or a relation of a stratum that
//! runs) and its relations were kept. Kept relations are bounded by a byte budget (`RERAIL_CROSS_RETAIN_MB`, 256 MB):
//! past it, a relation is not held in memory for the next run of this process, but it is still written to the store's
//! frames (`past`), so the next process reads it from there and does not derive it again (the budget is of memory,
//! not of the store; a relation past it used to be named by its reads only, and every next process derived it). Nor is one the run recomputed in its readers
//! (decision 0099: it has no rows to keep). Run again over unchanged reads, such a stratum derives the rows it did,
//! so it leaves its readers clean.
//!
//! Stage 2 (decision 0077 §3, `maintain`) maintains a changed stratum's rows from the deltas (DRed) instead of deriving
//! them again; it reads the old rows of what changed, so inputs' sources are kept too.
//!
//! Nothing is copied (decision 0090 step 3): a relation is kept as the engine held it (`InRel`, with the indexes its
//! waves and readers built), and put back into the next run as a native relation (`adopt`), read in place; an input
//! is kept as its source.

use rerail_blocks::vector::Vector;
/// A source's rows where they lie, digested and lent to a keep: the planner's (`plan::source_rows`).
pub(in crate::code_rule_dsl) use crate::code_rule_dsl::logic::plan::source_rows::{SourceRows, digest, each_row};
use super::{Error, Result, lower};
use crate::code_rule_dsl::logic::plan::Cells;
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::logic::plan::body::Placed;
use crate::code_rule_dsl::logic::program_image::{Lit, RuleRef, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use crate::code_rule_dsl::logic::native::component::InRel;
use crate::content_store::{ContentStore, KeptRel, KeptStored};
use crate::content_store::kept_facts::{KeptPut, KeptRows, KeptWhat};
use crate::code_rule_dsl::logic::program_image::passes::Parts as _;

/// A relation as the last run derived it: the engine's relation, and its decoded rows when it is an output.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Kept {
    pub(in crate::code_rule_dsl) rel: Arc<InRel>,
    pub(in crate::code_rule_dsl) width: usize,
    pub(in crate::code_rule_dsl) output: Option<(std::sync::Arc<crate::code_rule_dsl::logic::plan::output::Rows>, Vector<usize>)>,
}

impl Kept {
    fn bytes(&self) -> usize {
        self.rel.bytes() + self.output.as_ref().map_or(0, |(rows, _)| rows.bytes())
    }
    /// Its rows as a source named as the planner reads relation `name` (`sql_col`).
    fn source(&self, name: &str) -> Arc<dyn Source> {
        self.rel.source((0..self.width).map(|c| crate::code_rule_dsl::logic::plan::sql_col(name, c)).collect())
    }
}


/// What a session keeps of its cross-file part.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Retained {
    /// The program it is of (`key`).
    key: [u8; 32],
    /// A digest of each input's rows.
    inputs: HashMap<String, u64>,
    rels: HashMap<String, Kept>,
    /// Each input's source, for its deltas next run (stage 2).
    input_srcs: HashMap<String, Arc<dyn Source>>,
    /// The layout of each input source read from the store (`adopt_inputs`): its owners' ranges, by which a
    /// changed document's old rows are read (`document_delta_except`).
    /// Each with the source it lays out: the ranges are read only while that source is the input's.
    input_layouts: HashMap<String, (Arc<dyn Source>, Arc<InputLayout>)>,
    /// The changed documents' ranges in an input `load` read as an `OldInput`, with that source.
    framed_ranges: HashMap<String, (Arc<dyn Source>, Arc<Ranges>)>,
    /// The statistics a run measured of a kept relation, under the name it was bound as and the relation it is
    /// (decision 0094 §4: measured once, while it is kept as it is).
    measured: HashMap<(String, usize), Measured>,
    /// The documents the run read, and the file-local program their memo entries are under (`input_delta`).
    docs: (super::input_delta::Docs, [u8; 32]),
    /// Each input source's rows hashed by document (`document_delta`), when a run took them: the next run's old
    /// side, not hashed again.
    input_docs: HashMap<String, Arc<FramedDocs>>,
    /// What the store's frames hold of this part, when it was read from them (t33 K4): a process with no session
    /// starts from them, and writes back what its run changed.
    frames: Option<InKept>,
    /// The relations' rows read from the frames during a maintained run, when a stratum the run reached reads them
    /// (decision 0117 step 4, `admit`): taken with `rels` from then on. Rows read, and relations.
    late: std::cell::RefCell<HashMap<String, Kept>>,
    admitted: std::cell::Cell<(usize, usize)>,
    /// The statistics stored with the rows `admit` read (decision 0120 §3), under the address each has in this
    /// process. Taken as `measured` is (smoking gun 9, t34).
    late_stats: std::cell::RefCell<HashMap<(String, usize), Measured>>,
    /// The relations this run derived past the budget: written to the frames with the kept ones (`write_kept`,
    /// `parts`), then let go (`let_go_past`), never held for the next run of this process.
    past: HashMap<String, Kept>,
    /// The relations the run holds as views of an input's columns (`maintain::held_views`): never kept, their old
    /// rows the view over their input's old rows (decision 0090), read in place.
    views: BTreeMap<String, (String, Vector<usize>)>,
}

/// A source's rows by document: each document's sum of row hashes and its row count.
pub(in crate::code_rule_dsl) type DocDigests = rustc_hash::FxHashMap<i64, (u64, u64)>;

/// An input's rows' digests by document, as a run reads them: those it computed (`Made`, a derived fact), or a kept
/// documents' entry read where it lies (`Kept`, decision 0353 step 8: rows of (document, sum, count) in canonical
/// order, a document found by a search of its leading column; `Docs::kept` checks it).
pub(in crate::code_rule_dsl) enum Docs {
    Made(DocDigests),
    Kept(kept_read::Entry),
    /// `base` with the documents of `changed` as given there (`None`: gone): an edit's documents over the kept ones,
    /// none of the kept read into a table of its own. `n`: how many documents it holds.
    Over { base: Arc<Docs>, changed: rustc_hash::FxHashMap<i64, Option<(u64, u64)>>, n: usize },
}

impl Docs {
    /// A kept documents' entry, when it is three columns led by the document, each document once, and its digest is
    /// `digest` (`None`: not checked, a relation's count).
    fn kept(entry: &kept_read::Entry, digest: Option<u64>) -> Option<Docs> {
        let rows = entry.packed();
        if rows.arity != 3 || (rows.len() > 1 && !rows.leads(0)) {
            return None;
        }
        if (1..rows.len()).any(|i| rows.get(i, 0) == rows.get(i - 1, 0)) {
            return None;
        }
        let docs = Docs::Kept(entry.clone());
        digest.is_none_or(|d| docs.digest() == d).then_some(docs)
    }
    /// `base`, to be changed (`set`).
    pub(in crate::code_rule_dsl) fn over(base: Arc<Docs>) -> Docs {
        let n = base.len();
        Docs::Over { base, changed: Default::default(), n }
    }
    /// Document `doc`'s digest now `v` (`None`: gone): in a table the run made, or over the kept one.
    pub(in crate::code_rule_dsl) fn set(&mut self, doc: i64, v: Option<(u64, u64)>) {
        let had = self.contains_key(&doc);
        match self {
            Docs::Made(m) => {
                match v {
                    Some(v) => m.insert(doc, v),
                    None => m.remove(&doc),
                };
            }
            Docs::Kept(_) => {
                let base = Arc::new(std::mem::replace(self, Docs::Made(DocDigests::default())));
                *self = Docs::over(base);
                self.set(doc, v);
            }
            Docs::Over { changed, n, .. } => {
                *n = *n + v.is_some() as usize - had as usize;
                changed.insert(doc, v);
            }
        }
    }
    pub(in crate::code_rule_dsl) fn get(&self, doc: &i64) -> Option<(u64, u64)> {
        match self {
            Docs::Made(m) => m.get(doc).copied(),
            Docs::Over { base, changed, .. } => match changed.get(doc) {
                Some(v) => *v,
                None => base.get(doc),
            },
            Docs::Kept(e) => {
                let rows = e.packed();
                let (lo, hi) = rows.col(0).dec().equal_range(0, rows.len(), *doc);
                (lo < hi).then(|| (rows.get(lo, 1) as u64, rows.get(lo, 2) as u64))
            }
        }
    }
    pub(in crate::code_rule_dsl) fn contains_key(&self, doc: &i64) -> bool {
        self.get(doc).is_some()
    }
    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        match self {
            Docs::Made(m) => m.len(),
            Docs::Kept(e) => e.len(),
            Docs::Over { n, .. } => *n,
        }
    }
    pub(in crate::code_rule_dsl) fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Each document and its digest.
    pub(in crate::code_rule_dsl) fn iter(&self) -> Box<dyn Iterator<Item = (i64, (u64, u64))> + '_> {
        match self {
            Docs::Made(m) => Box::new(m.iter().map(|(d, v)| (*d, *v))),
            Docs::Over { base, changed, .. } => Box::new(
                base.iter().filter(|(d, _)| !changed.contains_key(d)).chain(changed.iter().filter_map(|(d, v)| v.map(|v| (*d, v)))),
            ),
            Docs::Kept(e) => {
                let rows = e.packed();
                Box::new((0..rows.len()).map(move |i| (rows.get(i, 0), (rows.get(i, 1) as u64, rows.get(i, 2) as u64))))
            }
        }
    }
    pub(in crate::code_rule_dsl) fn keys(&self) -> impl Iterator<Item = i64> + '_ {
        self.iter().map(|(d, _)| d)
    }
    /// Their digest, as `digest_of_docs` takes it.
    pub(in crate::code_rule_dsl) fn digest(&self) -> u64 {
        let (sum, n) = self.iter().fold((0u64, 0u64), |(s, n), (_, (ds, dn))| (s.wrapping_add(ds), n + dn));
        sum ^ n.wrapping_mul(0xff51_afd7_ed55_8ccd)
    }
}

/// A relation's statistics as bound (`syntax::statistics`), and its counts of the constants rules test it by.
/// The counts are of the constants tested when they were measured (the third): taken again only for the same.
pub(in crate::code_rule_dsl) type Measured = (Vec<(String, i64, i64, i64, i64)>, Vec<(String, i64, i64)>, BTreeSet<(usize, i64)>);

/// The key of a cross-file part: the prepared program's key and the part's program as lowered, by its digest
/// (`Lowered::digest`, decision 0173 §7), which reads no rule.
pub(in crate::code_rule_dsl) fn key(prepared: &[u8; 32], cross: u64) -> [u8; 32] {
    keys(prepared, cross).0
}

/// A cross-file part's key (`key`), and the key of its program alone (`cross`, its digest), which names its inputs'
/// frames (`input_ident`) and a session's kept part: the prepared program's key folds in every rule's text, so a
/// file-local rule's edit gave every input of an unchanged cross part a frame of a new name, none found again (every
/// input's frame written again each edit, and an input no memo entry holds read as changed: its strata run again).
pub(in crate::code_rule_dsl) fn keys(prepared: &[u8; 32], cross: u64) -> ([u8; 32], [u8; 32]) {
    use sha2::Digest as _;
    let of = |prepared: &[u8; 32]| -> [u8; 32] {
        let mut hash = sha2::Sha256::new();
        // v3: under the engine build and the grammar (decision 0077 §4): another build may derive other rows from the
        // same inputs, so it keeps its own part, and reads none of an older build's.
        hash.update(b"rerail.cross-retained/v3\0");
        hash.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
        hash.update(crate::code_rule_dsl::build_ids::grammar_version().to_le_bytes());
        hash.update(prepared);
        hash.update(cross.to_le_bytes());
        hash.finalize().into()
    };
    (of(prepared), of(&[0; 32]))
}

/// The cells of relation `rel`'s rows as the program types its columns (`types_of`), when they are `width` wide
/// (else none is an atom: rows of another shape are no frame's).
pub(in crate::code_rule_dsl) fn cells_of<'a>(atoms: &'a crate::content_store::atoms::Atoms, prog: &Placed, domains: &lower::Domains, rel: &str, width: usize) -> Cells<'a> {
    let types = types_of(prog, domains, rel);
    if types.len() == width { Cells::new(atoms, &types) } else { Cells::plain(width) }
}

/// An input's rows new and its rows gone between its last run's source `old` and this run's `new`, as
/// `maintain::rewrite`'s `given` binds them (`r__plus`, `r__minus`), found document by document: each document's rows (by the
/// one its first column names, `memo::document_of`) are hashed as `digest` hashes rows, and only the documents whose
/// hashes differ are compared row by row. A pass over the columns, not a join of every new row against every old
/// one (`r__plus :- r, !r__old`), which read each base relation whole every run however few files changed. Exact
/// whatever the documents are: one whose rows all moved is a document that differs. `None` when the two hold other
/// columns. Each is named as the planner reads its statistics (`sql_col` of `rel__plus`, `rel__minus`).
pub(in crate::code_rule_dsl) fn document_delta(rel: &str, old: &Arc<dyn Source>, was: Option<&Docs>, new: &Arc<dyn Source>, g: usize, cells: &Cells) -> Option<((Arc<dyn Source>, Arc<dyn Source>), Arc<Docs>)> {
    document_delta_except(rel, old, was, new, g, cells, None, None)
}

/// `document_delta`, given the documents whose rows are as they were (`same`: their content, key and memo plan are
/// the last run's, as the base relations' documents' table has them, `base_documents`): their digests are `was`'s,
/// and only the other owners' rows are hashed, not every row of every document. Each owner's rows are counted first, a pass over the owner column: a document of
/// `same` whose count is not `was`'s (a relation fed otherwise than from its documents' entries) sends the whole
/// relation to `document_delta`'s pass.
///
/// `old_ranges`: each owner's rows in `old` (a kept input's layout), by which the old rows of a document that
/// differs are read, not found by reading every row's owner.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn document_delta_except(rel: &str, old: &Arc<dyn Source>, was: Option<&Docs>, new: &Arc<dyn Source>, g: usize, cells: &Cells, same: Option<&rustc_hash::FxHashSet<i64>>, old_ranges: Option<&Ranges>) -> Option<((Arc<dyn Source>, Arc<dyn Source>), Arc<Docs>)> {
    use rustc_hash::FxHashSet;
    let width = new.columns().len();
    // By position: both are the relation's rows in its declared order, whatever their sources name the columns
    // (`c0` for rows kept as frames, the SQL's or the declaration's names for a run's own).
    if width == 0 || old.columns().len() != width || g >= width {
        if std::env::var_os("RERAIL_STRATA_LIVE").is_some() {
            eprintln!("RERAIL_STRATA_LIVE document delta {rel}: columns were {:?}, are {:?}", old.columns(), new.columns());
        }
        return None;
    }
    let all: Vector<usize> = (0..width).collect();
    let mut row = rerail_blocks::vector![0i64; width];
    // A document's rows are mostly one run of rows: its sum is kept while the run lasts, not looked up per row.
    // Each row's hash is of its content (`each_row`), as the frames keep a document's digest.
    let by_doc = |s: &dyn Source, _: &mut [i64]| {
        let mut by = DocDigests::default();
        let mut at: Option<(i64, u64, u64)> = None;
        each_row(s, cells, |row, h| {
            let d = owner_of(row, g);
            match &mut at {
                Some((doc, sum, n)) if *doc == d => {
                    *sum = sum.wrapping_add(h);
                    *n += 1;
                }
                _ => {
                    if let Some((doc, sum, n)) = at.replace((d, h, 1)) {
                        let e = by.entry(doc).or_default();
                        e.0 = e.0.wrapping_add(sum);
                        e.1 += n;
                    }
                }
            }
        });
        if let Some((doc, sum, n)) = at {
            let e = by.entry(doc).or_default();
            e.0 = e.0.wrapping_add(sum);
            e.1 += n;
        }
        by
    };
    // The other owners' rows alone, hashed as `by_doc` hashes them; the documents of `same` as `was` has them.
    let except = |was: &Docs, same: &FxHashSet<i64>| -> Option<(DocDigests, Vector<usize>)> {
        // Each row's document as the source gives it (`documents_of`: a view of a batch's documents names each
        // document's rows by its key, reading no row).
        let _s = crate::code_rule_dsl::logic::profile::span(|| "document delta: its owners counted".into());
        let mut counts: rustc_hash::FxHashMap<i64, u64> = rustc_hash::FxHashMap::default();
        let mut others: rerail_blocks::vector::Vector<usize> = rerail_blocks::vector::Vector::new();
        new.documents_of(g, &mut |r, d| {
            if same.contains(&d) {
                *counts.entry(d).or_default() += 1;
            } else {
                others.push(r);
            }
        });
        if same.iter().any(|d| was.get(d).map_or(0, |w| w.1) != counts.get(d).copied().unwrap_or(0)) {
            return None;
        }
        drop(_s);
        let _s = crate::code_rule_dsl::logic::profile::span(|| "document delta: the other owners' rows hashed".into());
        let mut by: DocDigests = same.iter().filter_map(|d| was.get(d).map(|w| (*d, w))).collect();
        let (mut row, mut keyed) = (vec![0i64; width], vec![0i64; width]);
        for &r in &others {
            new.row_at(r, &all, &mut row);
            keyed.copy_from_slice(&row);
            cells.key(&mut keyed);
            let e = by.entry(owner_of(&row, g)).or_default();
            e.0 = e.0.wrapping_add(crate::code_rule_dsl::logic::plan::row_digest(&keyed));
            e.1 += 1;
        }
        Some((by, others))
    };
    let taken;
    let was = match was {
        Some(w) => w,
        None => {
            taken = Docs::Made(by_doc(old.as_ref(), &mut row));
            &taken
        }
    };
    // The rows of the documents not of `same`, when only those were hashed: the new rows of a document that differs
    // are among them.
    let mut new_others: Option<rerail_blocks::vector::Vector<usize>> = None;
    let now = match same.and_then(|s| except(was, s)) {
        Some((now, others)) => {
            new_others = Some(others);
            now
        }
        None => {
            super::maintain::shape::input_hashed_whole();
            let _s = crate::code_rule_dsl::logic::profile::span(|| "document delta: every row hashed".into());
            by_doc(new.as_ref(), &mut row)
        }
    };
    let differ: FxHashSet<i64> = was.iter().filter(|(d, h)| now.get(d) != Some(h)).map(|(d, _)| d).chain(now.keys().filter(|d| !was.contains_key(d)).copied()).collect();
    // The rows of the documents that differ, each with its values' hash, read once: where the source finds them
    // without reading every row (a kept input's by its owners' ranges, a view's by its documents: `rows_owned`), else
    // every row's owner is read.
    let hashed = |s: &dyn Source, rs: &mut dyn Iterator<Item = usize>, check: bool, row: &mut [i64]| -> Vec<((u64, u64), usize)> {
        rs.filter_map(|r| {
            s.row_at(r, &all, row);
            (!check || differ.contains(&owner_of(row, g))).then(|| (row_hash(row), r))
        })
        .collect()
    };
    let ids_of = |s: &dyn Source, row: &mut [i64]| -> Vec<((u64, u64), usize)> {
        if differ.is_empty() {
            return Vec::new();
        }
        let owned = |v: i64| differ.contains(&super::memo::document_of(v));
        let rows = s.rows_owned(g, &owned).unwrap_or_else(|| {
            let mut out = rerail_blocks::vector::Vector::new();
            s.documents_of(g, &mut |r, d| {
                if differ.contains(&d) {
                    out.push(r);
                }
            });
            out
        });
        hashed(s, &mut rows.into_iter(), false, row)
    };
    let gone_ids = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "document delta: the old rows of the documents that differ".into());
        match old_ranges {
            Some(ranges) => {
                let mut rs = differ.iter().filter_map(|d| ranges.get(d)).flat_map(|&(at, n)| at as usize..(at + n) as usize);
                hashed(old.as_ref(), &mut rs, true, &mut row)
            }
            None => ids_of(old.as_ref(), &mut row),
        }
    };
    let made_ids = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| "document delta: the new rows of the documents that differ".into());
        match &new_others {
            Some(others) => hashed(new.as_ref(), &mut others.iter().copied(), true, &mut row),
            None => ids_of(new.as_ref(), &mut row),
        }
    };
    Some((rows_delta(rel, width, old, &gone_ids, new, &made_ids), Arc::new(Docs::Made(now))))
}

/// A row's values' hash, by which `rows_delta` matches the rows of two sources: two independent 64-bit hashes (Fx,
/// and the frames' `row_digest`), so a row is matched by 128 bits, read once. A document's change is already judged
/// by a 64-bit sum of its rows' digests (`DocDigests`).
fn row_hash(row: &[i64]) -> (u64, u64) {
    use std::hash::{Hash, Hasher};
    let mut h = rustc_hash::FxHasher::default();
    row.hash(&mut h);
    (h.finish(), crate::code_rule_dsl::logic::plan::row_digest(row))
}

/// Each row of `ids` of `s` with its hash (`row_hash`).
fn hashed_rows(s: &dyn Source, width: usize, ids: impl Iterator<Item = usize>) -> Vec<((u64, u64), usize)> {
    let all: Vector<usize> = (0..width).collect();
    let mut row = rerail_blocks::vector![0i64; width];
    ids.map(|r| {
        s.row_at(r, &all, &mut row);
        (row_hash(&row), r)
    })
    .collect()
}

/// The rows of `made` (of `new`) that no row of `gone` (of `old`) holds, added, and those of `gone` that no row of
/// `made` holds, gone, as views of their ids over each source (0364 R2: an input's change named by its rows'
/// places, no row copied). Both sides come with their rows' hashes (`row_hash`), taken as each row was read; the
/// two sides are matched by them, sorted. A row held twice on one side counts once.
fn rows_delta(rel: &str, width: usize, old: &Arc<dyn Source>, gone: &[((u64, u64), usize)], new: &Arc<dyn Source>, made: &[((u64, u64), usize)]) -> (Arc<dyn Source>, Arc<dyn Source>) {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "document delta: the rows matched".into());
    let (mut gone, mut made) = (gone.to_vec(), made.to_vec());
    gone.sort_unstable();
    made.sort_unstable();
    gone.dedup_by_key(|x| x.0);
    made.dedup_by_key(|x| x.0);
    let (mut plus, mut minus): (Vector<u32>, Vector<u32>) = (rerail_blocks::vector::Vector::new(), rerail_blocks::vector::Vector::new());
    let (mut i, mut j) = (0, 0);
    while i < gone.len() || j < made.len() {
        match (gone.get(i), made.get(j)) {
            (Some(g), Some(m)) if g.0 == m.0 => {
                i += 1;
                j += 1;
            }
            (Some(g), Some(m)) if g.0 < m.0 => {
                minus.push(g.1 as u32);
                i += 1;
            }
            (Some(g), None) => {
                minus.push(g.1 as u32);
                i += 1;
            }
            (_, Some(m)) => {
                plus.push(m.1 as u32);
                j += 1;
            }
            (None, None) => break,
        }
    }
    plus.sort_unstable();
    minus.sort_unstable();
    let names = || (0..width).map(|c| crate::code_rule_dsl::logic::plan::sql_col(rel, c)).collect::<Vec<_>>();
    use crate::code_rule_dsl::logic::native::component::InRel;
    (InRel::picked(Arc::clone(new), plus, names()), InRel::picked(Arc::clone(old), minus, names()))
}

/// An input's delta when its rows only grew (`maintained::Delta`; the guests' rounds, decision 0331): a later round's
/// batch is the round before's with new documents, so a base relation holds the rows it held and the new documents'.
/// When the old rows are the new ones' first rows, in order (the batch builds documents in order, and a round's
/// documents come last), its new rows are the rest: one pass over the old rows' values, no row hashed. Otherwise the
/// two are compared as sets of rows (`rows_delta`), which is exact whatever moved. `None` when the two hold other
/// columns.
pub(in crate::code_rule_dsl) fn grown_delta(rel: &str, old: &Arc<dyn Source>, new: &Arc<dyn Source>) -> Option<(Arc<dyn Source>, Arc<dyn Source>)> {
    let width = new.columns().len();
    if old.columns().len() != width {
        return None;
    }
    let all: Vector<usize> = (0..width).collect();
    let (mut a, mut b) = (vec![0i64; width], vec![0i64; width]);
    let prefix = old.len() <= new.len()
        && (0..old.len()).all(|r| {
            old.row_at(r, &all, &mut a);
            new.row_at(r, &all, &mut b);
            a == b
        });
    if prefix {
        let made: Vector<usize> = (old.len()..new.len()).collect();
        return Some(rows_delta(rel, width, old, &[], new, &hashed_rows(new.as_ref(), width, made.into_iter())));
    }
    Some(rows_delta(rel, width, old, &hashed_rows(old.as_ref(), width, 0..old.len()), new, &hashed_rows(new.as_ref(), width, 0..new.len())))
}

/// An input's delta when the rows it gained are all of new documents and it lost none (the guests' rounds, decision
/// 0322): the rows whose node column `g` is of a document `is_new` names, with nothing gone. One pass over that
/// column, against `grown_delta`'s hashing of every row old and new. `None` unless the rows add up: the old less
/// none, with those, are the rows now.
pub(in crate::code_rule_dsl) fn owners_delta(rel: &str, old: &Arc<dyn Source>, new: &Arc<dyn Source>, g: usize, is_new: &dyn Fn(i64) -> bool) -> Option<(Arc<dyn Source>, Arc<dyn Source>)> {
    let width = new.columns().len();
    if old.columns().len() != width || g >= width {
        return None;
    }
    // The new documents' rows: a view over the batch's documents finds them itself (`Source::rows_owned`: a kind-set
    // axis by its nodes, without its solve, which was 348 MB of a PostHog run's peak for a round's few new documents);
    // else each row's document (`Source::documents_of`: a view's rows have their document's key, no id read).
    let fresh = new.rows_owned(g, &|n: i64| is_new(super::memo::document_of(n))).unwrap_or_else(|| {
        let mut fresh: rerail_blocks::vector::Vector<usize> = rerail_blocks::vector::Vector::new();
        new.documents_of(g, &mut |r, d| {
            if is_new(d) {
                fresh.push(r);
            }
        });
        fresh
    });
    (old.len() + fresh.len() == new.len()).then(|| rows_delta(rel, width, old, &[], new, &hashed_rows(new.as_ref(), width, fresh.into_iter())))
}

/// Which strata a run evaluates: those not `clean`.
pub(in crate::code_rule_dsl) struct Cut {
    /// Relations taken from what was kept: their strata do not run.
    pub(in crate::code_rule_dsl) clean: BTreeSet<String>,
    /// Strata run, and in all.
    pub(in crate::code_rule_dsl) strata: (usize, usize),
    /// Inputs whose rows changed since the last run.
    pub(in crate::code_rule_dsl) changed: Vec<String>,
    /// The relations the program derives as a view of an input's columns (Q37, `input_view_shape`), by its rules:
    /// never kept as rows, whatever form a run held them in (`after`, Q421).
    pub(in crate::code_rule_dsl) views: BTreeSet<String>,
    /// The views no stratum that runs reads, and no output is (t33 K4): not made. A view is never kept, so `clean`
    /// leaves it out, and it was made again every run (a view of a tape's axis read every document's tape) though
    /// every reader of it was clean.
    pub(in crate::code_rule_dsl) idle: BTreeSet<String>,
}

/// The relations a `nearest` literal reads.
pub(in crate::code_rule_dsl) const TREE: [&str; 2] = ["node", "child"];

/// What a stratum whose rules call a Git functor (`GIT_FUNCTORS`) reads beside its relations: their answers
/// (`strings::git::Answers`), which move between runs. It is an input of those strata by their digest, as a relation
/// is: a run that finds an answer changed runs them again, and a kept stratum names it among its reads.
pub(in crate::code_rule_dsl) const GIT: &str = "@git: the Git functors' answers";

/// The functors that answer from the repository holding the run's root (`strings::git`).
const GIT_FUNCTORS: [&str; 3] = ["git_tracked", "git_commit", "git_has"];

/// Whether rule `r` calls a Git functor: in its head, its atoms' (and aggregates' atoms') terms, or a comparison.
fn calls_git(v: &Version, r: RuleRef<'_>) -> bool {
    fn calls(v: &Version, t: TermRef<'_>) -> bool {
        matches!(t, TermRef::Call(f, args) if GIT_FUNCTORS.contains(&v.text(f)) || args.clone().any(|a| calls(v, a)))
    }
    std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).any(|a| a.args().any(|t| calls(v, t)))
        || r.body().any(|l| matches!(l.view(), Lit::Cmp { l, r, .. } if calls(v, l) || calls(v, r)))
}

/// Whether a rule of `prog` calls a Git functor (`GIT`): its run gives their answers' digest as an input's.
pub(in crate::code_rule_dsl) fn reads_git(prog: &Placed) -> bool {
    prog.v.rules().any(|r| calls_git(&prog.v, r))
}

/// The relations each stratum's rules read (atoms, aggregates' atoms and a native operator's inputs), its rules found
/// by their heads in one pass over the program: a scan of every rule per stratum was 15% of a one-file run of the
/// Python pack (16 k rules, 10 k strata).
pub(super) struct Reads<'a> {
    f: super::super::fields::Fields<'a>,
    by_head: HashMap<&'a str, Vec<RuleRef<'a>>>,
}

impl<'a> Reads<'a> {
    pub(super) fn of(prog: &'a Placed) -> Result<Self> {
        let f = prog.fields()?;
        let mut by_head: HashMap<&str, Vec<RuleRef<'a>>> = HashMap::new();
        for r in prog.v.rules() {
            by_head.entry(prog.v.rel_name(r.head().rel())).or_default().push(r);
        }
        Ok(Reads { f, by_head })
    }

    pub(super) fn of_scc(&self, scc: &crate::code_rule_dsl::logic::front::Comp) -> BTreeSet<String> {
        let v = self.f.v;
        let rules = || scc.iter().filter_map(|rel| self.by_head.get(rel)).flatten();
        let mut out: BTreeSet<String> = rules().flat_map(|r| r.reads()).map(|a| v.rel_name(a.rel()).to_owned()).collect();
        // A `nearest` literal climbs the tree: it reads `node` and `child`, which an edit changes.
        if rules().any(|r| r.body().any(|l| matches!(l.view(), Lit::Nearest { .. }))) {
            out.extend(TREE.iter().map(|r| (*r).to_owned()));
        }
        if rules().any(|r| calls_git(v, *r)) {
            out.insert(GIT.to_owned());
        }
        for rel in scc {
            if let Some(native) = self.f.native(rel) {
                out.extend(native.reads.iter().map(|r| v.rel_name(*r).to_owned()));
            }
        }
        out
    }
}

/// The strata of `prog` that `kept` (of program `key`) lets a run skip, given its inputs' digests.
/// `sccs`: `prog`'s strata (`analyses::strata`), computed once by the caller for every pass over the program.
pub(in crate::code_rule_dsl) fn cut(kept: Option<&Retained>, key: [u8; 32], prog: &Placed, sccs: &crate::code_rule_dsl::logic::front::Components, inputs: &HashMap<String, u64>) -> Result<Cut> {
    let kept = kept.filter(|k| k.key == key);
    let mut changed: Vec<String> = inputs.iter().filter(|(rel, d)| kept.and_then(|k| k.inputs.get(*rel)) != Some(*d)).map(|(rel, _)| rel.clone()).collect();
    changed.sort();
    let mut dirty: BTreeSet<String> = changed.iter().cloned().collect();
    let mut clean = BTreeSet::new();
    // Gates decide on the whole program. Reuse it only when every input and derived relation survived unchanged.
    let mut views: BTreeMap<String, ()> = BTreeMap::new();
    // A copy of an input's columns is held as a view of it in every run (Q37, `input_view_shape`): not kept either.
    {
        let f = prog.fields()?;
        let mut by_head: BTreeMap<&str, Vec<RuleRef<'_>>> = BTreeMap::new();
        for r in prog.v.rules() {
            by_head.entry(prog.v.rel_name(r.head().rel())).or_default().push(r);
        }
        for (rel, rules) in by_head {
            if let [rule] = rules.as_slice()
                && !f.has_facts(rel) && f.native(rel).is_none() && !f.lattice(rel) && !f.request(rel) && !f.produced(rel) && !f.gate(rel)
                && crate::code_rule_dsl::logic::plan::input_view_shape(&prog.v, *rule, |r| f.is_input(r)).is_some()
            {
                views.insert(rel.to_owned(), ());
            }
        }
    }
    let complete = kept.is_some_and(|k| sccs.iter().flatten().all(|r| k.holds(r, prog) || views.contains_key(r)));
    let whole = kept.is_none() || (!prog.fields()?.gates().is_empty() && (!changed.is_empty() || !complete));
    let mut ran = 0;
    let reads = Reads::of(prog)?;
    for scc in sccs {
        let stale = whole || reads.of_scc(scc).iter().any(|r| dirty.contains(r));
        if stale {
            ran += 1;
            dirty.extend(scc.iter().map(str::to_owned));
        } else if scc.iter().all(|r| views.contains_key(r)) {
            // A view of unchanged reads: not run, and not clean (no rows of it are kept to bind).
        } else if !scc.iter().all(|r| kept.is_some_and(|k| k.holds(r, prog))) {
            // Not kept (past the budget, or recomputed in its readers, decision 0099) but of unchanged reads: run
            // again, its rows as before, so its readers stay clean.
            ran += 1;
        } else {
            clean.extend(scc.iter().map(str::to_owned));
        }
    }
    // What the strata that run read, and the outputs not kept, from the last stratum back (a stratum's reads come
    // before it): a view that none of them reads is idle. A clean stratum's reads are none's: its rows are kept.
    let mut needed: BTreeSet<String> = prog.v.outputs().map(|a| prog.v.text(a).to_owned()).filter(|o| !clean.contains(o)).collect();
    needed.extend(sccs.iter().flatten().filter(|r| !clean.contains(*r) && !views.contains_key(*r)).map(str::to_owned));
    let mut idle = BTreeSet::new();
    for scc in sccs.iter().rev() {
        if scc.iter().any(|r| needed.contains(r) && !clean.contains(r)) {
            needed.extend(reads.of_scc(scc));
        } else if scc.iter().all(|r| views.contains_key(r)) {
            idle.extend(scc.iter().map(str::to_owned));
        }
    }
    if !views.is_empty() {
        crate::code_rule_dsl::logic::profile::decision("views not made (t33 K4)", || {
            let made: Vec<&String> = views.keys().filter(|v| !idle.contains(*v)).take(8).collect();
            format!("{} of {} views read by no stratum that runs, nor an output; made: {made:?}", idle.len(), views.len())
        });
    }
    Ok(Cut { clean, strata: (ran, sccs.len()), changed, views: views.into_keys().collect(), idle })
}

impl Cut {
    /// `cross` without the clean strata's rules: the relations dirty rules read become inputs, and clean outputs are
    /// left to `outputs`.
    ///
    /// The cut is a part of `cross`'s version (decision 0244), read and made by its handles.
    pub(in crate::code_rule_dsl) fn program(&self, cross: &lower::Lowered) -> super::Result<lower::Lowered> {
        use crate::code_rule_dsl::logic::program_image::PartFields;
        let (dirty, all, none) = (|rel: &str| !self.clean.contains(rel) && !self.idle.contains(rel), |_: &str| true, |_: &str| false);
        let gates: &dyn Fn(&str) -> bool = if self.strata.0 == 0 { &none } else { &all };
        let v = &cross.version;
        {
            let read: BTreeSet<&str> = v.rules().filter(|r| dirty(v.rel_name(r.head().rel()))).flat_map(|r| r.reads().map(|a| v.rel_name(a.rel()))).collect();
            let mut inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
            for rel in self.clean.iter().filter(|r| read.contains(r.as_str())) {
                if !inputs.contains(rel) {
                    inputs.push(rel.clone());
                }
            }
            let outputs: Vec<String> = v.outputs().map(|a| v.text(a)).filter(|o| dirty(o)).map(str::to_owned).collect();
            let part = v.part(dirty, &inputs, &outputs, PartFields { natives: &dirty, lattice: &all, gates }).map_err(super::Error::Plan)?;
            crate::code_rule_dsl::logic::version_trace::note("session.cut", v, &part);
            Ok(cross.with_version(part))
        }
    }

    /// The clean relations a program reads (its inputs, `is_input`), as kept: each as a source (bound by the caller, as the other inputs) and
    /// as the engine's relation (adopted by the caller, read in place).
    pub(in crate::code_rule_dsl) fn sources(&self, kept: &Retained, is_input: &dyn Fn(&str) -> bool) -> Vec<(String, Arc<dyn Source>, Option<Arc<InRel>>)> {
        self.clean
            .iter()
            .filter(|rel| is_input(rel))
            .filter_map(|rel| {
                let k = kept.rels.get(rel)?;
                Some((rel.clone(), k.source(rel), Some(Arc::clone(&k.rel))))
            })
            .collect()
    }

    /// The clean outputs' decoded rows and node columns, as the last run gave them.
    pub(in crate::code_rule_dsl) fn outputs(&self, kept: &Retained, outputs: &[String]) -> Vec<(String, std::sync::Arc<crate::code_rule_dsl::logic::plan::output::Rows>, Vector<usize>)> {
        outputs
            .iter()
            .filter(|o| self.clean.contains(*o))
            .filter_map(|o| kept.rels.get(o).and_then(|k| k.output.clone()).map(|(lines, nodes)| (o.clone(), lines, nodes)))
            .collect()
    }

    /// The relations this run derives, in stratum order: kept for the next run, as far as the budget goes.
    /// `sccs`: the program's strata, as `cut` was given them.
    pub(in crate::code_rule_dsl) fn derived(&self, sccs: &crate::code_rule_dsl::logic::front::Components) -> Result<Vec<String>> {
        Ok(sccs.iter().flatten().filter(|r| !self.clean.contains(*r) && !self.idle.contains(*r)).map(str::to_owned).collect())
    }
}

/// The byte budget of what a session keeps of its cross-file part.
pub(in crate::code_rule_dsl) fn budget() -> usize {
    // 256 MB of the default 1 GB budget, scaled with it (decision 0126).
    std::env::var("RERAIL_CROSS_RETAIN_MB").ok().and_then(|v| v.parse::<usize>().ok()).map_or_else(|| rerail_accel::budget::share(256 << 20), |mb| mb << 20)
}

/// What the next run keeps: the clean relations as they were, and those this run derived (each the engine's
/// relation, with its width), in stratum order while they fit the budget, with the decoded `outputs`; and every
/// input's source (held anyway while the session's inputs are: nothing is copied). `budget`: the bytes held in
/// memory (`budget()`); what is past it is still written to the frames (`past`).
pub(in crate::code_rule_dsl) fn after(
    old: Option<Retained>,
    key: [u8; 32],
    inputs: HashMap<String, u64>,
    cut: &Cut,
    derived: Vec<(String, usize, Arc<InRel>)>,
    outputs: &BTreeMap<String, (std::sync::Arc<crate::code_rule_dsl::logic::plan::output::Rows>, Vector<usize>)>,
    sources: &[(String, Arc<dyn Source>)],
    measured: HashMap<(String, usize), Measured>,
    input_docs: HashMap<String, Arc<Docs>>,
    docs: (super::input_delta::Docs, [u8; 32]),
    budget: usize,
    indexes: bool,
) -> Retained {
    let mut input_docs: HashMap<String, Arc<FramedDocs>> = input_docs.into_iter().map(|(rel, d)| (rel, FramedDocs::ready(d))).collect();
    // An input bound as the last run's source keeps that source's hashes: as the last run held them, decoded from
    // its frame only where asked (Q91).
    if let Some(old) = &old {
        for (rel, s) in sources {
            if !input_docs.contains_key(rel)
                && old.input_srcs.get(rel).is_some_and(|was| Arc::ptr_eq(was, s))
                && let Some(d) = old.docs_handle(rel)
            {
                input_docs.insert(rel.clone(), d);
            }
        }
    }
    let mut rels: HashMap<String, Kept> = HashMap::new();
    if let Some(old) = old.filter(|o| o.key == key) {
        for (rel, k) in old.rels.into_iter().chain(old.late.into_inner()) {
            if cut.clean.contains(&rel) {
                rels.insert(rel, k);
            }
        }
    }
    let mut bytes: usize = rels.values().map(Kept::bytes).sum();
    let mut past: HashMap<String, Kept> = HashMap::new();
    for (rel, width, held) in derived {
        // An index's relation (the reach index, decision 0119), or a view of an input's columns (Q37, 0329), is made
        // again in less than reading it back costs: not kept, so it never takes the budget from the relations whose
        // rows are work. An index's relation is kept where the part stays in memory (`indexes`: a chunk's guest
        // rounds, decision 0331), so its next run's delta is made against it and its readers are maintained. A kept
        // relation read where it lies (decision 0456 step 6b), which a round's maintain mode passes on unchanged, is
        // a relation's rows: kept (dropped, it was neither held nor current, and the next round gave up, "not kept").
        // A view by the program's rules (`Cut::views`) is a view whatever form this run held it in: a run that made
        // its rows (a maintain mode deriving it from its input's versions) keeps none of them either (Q421, 0353).
        let by_rules = cut.views.contains(&rel);
        if (by_rules || held.view.is_some() || held.viewed) && !held.kept_in_place() && !(indexes && held.held_source().is_some()) {
            if std::env::var_os("RERAIL_SHAPE_TRACE").is_some() {
                eprintln!("RERAIL_SHAPE_TRACE not kept {rel}: a view ({} rows; viewed {}, over a source {}, by its rules {by_rules})", held.rows(), held.viewed, held.view.is_some());
            }
            continue;
        }
        let k = Kept { rel: held, width, output: outputs.get(&rel).cloned() };
        // In stratum order while they fit; after the first that does not, every other is past the budget too.
        if bytes <= budget {
            bytes += k.bytes();
        }
        if bytes > budget {
            past.insert(rel, k);
            continue;
        }
        rels.insert(rel, k);
    }
    let input_srcs = sources.iter().filter(|(_, s)| !s.columns().is_empty()).map(|(rel, s)| (rel.clone(), Arc::clone(s))).collect();
    // Statistics of the relations still kept, as they are.
    let alive: std::collections::HashSet<usize> = rels.values().chain(past.values()).map(|k| Arc::as_ptr(&k.rel) as *const u8 as usize).collect();
    let measured = measured.into_iter().filter(|((_, at), _)| alive.contains(at)).collect();
    Retained { key, inputs, rels, input_srcs, measured, input_docs, docs, frames: None, past, ..Default::default() }
}

impl Retained {
    /// The part taken under program `key`, another program of the same cross part (`keys`): its rows are the cross
    /// program's, its inputs compared by digest (`cut`), and its documents answered only under the file-local
    /// program they were read under (`docs`).
    pub(in crate::code_rule_dsl) fn rekeyed(mut self, key: [u8; 32]) -> Self {
        self.key = key;
        self
    }

    /// The part a run of this process kept (`cells`), read again by the next run: its space's blocks bound as the
    /// store's last commit has them (this process's own writes in it, `Bound::of`), and what it knows of the entries
    /// kept only where that commit agrees. Bound at its first run, it read that commit's entries in every later run,
    /// against what it had kept since: none agreed, and each later run derived its part again (or, after a run on
    /// another store, read that store's blocks).
    pub(in crate::code_rule_dsl) fn rebind(&mut self, store: &ContentStore) -> Result<()> {
        let Some(frames) = self.frames.as_mut() else { return Ok(()) };
        let bound = kept_read::Bound::of(store, &frames.space).map_err(|e| Error::Plan(e.to_string()))?;
        if let (Some(a), Some(b)) = (&frames.bound, &bound)
            && Arc::ptr_eq(a, b)
        {
            return Ok(());
        }
        let digest_of = |rel: &str| {
            let name = frames.names.get(rel)?;
            bound.as_ref()?.entry(&name.name, &name.ident, DOCS_PART).map(|e| e.digest())
        };
        let (was_derived, was_inputs) = (frames.derived.len(), frames.inputs.len());
        frames.derived.retain(|rel, s| digest_of(rel) == Some(s.digest));
        frames.inputs.retain(|rel, (d, _)| digest_of(rel) == Some(*d));
        let (derived, inputs) = (frames.derived.len(), frames.inputs.len());
        crate::code_rule_dsl::logic::profile::decision("a part kept in this process, bound again (decision 0401 F3)", || {
            format!("{derived} of {was_derived} relations and {inputs} of {was_inputs} inputs as the store's last commit keeps them")
        });
        frames.bound = bound;
        Ok(())
    }

    /// What a run of program `key` kept, read back from the store in a new process (t32 4.1, `kept_strata::load`):
    /// each input's digest and source, and each relation's rows as the engine holds them. Nothing measured.
    /// `digested`: inputs known by their digest alone (no rows kept: unchanged, or derived again if they changed).
    pub(in crate::code_rule_dsl) fn of_rows(
        key: [u8; 32],
        inputs: Vec<(String, u64, Arc<dyn Source>)>,
        digested: Vec<(String, u64)>,
        rels: Vec<(String, usize, Arc<InRel>)>,
    ) -> Retained {
        let mut out = Retained { key, ..Default::default() };
        for (rel, digest, source) in inputs {
            out.inputs.insert(rel.clone(), digest);
            out.input_srcs.insert(rel, source);
        }
        out.inputs.extend(digested);
        for (rel, width, held) in rels {
            out.rels.insert(rel, Kept { rel: held, width, output: None });
        }
        out
    }

    /// Each input's digest and source, and each relation kept (`of_rows`'s parts): what the store keeps.
    pub(in crate::code_rule_dsl) fn parts(&self) -> (Vec<(&str, u64, &Arc<dyn Source>)>, Vec<(&str, usize, &Arc<InRel>)>) {
        let inputs = self.input_srcs.iter().filter_map(|(rel, s)| Some((rel.as_str(), *self.inputs.get(rel)?, s))).collect();
        let rels = self.rels.iter().chain(&self.past).map(|(rel, k)| (rel.as_str(), k.width, &k.rel)).collect();
        (inputs, rels)
    }

    /// Lets go of the relations past the budget, once they are written to the store (`past`).
    pub(in crate::code_rule_dsl) fn let_go_past(&mut self) {
        self.past = HashMap::new();
    }

    /// Relation `rel` as this run leaves it for the store: kept, or past the budget.
    fn framed(&self, rel: &str) -> Option<&Kept> {
        self.rels.get(rel).or_else(|| self.past.get(rel))
    }

    /// The inputs of program `key` whose rows are as the last run had them (`inputs`, each input's digest): the
    /// last run's sources of them, read again in place of this run's copies, so their column indexes and
    /// statistics are kept with them (0094 §4), not built again.
    pub(in crate::code_rule_dsl) fn same_inputs(&self, key: [u8; 32], inputs: &HashMap<String, u64>) -> Vec<(String, Arc<dyn Source>)> {
        if self.key != key {
            return Vec::new();
        }
        inputs
            .iter()
            .filter(|(rel, d)| self.inputs.get(*rel) == Some(*d))
            .filter_map(|(rel, _)| self.input_srcs.get(rel).map(|s| (rel.clone(), Arc::clone(s))))
            .collect()
    }

    /// The statistics measured of kept relation `held` bound as `name`, if a run measured them.
    pub(in crate::code_rule_dsl) fn measured(&self, name: &str, held: &Arc<InRel>) -> Option<Measured> {
        let key = (name.to_owned(), Arc::as_ptr(held) as *const u8 as usize);
        self.measured.get(&key).cloned().or_else(|| self.late_stats.borrow().get(&key).cloned())
    }

    /// Every statistic kept, to be carried to the next run's (`after`).
    pub(in crate::code_rule_dsl) fn measured_all(&self) -> HashMap<(String, usize), Measured> {
        let mut all = self.late_stats.borrow().clone();
        all.extend(self.measured.iter().map(|(k, m)| (k.clone(), m.clone())));
        all
    }

    /// The last run's source of an input, as it bound it.
    /// Kept relation `rel` and its width, if this run kept it.
    pub(in crate::code_rule_dsl) fn kept(&self, rel: &str) -> Option<(Arc<InRel>, usize)> {
        match self.rels.get(rel) {
            Some(k) => Some((Arc::clone(&k.rel), k.width)),
            None => self.late.borrow().get(rel).map(|k| (Arc::clone(&k.rel), k.width)),
        }
    }

    /// Whether `rel`'s kept rows are known without reading them: held, read, or in the frames (`admit` reads them).
    pub(in crate::code_rule_dsl) fn keeps(&self, rel: &str) -> bool {
        self.rels.contains_key(rel) || self.late.borrow().contains_key(rel) || self.frames.as_ref().is_some_and(|f| f.derived.contains_key(rel))
    }

    /// Binds the kept rows of those of `rels` the space holds and this part has not read (decision 0117 step 4), where
    /// they lie (decision 0401 F3): a maintained run's level reads the old rows of the strata it reaches and the kept
    /// rows they read, with their levels (decision 0090) and statistics (decision 0120 §3). `false`: an entry is not
    /// as the run's cut read it (missing, or of another shape).
    pub(in crate::code_rule_dsl) fn admit(&self, store: &ContentStore, prog: &Placed, rels: &[String]) -> Result<bool> {
        let Some(frames) = self.frames.as_ref() else { return Ok(true) };
        let wanted: Vec<&String> = {
            let late = self.late.borrow();
            rels.iter().filter(|r| frames.derived.contains_key(*r) && !self.rels.contains_key(*r) && !late.contains_key(*r)).collect()
        };
        if wanted.is_empty() {
            return Ok(true);
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("{} relations' kept rows, bound where they lie", wanted.len()));
        let Some(bound) = frames.bound.as_ref() else { return Ok(false) };
        let atoms = store.atoms_handle();
        let mut late = self.late.borrow_mut();
        let (mut rows, mut n) = self.admitted.get();
        for rel in wanted {
            let name = &frames.names[rel];
            let Some(entry) = bound.entry(&name.name, &name.ident, DOCS_PART) else { return Ok(false) };
            super::maintain::shape::frames_read(1);
            let Some(width) = prog.fields()?.arity(rel) else { return Ok(false) };
            if Some(entry.digest()) != frames.derived.get(rel).map(|s| s.digest) || entry.width() != width {
                return Ok(false);
            }
            let Some(kept) = entry.rel(&atoms) else { return Ok(false) };
            rows += entry.len();
            n += 1;
            if kept_stats::on() {
                self.take_stats(frames, rel, &kept);
            }
            late.insert(rel.clone(), Kept { rel: kept, width, output: None });
        }
        self.admitted.set((rows, n));
        Ok(true)
    }

    /// The statistics entry kept with `rel`'s rows, when kept, taken for `held`, the relation those rows are in this
    /// process. An entry of another digest is of other rows: not taken.
    fn take_stats(&self, frames: &InKept, rel: &str, held: &Arc<InRel>) {
        let Some(all) = kept_stats_of(frames, rel) else { return };
        let at = Arc::as_ptr(held) as *const u8 as usize;
        let mut late = self.late_stats.borrow_mut();
        for ((name, _), m) in all {
            late.insert((name, at), m);
        }
    }

    /// The rows' count of each of `rels` the space keeps, read from their entries (their rows are not read).
    pub(in crate::code_rule_dsl) fn stored_rows(&self, _store: &ContentStore, rels: &[String]) -> HashMap<String, usize> {
        let Some((frames, bound)) = self.frames.as_ref().and_then(|f| Some((f, f.bound.as_ref()?))) else { return HashMap::new() };
        rels.iter()
            .filter(|r| frames.derived.contains_key(*r))
            .filter_map(|r| {
                let name = &frames.names[r];
                Some((r.clone(), bound.entry(&name.name, &name.ident, DOCS_PART)?.len()))
            })
            .collect()
    }

    /// The relations and rows `admit` read.
    pub(in crate::code_rule_dsl) fn admitted(&self) -> (usize, usize) {
        self.admitted.get()
    }

    pub(in crate::code_rule_dsl) fn input_source(&self, rel: &str) -> Option<&Arc<dyn Source>> {
        self.input_srcs.get(rel)
    }

    /// That source's owners' ranges, when it was read from the store (`adopt_inputs`) and its layout reads.
    pub(in crate::code_rule_dsl) fn input_ranges(&self, rel: &str) -> Option<&Arc<Ranges>> {
        if let Some((source, ranges)) = self.framed_ranges.get(rel)
            && Arc::ptr_eq(source, self.input_srcs.get(rel)?)
        {
            return Some(ranges);
        }
        // A session's patched input (`Layered`): its ranges as its rows are numbered now.
        if let Some(l) = self.input_srcs.get(rel)?.as_any().and_then(|a| a.downcast_ref::<Layered>()) {
            return Some(l.ranges_now());
        }
        let (source, layout) = self.input_layouts.get(rel)?;
        if !Arc::ptr_eq(source, self.input_srcs.get(rel)?) {
            return None;
        }
        layout.laid_out().ok().map(|l| &l.ranges)
    }

    /// That source's rows hashed by document, if a run took them (`document_delta`).
    pub(in crate::code_rule_dsl) fn input_docs(&self, rel: &str) -> Option<&Arc<Docs>> {
        match self.input_docs.get(rel) {
            Some(d) => d.get(),
            None => self.frames.as_ref()?.inputs.get(rel).filter(|(d, _)| self.inputs.get(rel) == Some(d))?.1.get(),
        }
    }

    /// `input_docs`' handle, not decoded.
    fn docs_handle(&self, rel: &str) -> Option<Arc<FramedDocs>> {
        match self.input_docs.get(rel) {
            Some(d) => Some(Arc::clone(d)),
            None => self.frames.as_ref()?.inputs.get(rel).filter(|(d, _)| self.inputs.get(rel) == Some(d)).map(|(_, d)| Arc::clone(d)),
        }
    }

    /// The digest the last run took of an input (`digest`, or a phase export's version).
    pub(in crate::code_rule_dsl) fn input_digest(&self, rel: &str) -> Option<u64> {
        self.inputs.get(rel).copied()
    }

    /// The documents the last run read, if under the file-local program `program`.
    pub(in crate::code_rule_dsl) fn docs(&self, program: &[u8; 32]) -> Option<&super::input_delta::Docs> {
        (self.docs.1 == *program).then_some(&self.docs.0)
    }
    /// A relation's or an input's rows as the last run had them, as a source named `name` (`sql_col`), with the
    /// engine's relation for a derived one (to adopt, read in place).
    pub(in crate::code_rule_dsl) fn old_source(&self, rel: &str, name: &str) -> Option<(Arc<dyn Source>, Option<Arc<InRel>>)> {
        if let Some(k) = self.rels.get(rel) {
            return Some((k.source(name), Some(Arc::clone(&k.rel))));
        }
        if let Some(k) = self.late.borrow().get(rel) {
            return Some((k.source(name), Some(Arc::clone(&k.rel))));
        }
        self.input_srcs.get(rel).map(|s| {
            let names = (0..s.columns().len()).map(|c| crate::code_rule_dsl::logic::plan::sql_col(name, c)).collect();
            (Arc::new(Renamed(Arc::clone(s), names)) as Arc<dyn Source>, None)
        })
    }
    pub(in crate::code_rule_dsl) fn relations(&self) -> usize {
        self.rels.len()
    }
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.rels.values().map(Kept::bytes).sum()
    }
    /// The index-cost spike (`RERAIL_INDEX_REPORT`): each kept relation's dedup set and indexes now, the key-column
    /// sets its probes bound (`probes`, by relation name: a stratum's own table, `+delta` for a delta probe), and
    /// what one index per probed key set would hold: a row id a row (4 B) and a directory (0.5 B), the dedup riding on the first such index (sorted by key hash then row), or a set of row ids (6 B) when none is
    /// probed; per-column statistics from sketches (256 B a column).
    pub(in crate::code_rule_dsl) fn index_report(&self, probes: &HashMap<(String, Vector<usize>), u64>) -> String {
        let base = |name: &str| -> String {
            name.split(" [").next().unwrap_or(name).trim_end_matches("+delta").to_owned()
        };
        let mut shapes: HashMap<String, BTreeMap<Vector<usize>, u64>> = HashMap::new();
        let mut reads: HashMap<String, BTreeMap<String, u64>> = HashMap::new();
        for ((name, cols), n) in probes {
            if let Some((_, kind)) = name.split_once(" [") {
                *reads.entry(base(name)).or_default().entry(kind.trim_end_matches(']').to_owned()).or_default() += n;
            } else if !cols.is_empty() {
                *shapes.entry(base(name)).or_default().entry(cols.clone()).or_default() += n;
            }
        }
        let mut lines = Vec::new();
        let mut totals = [("kept", 0usize, 0usize, 0usize, 0usize)];
        let mut dedup_only = (0usize, 0usize, 0usize, 0usize);
        let mut by_size: Vec<(&String, &Kept)> = self.rels.iter().collect();
        by_size.sort_by_key(|(_, k)| std::cmp::Reverse(k.rel.held_bytes().2));
        for (rel, kept) in by_size {
            let (k, set, chains, csr) = kept.rel.index_detail();
            let n = kept.rel.rows();
            // Spike (index levers): relations whose only order is the full-row one, the dedup test (`id_hash`'s case).
            dedup_only.0 += n;
            if chains.len() == 1 && chains[0].0.len() == k {
                dedup_only.1 += n;
                dedup_only.2 += 1;
                dedup_only.3 += chains[0].2;
            }
            let now = set + chains.iter().map(|c| c.2).sum::<usize>() + csr.iter().map(|c| c.1).sum::<usize>();
            let probed = shapes.get(rel.as_str());
            let sets = probed.map_or(0, BTreeMap::len);
            let proposed = sets * n * 9 / 2 + if sets == 0 { n * 6 } else { 0 } + k * 256;
            let t = &mut totals[0];
            t.1 += n;
            t.2 += now;
            t.3 += proposed;
            t.4 += sets;
            if lines.len() < 30 {
                let shown: Vec<String> = probed.map(|m| m.iter().map(|(c, x)| format!("{c:?}x{x}")).collect()).unwrap_or_default();
                let chain_cols: Vec<String> = chains.iter().map(|c| format!("{:?}:{}k", c.0, c.1 / 1000)).collect();
                lines.push(format!(
                    "  {rel}: {n} rows, k {k}; now {:.2} MB (set {:.2}, chains {}, lazy {}); probed {}; reads {:?}; proposed {:.2} MB ({:.1} B/row now, {:.1} proposed)",
                    now as f64 / 1e6,
                    set as f64 / 1e6,
                    chain_cols.join(" "),
                    csr.len(),
                    if shown.is_empty() { "none".to_owned() } else { shown.join(" ") },
                    reads.get(rel.as_str()),
                    proposed as f64 / 1e6,
                    now as f64 / n.max(1) as f64,
                    proposed as f64 / n.max(1) as f64
                ));
            }
        }
        let mut out: Vec<String> = totals
            .iter()
            .map(|(kind, n, now, proposed, sets)| {
                format!("INDEX {kind}: {n} rows; sets and indexes now {:.1} MB ({:.1} B/row), proposed {:.1} MB ({:.1} B/row); {sets} probed key sets", *now as f64 / 1e6, *now as f64 / (*n).max(1) as f64, *proposed as f64 / 1e6, *proposed as f64 / (*n).max(1) as f64)
            })
            .collect();
        out.push(format!("INDEX dedup-only: {} of {} kept rows in {} relations whose only order is the full row ({:.1} MB of orders)", dedup_only.1, dedup_only.0, dedup_only.2, dedup_only.3 as f64 / 1e6));
        out.extend(lines);
        // RERAIL_INDEX_SPIKE: every kept relation's orders built and measured; the 30 largest printed.
        if std::env::var_os("RERAIL_INDEX_SPIKE").is_some() {
            let mut by_size: Vec<(&String, &Kept)> = self.rels.iter().collect();
            by_size.sort_by_key(|(_, k)| std::cmp::Reverse(k.rel.held_bytes().2));
            let (mut total, mut rows, mut now, mut shown) = (0usize, 0usize, 0usize, 0usize);
            let mut weighted = (0.0f64, 0.0f64, 0.0f64);
            let mut placed = (0usize, 0usize);
            for (rel, kept) in by_size {
                let sets: Vec<Vector<usize>> = shapes.get(rel.as_str()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
                let counts: Vector<u64> = shapes.get(rel.as_str()).map(|m| m.values().copied().collect()).unwrap_or_default();
                if let Some((line, bytes, pbytes, dbytes, w_now, w_new, w_hash)) = kept.rel.index_spike(rel, &sets, &counts) {
                    placed.0 += pbytes;
                    placed.1 += dbytes;
                    weighted.0 += w_now;
                    weighted.1 += w_new;
                    weighted.2 += w_hash;
                    total += bytes;
                    rows += kept.rel.rows();
                    now += kept.rel.held_bytes().2;
                    if shown < 30 {
                        out.push(line);
                        shown += 1;
                    }
                }
            }
            let mixed = InRel::index_spike_mixed();
            let g = InRel::index_spike_grown();
            if g.2 > 0 {
                out.push(format!("SPIKE grown, all kept: {} offers, {} held; today {:.1} ms {:.1} B/row; mixed {:.1} ms (set {:.1}, placed {:.1}, orders {:.1}) {:.1} B/row; {} rows read again by doublings", g.2, g.3, g.0, g.4 as f64 / g.3.max(1) as f64, g.1, g.6, g.7, g.8, g.5 as f64 / g.3.max(1) as f64, g.14));
                let (v, orders, unprobed) = InRel::index_spike_variants();
                for (name, x) in ["eager (merged as they come)", "tagged (eager, a 16-bit tag an id)", "lazy (a run a wave, merged once where probes pay)", "lazy tagged", "tagged, interpolated probe", "lazy tagged, interpolated probe"].iter().zip(v) {
                    out.push(format!("SPIKE orders {name}: grow {:.1} ms, merge once {:.1} ms, probes {:.1} ms, total {:.1} ms; mean probe {:.0} ns; {} {}", x.0, x.2, x.1, x.0 + x.1 + x.2, x.3 / orders.max(1) as f64, x.4, if name.starts_with("lazy") { "orders merged once" } else if name.contains("interpolated") { "of the rows read in merges, tag collisions" } else { "ids merged or rows read in merges" }));
                }
                out.push(format!("SPIKE orders: {orders} orders, {unprobed} with no probe this step"));
                out.push(format!("SPIKE delta path, all kept ({} relations with a maintained run's rows, the rest their last wave): {} offers; today {:.1} ms + copy {:.1} ms; mixed {:.1} ms + copy {:.1} ms", g.15, g.9, g.10, g.11, g.12, g.13));
            }
            out.push(format!("SPIKE mixed (whole row placed, hot selective sets placed, the rest in orders): {:.1} MB ({:.1} B/row), {:.1} ms", mixed.0 as f64 / 1e6, mixed.0 as f64 / rows.max(1) as f64, mixed.1 / 1e6));
            out.push(format!("SPIKE all kept: {rows} rows; orders {:.1} MB ({:.1} B/row) vs sets and indexes {:.1} MB ({:.1} B/row); this step's probes of kept relations, at the measured costs: {:.1} ms by chains, {:.1} ms by fenced orders, {:.1} ms by placed indexes; placed on every probed set and the row {:.1} MB ({:.1} B/row), orders + placed row {:.1} MB ({:.1} B/row)", total as f64 / 1e6, total as f64 / rows.max(1) as f64, now as f64 / 1e6, now as f64 / rows.max(1) as f64, weighted.0 / 1e6, weighted.1 / 1e6, weighted.2 / 1e6, placed.0 as f64 / 1e6, placed.0 as f64 / rows.max(1) as f64, (total + placed.1) as f64 / 1e6, (total + placed.1) as f64 / rows.max(1) as f64));
        }
        let mut top: Vec<(&(String, Vector<usize>), &u64)> = probes.iter().collect();
        top.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        out.push(format!("  top probes: {}", top.iter().take(20).map(|((r, c), n)| format!("{r}{c:?}x{n}")).collect::<Vec<_>>().join(" ")));
        out.join("\n")
    }
    /// Each kept guest copy (`rr_in_<grammar>_<n>_<rel>`, decision 0058) against the relation it copies, as
    /// `(copy, rows, rows with a guest document's value, rows also in the original, the original's rows)`: a value is
    /// a guest's when its document key (`>> 32`) is in the guests' range (`hiding`). `RERAIL_COPIES` prints it.
    pub(in crate::code_rule_dsl) fn copies(&self) -> Vec<(String, usize, usize, usize, usize)> {
        let guest = |v: i64| (v >> 32) >= (1 << 30) && (v >> 32) < (1i64 << 31);
        let mut out = Vec::new();
        for (rel, k) in &self.rels {
            let Some(rest) = rel.strip_prefix("rr_in_") else { continue };
            // `<grammar>_<n>_` once or more (`sql_0_sql_1_`), then the relation.
            let mut parts: Vec<&str> = rest.split('_').collect();
            let mut i = 0;
            while i + 1 < parts.len() && parts[i + 1].chars().all(|c| c.is_ascii_digit()) && !parts[i + 1].is_empty() { i += 2; }
            let orig = parts.split_off(i).join("_");
            let Some(rows) = k.rel.live_rows() else { continue };
            let w = k.width.max(1);
            let held = self.rels.get(&orig).filter(|o| o.width == k.width).and_then(|o| o.rel.live_rows()).unwrap_or_default();
            let original: std::collections::HashSet<&[i64]> = held.chunks_exact(w).collect();
            let n = rows.len() / w;
            let guests = rows.chunks_exact(w).filter(|r| r.iter().any(|&v| guest(v))).count();
            let shared = rows.chunks_exact(w).filter(|r| original.contains(r)).count();
            out.push((rel.clone(), n, guests, shared, original.len()));
        }
        out.sort();
        out
    }
    /// What the kept relations hold: rows,
    /// values, levels, and indexes and sets, in bytes (`InRel::held_bytes`). `RERAIL_KEPT_BYTES` prints it.
    pub(in crate::code_rule_dsl) fn held(&self) -> [(&'static str, usize, usize, usize, usize); 1] {
        let mut out = [("kept", 0, 0, 0, 0)];
        for k in self.rels.values() {
            let (values, levels, index) = k.rel.held_bytes();
            let o = &mut out[0];
            o.1 += k.rel.rows();
            o.2 += values;
            o.3 += levels;
            o.4 += index;
        }
        out
    }
}

/// An input's source under another relation's column names (`r__old`'s, `sql_col`): read in place, not copied.
struct Renamed(Arc<dyn Source>, Vec<String>);

impl Renamed {
    /// The source's statistics under its new names.
    fn renamed(&self, stats: Vec<(String, i64, i64, i64, i64)>) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        let names = self.0.columns();
        stats.into_iter().map(|(c, a, b, x, y)| Some((self.1[names.iter().position(|n| *n == c)?].clone(), a, b, x, y))).collect()
    }
}

/// What `load` reads an input's old rows by, beside the documents the last run read: the documents whose content and
/// key are as they were (`same`), and each changed input's new rows, as the run binds them.
pub(in crate::code_rule_dsl) struct NewInputs<'a> {
    pub(in crate::code_rule_dsl) same: &'a rustc_hash::FxHashSet<i64>,
    pub(in crate::code_rule_dsl) source: &'a dyn Fn(&str) -> Option<Arc<dyn Source>>,
}

/// An input's rows as the last run had them, where most of its documents are as they were (0353: no copy): the new
/// rows of the documents that are the last run's, read in place in the run's own source, then the other documents'
/// old rows, read where they lie in their granules' kept entries (decision 0401 F3). Decoded whole, every input a stratum reached was copied into columns and hashed
/// row by row against its documents' digests (O125 step 05: 105 inputs, 1.6 s).
pub(in crate::code_rule_dsl) struct OldInput {
    names: Vec<String>,
    new: Arc<dyn Source>,
    /// The runs of `new`'s rows of the unchanged documents: each run's first row in `new`, and here.
    runs: Vec<(u32, u32)>,
    /// The rows read from `new`, before the changed documents' old rows.
    kept: usize,
    /// The changed documents' granules, bound where they lie: they hold other documents' rows too.
    changed: Arc<dyn Source>,
    /// The changed documents' rows in `changed`, each document's together: rows `kept..` here.
    pick: Vector<u32>,
    /// Each column's rows by value, made where first probed (the engine's lookups, as `Held`'s).
    idx: Vec<std::sync::OnceLock<rustc_hash::FxHashMap<i64, Vector<u32>>>>,
}

impl OldInput {
    /// Where row `row` lies: in `new` (`true`) or among the changed documents' rows, and its row there.
    fn at(&self, row: usize) -> (bool, usize) {
        if row >= self.kept {
            return (false, self.pick[row - self.kept] as usize);
        }
        let i = self.runs.partition_point(|&(_, here)| here as usize <= row) - 1;
        let (from, here) = self.runs[i];
        (true, from as usize + (row - here as usize))
    }
}

impl Source for OldInput {
    fn columns(&self) -> Vec<String> {
        self.names.clone()
    }
    fn len(&self) -> usize {
        self.kept + self.pick.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.new.estimated_matches(column, batch)
    }
    fn indexed(&self, _column: usize) -> bool {
        true
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        let idx = self.idx.get(column)?.get_or_init(|| {
            let mut by: rustc_hash::FxHashMap<i64, Vector<u32>> = rustc_hash::FxHashMap::default();
            for r in 0..self.len() {
                by.entry(self.value(r, column)).or_default().push(r as u32);
            }
            by
        });
        let mut out: rerail_blocks::vector::Vector<usize> = values.iter().flat_map(|v| idx.get(v).into_iter().flatten().map(|&r| r as usize)).collect();
        out.sort_unstable();
        out.dedup();
        Some(out)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        match self.at(row) {
            (true, r) => self.new.value(r, column),
            (false, r) => self.changed.value(r, column),
        }
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        match self.at(row) {
            (true, r) => self.new.row_at(r, columns, out),
            (false, r) => self.changed.row_at(r, columns, out),
        }
    }
}
impl Source for Renamed {
    fn columns(&self) -> Vec<String> {
        self.1.clone()
    }
    /// Its source's test: renamed, its columns are the same columns in the same order.
    fn holds(&self, equal: &[(usize, i64)]) -> Option<bool> {
        self.0.holds(equal)
    }
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.0.rows_of_documents(docs)
    }
    fn len(&self) -> usize {
        self.0.len()
    }
    fn estimated_len(&self) -> usize {
        self.0.estimated_len()
    }
    fn known_len(&self) -> Option<usize> {
        self.0.known_len()
    }
    fn may_hold_rows(&self) -> bool {
        self.0.may_hold_rows()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.0.estimated_matches(column, batch)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.0.lookup_cost(column)
    }
    fn scan_cost(&self) -> f64 {
        self.0.scan_cost()
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        self.0.estimated_matches_all(columns)
    }
    fn composite(&self) -> bool {
        self.0.composite()
    }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.0.lookup_all(equal)
    }
    fn doc_column(&self) -> Option<usize> {
        self.0.doc_column()
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.0.lookup_in(column, values, doc)
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.0.kept_to_doc(column)
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vector<u32> {
        self.0.filter_eq(eqs)
    }
    fn made_on_lookup(&self) -> bool {
        self.0.made_on_lookup()
    }
    fn complete_rows(&self) -> bool {
        self.0.complete_rows()
    }
    fn rows_owned(&self, g: usize, owned: &dyn Fn(i64) -> bool) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.0.rows_owned(g, owned)
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        self.0.expect_lookups(column, keys);
    }
    fn failure(&self) -> Option<String> {
        self.0.failure()
    }
    fn failed(&self) -> Option<String> {
        self.0.failed()
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.renamed(self.0.statistics()?)
    }
    fn estimated_statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.renamed(self.0.estimated_statistics()?)
    }
    fn indexed(&self, column: usize) -> bool {
        self.0.indexed(column)
    }
    fn range_upper_compatible(&self, lower: usize, upper: usize) -> bool {
        self.0.range_upper_compatible(lower, upper)
    }
    fn range_indexed(&self, column: usize) -> bool {
        self.0.range_indexed(column)
    }
    fn column_counts(&self, c: usize) -> Option<Vec<(i64, u64)>> {
        self.0.column_counts(c)
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.0.range_lookup(column, lower, upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.0.lookup(column, values)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.0.value(row, column)
    }
    fn sorted_rows(&self, column: usize, value: i64) -> Option<&[u32]> {
        self.0.sorted_rows(column, value)
    }
    fn sorted_range(&self, column: usize, value: i64) -> Option<std::ops::Range<u32>> {
        self.0.sorted_range(column, value)
    }
    fn sorts_rows(&self, column: usize) -> bool {
        self.0.sorts_rows(column)
    }
    fn prepare_lookup(&self, columns: &[usize]) {
        self.0.prepare_lookup(columns)
    }
    fn prepare_index(&self, columns: &[usize]) {
        self.0.prepare_index(columns)
    }
    fn primary_key_len(&self) -> Option<usize> {
        self.0.primary_key_len()
    }
    fn lookup_primary_batch(&self, values: &[Vector<i64>]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.0.lookup_primary_batch(values)
    }
    fn ordered(&self, equal: &[usize], order: &[(usize, bool)]) -> bool {
        self.0.ordered(equal, order)
    }
    fn ordered_range(&self, equal: &[(usize, i64)]) -> Option<std::ops::Range<usize>> {
        self.0.ordered_range(equal)
    }
}


/// A kept input with its delta beside it (decision 0117 step 2): the kept rows, read in place, less the rows of the
/// owners an edit changed, with their new rows after them. Nothing of the kept rows is copied: a row is found
/// through the kept rows' index and its place moved past the dead ranges before it.
pub(in crate::code_rule_dsl) struct Layered {
    names: Vec<String>,
    kept: Arc<dyn Source>,
    ranges: Arc<Ranges>,
    /// The changed owners' kept ranges, ascending: (first row, rows).
    dead: Vec<(u32, u32)>,
    /// Rows dead before each dead range (one more than `dead`: the last is all of them).
    before: Vector<u32>,
    added: Arc<dyn Source>,
    /// Each changed owner's rows in `added`.
    appended: rustc_hash::FxHashMap<i64, (u32, u32)>,
    /// Each owner's rows as a range of this input's rows (`ranges_now`), found where first asked.
    now: std::sync::OnceLock<Arc<Ranges>>,
}

impl Layered {
    /// `kept` (each owner's rows at `ranges`) with each owner of `changed` holding its new rows instead, named
    /// `names`.
    pub(in crate::code_rule_dsl) fn new(names: Vec<String>, kept: Arc<dyn Source>, ranges: Arc<Ranges>, changed: Vec<(i64, Vec<Vector<i64>>)>) -> Layered {
        let width = names.len();
        let mut dead: Vec<(u32, u32)> = changed.iter().filter_map(|(o, _)| ranges.get(o).copied()).filter(|(_, n)| *n > 0).collect();
        dead.sort_unstable();
        let mut before = rerail_blocks::vector::Vector::with_capacity(dead.len() + 1);
        let mut at = 0u32;
        before.push(0);
        for (_, n) in &dead {
            at += n;
            before.push(at);
        }
        let mut columns: Vec<Vector<i64>> = vec![rerail_blocks::vector::Vector::new(); width];
        let mut appended = rustc_hash::FxHashMap::default();
        for (owner, rows) in changed {
            let start = columns.first().map_or(0, |v| v.len()) as u32;
            for r in &rows {
                for (c, v) in r.iter().enumerate() {
                    columns[c].push(*v);
                }
            }
            appended.insert(owner, (start, rows.len() as u32));
        }
        let added = crate::code_rule_dsl::logic::native::component::columns_source(names.clone(), columns.into_iter().map(Some).collect());
        Layered { names, kept, ranges, dead, before, added, appended, now: std::sync::OnceLock::new() }
    }

    /// Each owner's rows as a range of this input's rows, as they are numbered now: a kept owner's at its live place,
    /// a changed one's among the appended rows.
    pub(in crate::code_rule_dsl) fn ranges_now(&self) -> &Arc<Ranges> {
        self.now.get_or_init(|| {
            let mut out = Ranges::default();
            out.reserve(self.ranges.len() + self.appended.len());
            for (&o, &(start, n)) in self.ranges.iter() {
                if n == 0 || self.appended.contains_key(&o) {
                    continue;
                }
                if let Some(at) = self.live_of_kept(start as usize) {
                    out.insert(o, (at as u32, n));
                }
            }
            let live = self.live_kept() as u32;
            for (&o, &(start, n)) in &self.appended {
                if n > 0 {
                    out.insert(o, (live + start, n));
                }
            }
            Arc::new(out)
        })
    }

    /// The changed owners' new rows, one after another (`OwnerPatch::plus` names its rows by their places here).
    pub(in crate::code_rule_dsl) fn added(&self) -> &Arc<dyn Source> {
        &self.added
    }

    fn live_kept(&self) -> usize {
        self.kept.len() - *self.before.last().expect("one at least") as usize
    }

    /// The kept row at live place `i` (below `live_kept`).
    fn to_kept(&self, i: usize) -> usize {
        // The dead ranges wholly before live place `i`: those whose first row's live place is at or below it.
        let (mut lo, mut hi) = (0, self.dead.len());
        while lo < hi {
            let m = (lo + hi) / 2;
            if ((self.dead[m].0 - self.before[m]) as usize) <= i { lo = m + 1 } else { hi = m }
        }
        i + self.before[lo] as usize
    }

    /// Kept row `r`'s live place, unless it is dead.
    fn live_of_kept(&self, r: usize) -> Option<usize> {
        let k = self.dead.partition_point(|&(s, _)| (s as usize) <= r);
        if k > 0 && r < (self.dead[k - 1].0 + self.dead[k - 1].1) as usize {
            return None;
        }
        Some(r - self.before[k] as usize)
    }

    /// Owner `owner`'s rows, flat: its new rows if it changed, else its kept ones.
    /// Where `owner`'s rows lie: the source that holds them (its new rows', or the kept ones'), their first row
    /// there and how many.
    pub(in crate::code_rule_dsl) fn owner_range(&self, owner: i64) -> (&Arc<dyn Source>, u32, u32) {
        match self.appended.get(&owner) {
            Some(&(start, n)) => (&self.added, start, n),
            None => {
                let (start, n) = self.ranges.get(&owner).copied().unwrap_or((0, 0));
                (&self.kept, start, n)
            }
        }
    }

    #[cfg(test)]
    fn owner_rows(&self, owner: i64) -> Vector<i64> {
        let width = self.names.len();
        let all: Vector<usize> = (0..width).collect();
        let (from, start, n) = self.owner_range(owner);
        let mut out = rerail_blocks::vector![0i64; width * n as usize];
        for (i, r) in (start..start + n).enumerate() {
            from.row_at(r as usize, &all, &mut out[i * width..(i + 1) * width]);
        }
        out
    }
}

impl Source for Layered {
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
    fn columns(&self) -> Vec<String> {
        self.names.clone()
    }
    fn len(&self) -> usize {
        self.live_kept() + self.added.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.kept.estimated_matches(column, batch)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.kept.lookup_cost(column)
    }
    fn scan_cost(&self) -> f64 {
        self.kept.scan_cost()
    }
    fn indexed(&self, column: usize) -> bool {
        self.kept.indexed(column)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        let live = self.live_kept();
        let mut out: rerail_blocks::vector::Vector<usize> = self.kept.lookup(column, values)?.into_iter().filter_map(|r| self.live_of_kept(r)).collect();
        out.extend(self.added.lookup(column, values)?.into_iter().map(|r| r + live));
        Some(out)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let live = self.live_kept();
        if row < live { self.kept.value(self.to_kept(row), column) } else { self.added.value(row - live, column) }
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        let live = self.live_kept();
        if row < live { self.kept.row_at(self.to_kept(row), columns, out) } else { self.added.row_at(row - live, columns, out) }
    }
}

#[cfg(test)]
pub mod tests {
    /// A program's version, as a run reads it (t36 item 7).
    fn pl(p: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Placed { Placed::whole(p.clone()) }
    /// A version's strata, each by its relations' names (the `Strata` analysis).
    fn strata_of(v: &Arc<crate::code_rule_dsl::logic::program_image::Version>) -> crate::code_rule_dsl::logic::front::Components {
        let s = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::logic::program_image::Strata>(v).unwrap();
        crate::code_rule_dsl::logic::front::Components::of_names(s.sccs.iter().map(|scc| scc.iter().map(|&r| v.rel_name(r))))
    }
    use super::*;

    /// A semi-join's rows at the held nodes read back as kept: the exports' digest they were taken of (or none), the
    /// nodes walked, a node with no row among them, and each row; a width of 0, an unknown flag or rows that are not
    /// whole are not read as kept.
    #[test]
    fn a_semijoins_kept_rows_read_back_as_written() {
        let node = |d: i64, i: i64| d << 32 | i;
        let kept = SemijoinRows { exports: None, stats: vec![[3, 2, 5, 5], [3, 3, 3, 3]], width: 3, held: vec![node(1, 0), node(1, 1), node(2, 4)], rows: vec![1 << 32, node(1, 0), 7, 1 << 32, node(1, 1), 9] };
        assert_eq!(SemijoinRows::decode(&kept.encode()), Some(kept.clone()));
        let known = SemijoinRows { exports: Some(u64::MAX - 3), ..kept.clone() };
        assert_eq!(SemijoinRows::decode(&known.encode()), Some(known.clone()));
        let mut words = kept.encode();
        words.pop();
        assert_eq!(SemijoinRows::decode(&words), None);
        let mut flag = known.encode();
        flag[0] = 2;
        assert_eq!(SemijoinRows::decode(&flag), None);
        assert_eq!(SemijoinRows::decode(&[0, 0, 0, 0, 0]), None);
        assert_eq!(SemijoinRows::decode(&[0, 0, 9, 1]), None);
    }

    /// The exports' digest is of each export's digest in the exports' order, and none when one has no digest.
    #[test]
    fn the_exports_digest_is_of_every_exports_digest() {
        let exports = vec!["a".to_owned(), "b".to_owned()];
        let digests: HashMap<String, u64> = [("a".to_owned(), 1), ("b".to_owned(), 2)].into_iter().collect();
        let d = exports_digest(&exports, &digests).expect("both digested");
        let mut other = digests.clone();
        other.insert("b".to_owned(), 3);
        assert_ne!(exports_digest(&exports, &other), Some(d));
        let swapped: HashMap<String, u64> = [("a".to_owned(), 2), ("b".to_owned(), 1)].into_iter().collect();
        assert_ne!(exports_digest(&exports, &swapped), Some(d));
        other.remove("b");
        assert_eq!(exports_digest(&exports, &other), None);
    }

    /// A kept input with its delta reads as its rows now: the changed owners' kept rows gone, their new rows after
    /// the rest, and a lookup finds a row through the kept rows' index at its place now.
    #[test]
    fn a_layered_input_reads_its_kept_rows_less_the_changed_owners_with_their_new_rows() {
        let node = |d: i64, i: i64| d << 32 | i;
        let rows = [[node(1, 0), 10], [node(1, 1), 11], [node(2, 0), 20], [node(2, 1), 21], [node(3, 0), 30], [node(4, 0), 40]];
        let kept = crate::code_rule_dsl::logic::native::component::columns_source(vec!["n".into(), "v".into()], (0..2).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect());
        let ranges: Ranges = [(1, (0, 2)), (2, (2, 2)), (3, (4, 1)), (4, (5, 1))].into_iter().collect();
        // Owner 2 edited, 3 gone, 5 new.
        let l = Layered::new(vec!["n".into(), "v".into()], kept, Arc::new(ranges), vec![(2, vec![rerail_blocks::vector![node(2, 0), 22]]), (3, Vec::new()), (5, vec![rerail_blocks::vector![node(5, 0), 50]])]);
        let read: Vec<(i64, i64)> = (0..l.len()).map(|r| (l.value(r, 0), l.value(r, 1))).collect();
        assert_eq!(read, vec![(node(1, 0), 10), (node(1, 1), 11), (node(4, 0), 40), (node(2, 0), 22), (node(5, 0), 50)]);
        for (r, (n, v)) in read.iter().enumerate() {
            assert_eq!(l.lookup(1, &[*v]), Some(rerail_blocks::vector![r]));
            assert!(l.lookup(0, &[*n]).unwrap().contains(&r));
        }
        assert_eq!(l.lookup(1, &[20, 21, 30]), Some(rerail_blocks::vector::Vector::new()));
        assert_eq!(l.owner_rows(2), vec![node(2, 0), 22]);
        assert_eq!(l.owner_rows(1), vec![node(1, 0), 10, node(1, 1), 11]);
        assert!(l.owner_rows(3).is_empty());
    }

    /// Only the documents whose rows differ are compared, and their rows new and gone are exactly the diff: with
    /// the old side's hashes given (as a session's next run has them) or taken.
    #[test]
    fn a_document_delta_is_the_diff_of_the_documents_that_changed() {
        let node = |d: i64, i: i64| d << 32 | i;
        let source = |rows: &[[i64; 2]]| {
            crate::code_rule_dsl::logic::native::component::columns_source(vec!["n".into(), "v".into()], vec![Some(rows.iter().map(|r| r[0]).collect()), Some(rows.iter().map(|r| r[1]).collect())])
        };
        // Document 1 unchanged (its rows in another order), 2 edited, 3 gone, 4 new.
        let old = source(&[[node(1, 0), 10], [node(1, 1), 11], [node(2, 0), 20], [node(2, 1), 21], [node(3, 0), 30]]);
        let new = source(&[[node(2, 0), 20], [node(2, 1), 22], [node(1, 1), 11], [node(1, 0), 10], [node(4, 0), 40]]);
        let rows = |s: &Arc<dyn Source>| (0..s.len()).map(|r| vec![s.value(r, 0), s.value(r, 1)]).collect::<BTreeSet<_>>();
        let ((plus, minus), docs) = document_delta("r", &old, None, &new, 0, &crate::code_rule_dsl::logic::plan::Cells::plain(2)).unwrap();
        assert_eq!(rows(&plus), BTreeSet::from([vec![node(2, 1), 22], vec![node(4, 0), 40]]));
        assert_eq!(rows(&minus), BTreeSet::from([vec![node(2, 1), 21], vec![node(3, 0), 30]]));
        // The next run: `new` is old, its hashes kept; nothing changed.
        let ((plus, minus), _) = document_delta("r", &new, Some(&docs), &source(&[[node(1, 0), 10], [node(1, 1), 11], [node(2, 0), 20], [node(2, 1), 22], [node(4, 0), 40]]), 0, &crate::code_rule_dsl::logic::plan::Cells::plain(2)).unwrap();
        assert_eq!((plus.len(), minus.len()), (0, 0));
    }

    #[test]
    fn unchanged_gated_programs_reuse_only_complete_retained_results() {
        use crate::code_rule_dsl::logic::plan::corpus::preparation::{magic::ProgramDemand, prune::Prune};
        use crate::code_rule_dsl::logic::program_image::{fields::ProgramFields, manager::Pipeline};
        let p = crate::code_rule_dsl::logic::rr::parse_version(include_str!("../../../../../../../src/code_rule_dsl/logic/plan/golden/programs/workload.rerail")).unwrap();
        let mut pipeline = Pipeline::new("gated demand", p);
        pipeline.program(&mut Prune::default());
        let mut demand = ProgramDemand { every: true, gate: true, ..ProgramDemand::default() };
        pipeline.program(&mut demand);
        let p = pipeline.finish().unwrap().0;
        assert!(matches!(demand.outcome, Some(Ok(_))) && !ProgramFields::gates(&*p).is_empty());
        let strata = strata_of(&p);
        let key=[1;32];
        let inputs:HashMap<_,_>=p.input_names().into_iter().map(|r|(r,7)).collect();
        let rels=strata.iter().flatten().map(|name| {
            let width=p.columns_of(name).unwrap().len();
            (name.to_owned(),Kept { rel:InRel::flat(&[],vec![b'i';width]),width,output:None })
        }).collect();
        let mut kept=Retained {key,inputs:inputs.clone(),rels,input_srcs:Default::default(),input_layouts:Default::default(),framed_ranges:Default::default(),measured:Default::default(),input_docs:Default::default(),docs:Default::default(),frames:None,admitted:Default::default(),late:Default::default(),late_stats:Default::default(),past:Default::default(),views:Default::default()};
        let result=cut(Some(&kept),key,&Placed::whole(p.clone()),&strata,&inputs).unwrap();
        assert_eq!(result.strata,(0,strata.len()));
        let lowered=lower::Lowered {version:p.clone(),domains:Default::default(),ops:[0;3],dialect:crate::code_rule_dsl::language::GrammarDialect::Python,normal:Default::default()};
        let reused=result.program(&lowered).unwrap();
        let placed=reused.placed().unwrap();
        // A view of an input's columns (Q37: `kind(n, k) :- node(_, n, k, _, _).`) is never kept, and every reader
        // of these is clean: not made (`Cut::idle`, t33 K4). Nothing runs.
        let left:BTreeSet<String>=placed.v.rules().map(|r| placed.v.rel_name(r.head().rel()).to_owned()).collect();
        assert!(left.is_empty(),"{left:?}");
        assert_eq!(result.idle,BTreeSet::from(["docof".to_owned(),"kind".to_owned()]));
        assert!(result.views.iter().all(|v| result.idle.contains(v)));
        assert!(placed.fields().unwrap().gates().is_empty());
        let mut changed=inputs.clone();
        *changed.values_mut().next().unwrap()=8;
        for (candidate,rows) in [(key,changed),([2;32],inputs.clone())] {
            assert_eq!(cut(Some(&kept),candidate,&Placed::whole(p.clone()),&strata,&rows).unwrap().strata,(strata.len(),strata.len()));
        }
        // A kept relation missing (not a view's: those are not kept, so their absence changes nothing).
        let missing=kept.rels.keys().find(|r| !result.idle.contains(r.as_str())).unwrap().clone();
        kept.rels.remove(&missing);
        assert_eq!(cut(Some(&kept),key,&Placed::whole(p.clone()),&strata,&inputs).unwrap().strata,(strata.len(),strata.len()));
    }
}

/// Granules an input's frames are split into, by document (K1's partition): an edit rewrites the granules of the
/// documents it changed, not the input.
const GRANULES: i64 = 64;
/// The part of an input's frames that holds each document's digest (`DocDigests`), as rows (document, sum, rows);
/// its frame's digest is the input's (`digest`). Granule `g` is part `1 + g`.
const DOCS_PART: i64 = 0;

fn digest_words(d: &[u8; 32]) -> [i64; 4] {
    std::array::from_fn(|i| i64::from_le_bytes(d[i * 8..i * 8 + 8].try_into().expect("8 bytes")))
}

fn words_digest(w: &[i64]) -> [u8; 32] {
    let mut d = [0u8; 32];
    for (i, v) in w.iter().enumerate().take(4) {
        d[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
    }
    d
}

/// An input's documents as its frame keeps them, decoded and checked where first asked (Q91): a run whose inputs
/// did not change reads only their digests. `None` where the frame does not decode or match its digest: the input
/// is then read as one with no frame.
pub(in crate::code_rule_dsl) struct FramedDocs {
    /// `None` for documents a run hashed itself (`ready`).
    frame: Option<kept_read::Entry>,
    digest: u64,
    /// A relation of no columns: its frame is its count (`write_kept`), one entry whose digest is not its rows'.
    counted: bool,
    docs: std::sync::OnceLock<Option<Arc<Docs>>>,
}

impl FramedDocs {
    /// Documents a run hashed itself, held as they are.
    fn ready(docs: Arc<Docs>) -> Arc<FramedDocs> {
        Arc::new(FramedDocs { frame: None, digest: 0, counted: false, docs: std::sync::OnceLock::from(Some(docs)) })
    }
    pub(in crate::code_rule_dsl) fn get(&self) -> Option<&Arc<Docs>> {
        self.docs
            .get_or_init(|| {
                let _span = crate::code_rule_dsl::logic::profile::span(|| "an input's documents, from its kept entry where first asked".into());
                // The entry read where it lies, found by its documents' search (decision 0353 step 8).
                Docs::kept(self.frame.as_ref()?, (!self.counted).then_some(self.digest)).map(Arc::new)
            })
            .as_ref()
    }
}

/// The statistics entry kept with `rel`'s rows (decision 0120 §3), when it is of the rows the run's cut read
/// (`InKept::derived`'s digest): decoded from its words.
fn kept_stats_of(frames: &InKept, rel: &str) -> Option<kept_stats::ByDigest> {
    let d = frames.derived.get(rel)?.digest;
    let name = frames.names.get(rel)?;
    let entry = frames.bound.as_ref()?.entry(&name.name, &name.ident, kept_stats::PART).filter(|e| e.digest() == d)?;
    // Its words read where they lie (decision 0353 step 8): one column of them.
    let rows = entry.packed();
    if rows.arity != 1 {
        return None;
    }
    kept_stats::from_words(rows.len(), &|i| rows.get(i, 0))
}

/// What an input's documents frame records beside its digest (an input's frame has no reads of its own): the
/// granules its rows lie in, a bit each (`granule_bit`).
const GRANULES_READ: &str = "#granules";
/// An input kept as its documents' digest alone, no rows (decision 0458: a base relation, read off the corpus batch):
/// marked on its documents' entry. A run takes its old digest from it, and its versions from the documents.
const DIGEST_ONLY_READ: &str = "#digest-only";

/// Granule `g`'s bit in `GRANULES_READ`'s mask (granules are `1..=GRANULES`).
fn granule_bit(g: i64) -> u64 {
    1u64 << ((g - 1) as u32 & 63)
}

/// The granules `GRANULES_READ`'s mask names, ascending.
fn granules_of(mask: u64) -> Vector<i64> {
    (1..=GRANULES).filter(|g| mask & granule_bit(*g) != 0).collect()
}
/// What the store's frames hold of a cross-file part (t33 K4), read before the run's cut: the derived relations
/// stored and current, and each input's digest by document. Rows are read only for what the run then reads.
#[derive(Clone, Default)]
pub(in crate::code_rule_dsl) struct InKept {
    space: [u8; 32],
    /// The space's kept blocks as the run read them before its first wave (decision 0401 F3): every entry it binds
    /// is of this commit of them.
    bound: Option<Arc<kept_read::Bound>>,
    /// Each relation and input of the program, as its frame names it.
    names: HashMap<String, KeptRel>,
    /// Each derived relation stored and current: its digest and the digest each of its reads had.
    derived: HashMap<String, KeptStored>,
    /// Each input as stored: its digest, and its rows' digest by document (decoded where first asked, Q91).
    inputs: HashMap<String, (u64, Arc<FramedDocs>)>,
    /// The inputs whose owners are atoms (`atom_owners`, as their documents' table types them): their granules are
    /// by content (`granule`).
    atom_owned: std::collections::HashSet<String>,
    /// The relations admitted with their statistics part at their rows' digest (decision 0120 §3): written again
    /// only if their rows change.
    stats_kept: std::collections::HashSet<String>,
    /// The inputs kept as their digest alone (`DIGEST_ONLY_READ`): no granules to read.
    digest_only: std::collections::HashSet<String>,
}

impl InKept {
    /// The digest input `rel` is kept at, when it is kept.
    pub(in crate::code_rule_dsl) fn input_digest(&self, rel: &str) -> Option<u64> {
        self.inputs.get(rel).map(|(d, _)| *d)
    }
    /// Whether every input of `measured` (statistics a run measured, `write_input_kept`'s `input_stats`) has them
    /// kept at the rows of its digest in `digests`: a run that put its inputs as they were then puts nothing.
    pub(in crate::code_rule_dsl) fn holds_all_stats<V>(&self, measured: &HashMap<String, V>, digests: &HashMap<String, u64>) -> bool {
        measured.keys().all(|rel| self.names.get(rel).zip(digests.get(rel)).is_some_and(|(name, d)| self.holds_stats(name, *d)))
    }
    /// Whether a statistics entry (`kept_stats::PART`) of the rows of digest `digest` is kept under `name`.
    fn holds_stats(&self, name: &KeptRel, digest: u64) -> bool {
        self.bound.as_ref().and_then(|b| b.entry(&name.name, &name.ident, kept_stats::PART)).is_some_and(|e| e.digest() == digest)
    }
    /// The digest of `rel`'s stored frame, when it is stored and current.
    pub(in crate::code_rule_dsl) fn derived_digest(&self, rel: &str) -> Option<u64> {
        self.derived.get(rel).map(|s| s.digest)
    }

    /// Whether `other` holds the same kept rows: a revision's rounds (`corpus`, `cone_rounds::On::alternate`) are
    /// written into its space too only where they differ.
    pub(super) fn same_rows(&self, other: &Self) -> bool {
        // By digests: an input's documents are its digest (`FramedDocs` holds them as read), a derived relation its own.
        self.inputs.len() == other.inputs.len()
            && self.inputs.iter().all(|(rel, (digest, _))| other.inputs.get(rel).is_some_and(|(d, _)| d == digest))
            && self.derived.len() == other.derived.len()
            && self.derived.iter().all(|(rel, s)| other.derived.get(rel).is_some_and(|o| o.digest == s.digest))
    }
}

/// The digest of `read` a derived relation records (t33 K4): as `known` has it (an input as the run bound it, a
/// relation as derived or named), or, for the program's stated facts, their identity, which is the program's.
/// Anything else (an answer the run binds by another way, as a load's) has none: what reads it is not current.
fn read_digest(read: &str, known: &HashMap<String, u64>, derived: &BTreeSet<&str>, prog: &Placed, leaves: &crate::code_rule_dsl::logic::plan::Leaves) -> Option<u64> {
    if let Some(d) = known.get(read) {
        return Some(*d);
    }
    // A relation with literal rows that is also an input is named by its bound rows' digest, never by its literals.
    if derived.contains(read) || prog.fields().ok()?.is_input(read) {
        return None;
    }
    // Neither derived nor an input, and not bound: its rows are the program's own (its facts, or none, as an
    // earlier phase's export of no rows), the same whatever the run reads, so its identity names them.
    let id = leaves.identity(read)?;
    Some(u64::from_le_bytes(id[..8].try_into().expect("eight bytes")))
}

/// The digest of a derived relation with no rows kept, from the digests of what it reads: the same reads derive
/// the same rows.
fn unkept_digest(rel: &str, reads: &[(String, u64)]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in b"unkept\0".iter().chain(rel.as_bytes()) {
        h = (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    for (r, d) in reads {
        for b in r.as_bytes().iter().chain(&[0]) {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h = (h ^ d).wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 29;
    }
    h
}

/// An input's identity: its name, its columns and the program that binds it (`key`: `keys`' second, of the part's
/// program alone). Each program's input is a frame
/// of its own (phase 1's `doc` and the pack's are not one frame each overwrites with its rows), while a derived
/// relation is named by its rules and what they read (`plan::identities`), so two programs that derive it from
/// inputs of the same digests share it (decision 0103).
fn input_ident(rel: &str, prog: &Placed, key: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    // v3: an input's rows grouped by owner within each granule (decision 0117 step 2), so each owner's are one range.
    h.update(b"rerail.relation/v3\0input\0");
    h.update(rel.as_bytes());
    // The declaration as the tree's `decls` held it, in the same form (a stored frame's name is of it).
    let decl: Option<front::List<(front::Name, front::Name)>> = prog.fields().ok().and_then(|f| f.columns(rel)).map(|cs| cs.into_iter().map(|(n, t)| (n.into(), t.into())).collect());
    h.update(format!("\0{:?}\0", decl.as_ref()).as_bytes());
    h.update(key);
    h.finalize().into()
}

/// What names a program's frames, for the key its identities are kept under (`kept_identities`): its version (its
/// rules in order, its declarations, facts, inputs and outputs) and its digest.
fn program_key(h: &mut sha2::Sha256, p: &Placed) {
    use sha2::Digest as _;
    h.update(crate::code_rule_dsl::logic::program_image::manager::identity(&p.v));
    h.update(p.digest().to_le_bytes());
}

/// Identities kept as a stored fact under `key` (`parts::keep_facts`): read back where kept, else made by `make` and
/// kept. The key names everything they are made of (the programs, the binds, the engine), so a program the run held
/// before reads its identities rather than hashing its strata again (Q19's identities, decision 0379).
/// `named`: the source of the facts read, for the run's manifest (`kept_facts_alone`).
fn kept_identities(store: &ContentStore, what: &str, key: [u8; 32], named: &mut Vec<crate::content_store::SourceId>, make: impl FnOnce() -> Result<Vec<(String, [u8; 32])>>) -> Result<Vec<(String, [u8; 32])>> {
    use super::preparation::parts::{hex, keep_facts, kept_facts_alone, rows_of, str};
    let kept = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("{what}: kept"));
        kept_facts_alone(store, &key).and_then(|(f, source)| rows_of(&f, "frame_ident").map(|r| Some((r.first()?.clone(), super::preparation::parts::unhex(r.get(1)?)?))).collect::<Option<Vec<_>>>().map(|k| (k, source)))
    };
    if let Some((k, source)) = kept {
        named.push(source);
        return Ok(k);
    }
    let made = make()?;
    let mut facts = crate::code_rule_dsl::logic::program_facts::Facts::new();
    facts.insert("frame_ident".to_owned(), made.iter().map(|(name, ident)| vec![str(name), str(&hex(ident))].into()).collect());
    keep_facts(store, &key, &facts);
    Ok(made)
}

/// A program's derived relations' identities (`plan::identities`, by its components), as `Retained::bind_kept`
/// names them.
pub(in crate::code_rule_dsl) type StrataIdents = Arc<HashMap<String, [u8; 32]>>;

/// `p`'s strata identities (`identities_by_heads` over `sccs`), kept as a stored fact under the program and its
/// components. Read before the first wave only (decision 0104): a reader inside the waves is handed them.
fn strata_idents_kept(store: &ContentStore, p: &Placed, sccs: &crate::code_rule_dsl::logic::front::Components, named: &mut Vec<crate::content_store::SourceId>) -> Result<StrataIdents> {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail/strata identities v1\0");
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    program_key(&mut h, p);
    for scc in sccs {
        for r in scc {
            h.update(r.as_bytes());
            h.update([0]);
        }
        h.update([1]);
    }
    let leaves = crate::code_rule_dsl::logic::plan::Leaves::of_placed(p.clone());
    let made = kept_identities(store, "a program's strata identities", h.finalize().into(), named, || Ok(leaves.identities_by_heads(sccs)?.into_iter().collect()))?;
    Ok(Arc::new(made.into_iter().collect()))
}

/// Each of `progs`' strata identities (`strata_idents_kept`), read before the first wave (decision 0104), for
/// `Retained::bind_kept_named`: what the part's kept entries are named by. `named`: the sources of the kept facts
/// read, for the run's manifest.
pub(in crate::code_rule_dsl) fn strata_kept(store: &ContentStore, progs: &[&Placed], named: &mut Vec<crate::content_store::SourceId>) -> Result<Vec<StrataIdents>> {
    progs.iter().map(|p| strata_idents_kept(store, p, &*p.components()?, named)).collect()
}

/// A relation's column types, as frames store them.
pub(super) fn types_of(prog: &Placed, domains: &lower::Domains, rel: &str) -> Vec<u8> {
    let types = prog.fields().ok().and_then(|f| f.columns(rel)).map(|cs| cs.into_iter().map(|(_, t)| t).collect::<Vec<_>>()).unwrap_or_default();
    lower::atom_columns(&types, domains, rel).into_iter().map(|atom| if atom { b's' } else { b'i' }).collect()
}

/// The column types a frame keeps relation `rel`'s rows under, held by the engine as `engine`: its declaration's
/// (`types_of`), whose `s` columns the store maps for a forked process (decision 0170); the engine's when the rows are
/// not of its declared width.
pub(super) fn frame_types(prog: &Placed, domains: &lower::Domains, rel: &str, engine: &[u8]) -> Vec<u8> {
    let declared = types_of(prog, domains, rel);
    if declared.len() == engine.len() { declared } else { engine.to_vec() }
}

/// A kept relation's column types as the engine holds them: a corpus program's symbols are atoms, in integer columns
/// (O13), which its frame types `s` (`frame_types`).
pub(super) fn engine_types(stored: Vec<u8>) -> Vec<u8> {
    stored.into_iter().map(|t| if t == b's' { b'i' } else { t }).collect()
}

/// Each relation's location columns, as the split found them (`Split::locations`).
pub(in crate::code_rule_dsl) type Locations = BTreeMap<String, Vector<usize>>;

/// The column whose document owns an input's rows (decision 0117 step 2): its first location column (`locations`),
/// the column the memo attributes an export's rows by; else its first node column; else the column `memo::owner`
/// names (a string's, for `value_type`); else the first. Every row of one owner is held by the same files, and the
/// frames keep an input's rows grouped by it, so an edit reads only the owners it changed. The first node column is
/// not always a location: `p197_arg_at(d, i, a)` has `a` from another file in one rule, so the memo attributes its rows
/// by `d`, and frames grouped by `a` refused every edit's kept inputs.
pub(in crate::code_rule_dsl) fn group_col(prog: &Placed, locations: &Locations, rel: &str) -> usize {
    let decl = prog.fields().ok().and_then(|f| f.columns(rel));
    let width = decl.as_ref().map_or(0, |c| c.len());
    locations.get(rel).and_then(|l| l.first().copied()).filter(|c| *c < width)
        .or_else(|| decl.as_ref().and_then(|c| c.iter().position(|(_, t)| *t == "node")))
        .or_else(|| crate::code_rule_dsl::typing::file_local::document_column(rel).filter(|c| *c < width))
        .or_else(|| match super::memo::owner(rel) {
            super::memo::Owner::Node(c) | super::memo::Owner::Text(c) => Some(c).filter(|c| *c < width),
        })
        .unwrap_or(0)
}

/// A row's owner: the document its group column names (`group_col`).
pub(in crate::code_rule_dsl) fn owner_of(row: &[i64], g: usize) -> i64 {
    super::memo::document_of(row[g])
}

/// A digest from documents' digests, as `digest` takes it of all their rows.
pub(in crate::code_rule_dsl) fn digest_of_docs(docs: &DocDigests) -> u64 {
    let (sum, n) = docs.values().fold((0u64, 0u64), |(s, n), (ds, dn)| (s.wrapping_add(*ds), n + dn));
    sum ^ n.wrapping_mul(0xff51_afd7_ed55_8ccd)
}

/// A source's rows' digest by owner (`group_col` `g`), as `document_delta` takes it: of their content (`Cells`).
pub(in crate::code_rule_dsl) fn docs_of(source: &dyn Source, g: usize, cells: &Cells) -> DocDigests {
    let mut by = DocDigests::default();
    each_row(source, cells, |row, h| {
        let e = by.entry(owner_of(row, g)).or_default();
        e.0 = e.0.wrapping_add(h);
        e.1 += 1;
    });
    by
}

/// An input of the cross part as the last run kept it (decision 0117 step 1): its rows, named as the program
/// declares them, its rows' digests by owner, its digest, and each owner's rows as a range of them (step 2).
pub(in crate::code_rule_dsl) struct KeptInput {
    pub(in crate::code_rule_dsl) source: Arc<dyn Source>,
    /// Its documents and each owner's range, laid out where first asked (Q91).
    pub(in crate::code_rule_dsl) layout: Arc<InputLayout>,
    pub(in crate::code_rule_dsl) digest: u64,
    /// Its statistics as a run measured them at these rows (`digest`), kept beside its frame (decision 0120 §3's
    /// part): asked only for an input `kept_inputs` is told to (`stats_of`), whose statistics no keeper counts.
    pub(in crate::code_rule_dsl) stats: Option<Vec<(String, i64, i64, i64, i64)>>,
}

/// Each owner's rows in a kept input: (first row, rows).
pub(in crate::code_rule_dsl) type Ranges = rustc_hash::FxHashMap<i64, (u32, u32)>;

impl KeptInput {
    /// An input whose documents and ranges are held already (a test's).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn laid(source: Arc<dyn Source>, docs: DocDigests, ranges: Ranges) -> KeptInput {
        let digest = digest_of_docs(&docs);
        let laid = LaidOut { docs: Arc::new(Docs::Made(docs)), ranges: Arc::new(ranges), owners: Vec::new() };
        let layout = InputLayout { rel: String::new(), frame: None, digest, atoms: None, granules: Vec::new(), laid: std::sync::OnceLock::from(Ok(laid)) };
        KeptInput { source, layout: Arc::new(layout), digest, stats: None }
    }
    /// Its rows' digests by owner (its documents' frame, decoded and checked where first asked).
    pub(in crate::code_rule_dsl) fn docs(&self) -> std::result::Result<&Arc<Docs>, String> {
        Ok(&self.layout.laid_out()?.docs)
    }
    /// Each owner's rows as a range of its rows.
    pub(in crate::code_rule_dsl) fn ranges(&self) -> std::result::Result<&Arc<Ranges>, String> {
        Ok(&self.layout.laid_out()?.ranges)
    }
}

/// A kept input's layout as its documents' frame holds it (`kept_inputs`): which granules its rows lie in and how
/// many each holds are known without decoding (the keeper records the granules, `GRANULES_READ`, and each granule's
/// frame its count); its documents, each owner's range and each granule's owners are decoded where first asked. A
/// rule edit reads none of its owners, and decodes no documents' frame (Q91).
pub(in crate::code_rule_dsl) struct InputLayout {
    rel: String,
    /// `None` for one laid out already (`KeptInput::laid`).
    frame: Option<kept_read::Entry>,
    digest: u64,
    atoms: Option<Arc<crate::content_store::atoms::Atoms>>,
    /// Each granule its rows lie in, and its rows' count, ascending.
    granules: Vec<(i64, usize)>,
    laid: std::sync::OnceLock<std::result::Result<LaidOut, String>>,
}

/// A kept input's documents, ranges and each granule's owners (`InputLayout::laid_out`).
pub(in crate::code_rule_dsl) struct LaidOut {
    docs: Arc<Docs>,
    ranges: Arc<Ranges>,
    /// Each granule (in `InputLayout::granules`' order): its owners and their rows' counts, owners ascending.
    owners: Vec<Vec<(i64, u32)>>,
}

impl InputLayout {
    fn laid_out(&self) -> std::result::Result<&LaidOut, String> {
        self.laid.get_or_init(|| {
            let (Some(frame), Some(atoms)) = (&self.frame, &self.atoms) else { return Err(format!("{}: no documents' frame to lay out", self.rel)) };
            let laid = lay_out(&self.rel, frame, self.digest, atoms)?;
            let granules: Vec<(i64, usize)> = laid.1.iter().map(|(g, owners)| (*g, owners.iter().map(|o| o.1 as usize).sum())).collect();
            if granules != self.granules {
                return Err(format!("{}: its granules are not those its documents' frame recorded", self.rel));
            }
            Ok(laid.0)
        }).as_ref().map_err(Clone::clone)
    }
}

/// An input's documents' frame decoded: its documents, checked against its digest; each owner's range; and each
/// granule its rows lie in with its owners (the granule, then the owner, ascending), as `write_input_kept` laid
/// them out.
fn lay_out(rel: &str, frame: &kept_read::Entry, digest: u64, atoms: &crate::content_store::atoms::Atoms) -> std::result::Result<(LaidOut, Vec<(i64, Vec<(i64, u32)>)>), String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "a kept input's documents, laid out where first asked".into());
    if frame.width() != 3 {
        return Err(format!("{rel}: its documents' entry is not three columns a row"));
    }
    // Its documents read where they lie (decision 0353 step 8); each owner's range and each granule's owners are
    // laid out from them.
    let Some(docs) = Docs::kept(frame, Some(digest)) else {
        return Err(format!("{rel}: its documents' entry does not match its digest"));
    };
    let rows = frame.packed();
    let mut keys: Vec<i64> = (0..rows.len()).map(|i| rows.get(i, 0)).collect();
    if frame.types().first() == Some(&b's') {
        atoms.content_keys(&mut keys);
    }
    let mut owners: Vec<(i64, i64, u64)> = keys.into_iter().enumerate().map(|(i, k)| (1 + k.rem_euclid(GRANULES), rows.get(i, 0), rows.get(i, 2) as u64)).collect();
    owners.sort_unstable();
    let mut ranges = Ranges::default();
    ranges.reserve(owners.len());
    let mut granules: Vec<(i64, Vec<(i64, u32)>)> = Vec::new();
    let mut at = 0u32;
    for run in owners.chunk_by(|a, b| a.0 == b.0) {
        for &(_, owner, n) in run {
            ranges.insert(owner, (at, n as u32));
            at += n as u32;
        }
        granules.push((run[0].0, run.iter().map(|o| (o.1, o.2 as u32)).collect()));
    }
    let owners = granules.iter().map(|(_, o)| o.clone()).collect();
    Ok((LaidOut { docs: Arc::new(docs), ranges: Arc::new(ranges), owners }, granules))
}

/// The documents' table of a cross part's base relations (decision 0458): each file their rows were read off when its
/// kept part was written, by path, document key and content. Kept beside the base relations' digest-only entries,
/// under the cross part's `binds`: the next run compares it with its own files, before its first wave, and makes the
/// old documents of those changed or gone.
const BASE_DOCUMENTS: &str = "#base-documents";

fn base_documents_ident(binds: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.base-documents/v1\0");
    h.update(binds);
    h.finalize().into()
}

/// A file of the documents' table: its path, document key and content (`Stored`, or `Unstored` by its digest when
/// the store did not hold it yet as the table was written).
pub(in crate::code_rule_dsl) type BaseDocument = (String, i64, super::memo::Content);

/// The memo plan a documents' table was written under (`put_base_documents`), as a read of its entry: a document of
/// the same content and key holds the rows it held then only under the same plan (its modules renamed alike).
const MEMO_PLAN_READ: &str = "#memo-plan";

/// The tag of a memo plan, as a documents' table records it.
pub(in crate::code_rule_dsl) fn memo_plan_tag(plan: &[u8; 32]) -> u64 {
    u64::from_le_bytes(plan[..8].try_into().expect("eight bytes"))
}

/// A documents' table as read (`base_documents`).
pub(in crate::code_rule_dsl) struct BaseTable {
    /// Its files, as written (an `Unstored` content is the caller's to resolve), each with its places where the writer
    /// found them (decision 0494 step 4 part 2).
    pub(in crate::code_rule_dsl) docs: Vec<PlacedDocument>,
    /// The memo plan it was written under (`memo_plan_tag`), none for a table written before it was recorded.
    pub(in crate::code_rule_dsl) plan: Option<u64>,
    /// The digest of the documents it names: the base relations' digests are of it (decision 0458).
    pub(in crate::code_rule_dsl) digest: u64,
}

/// The documents' table the store's frames in `space` keep for the cross part of `binds`, as a `BaseTable`; `None`
/// when there is none. Read before the first wave.
pub(in crate::code_rule_dsl) fn base_documents(store: &ContentStore, space: [u8; 32], binds: [u8; 32]) -> Result<Option<BaseTable>> {
    let Some(bound) = kept_read::Bound::of(store, &space).map_err(|e| Error::Plan(e.to_string()))? else { return Ok(None) };
    base_documents_in(store, &bound, binds)
}

/// The documents' table of `base_documents`, read from a space already bound (a kept part's, `InKept::bound_space`):
/// a run inside another's waves reads it there, where it reads no store.
pub(in crate::code_rule_dsl) fn base_documents_in(store: &ContentStore, bound: &Arc<kept_read::Bound>, binds: [u8; 32]) -> Result<Option<BaseTable>> {
    let Some(entry) = bound.entry(BASE_DOCUMENTS, &base_documents_ident(&binds), DOCS_PART) else { return Ok(None) };
    let plan = entry.reads().into_iter().find(|(r, _)| r == MEMO_PLAN_READ).map(|(_, tag)| tag);
    Ok(documents_table(store, &entry)?.map(|docs| BaseTable { docs, plan, digest: entry.digest() }))
}

/// A documents' table's rows (`put_base_documents`): each file's path (an atom), document key, and content (a tag,
/// then its source or digest in four words), and in a base relations' table its places (`DocumentPlaces`), read where
/// they lie. `None` when a row does not read, or names a content only in memory (tag 2), which no later run can take
/// as unchanged.
fn documents_table(store: &ContentStore, entry: &kept_read::Entry) -> Result<Option<Vec<PlacedDocument>>> {
    use crate::content_store::{DOCUMENT_WORDS, DocumentPlaces, PLACED_WORDS};
    let width = entry.width();
    if width != DOCUMENT_WORDS && width != PLACED_WORDS {
        return Ok(None);
    }
    let atoms = store.atoms_handle();
    // Each row read where it lies into one row's words (decision 0353 step 8).
    let packed = entry.packed();
    let mut row = [0i64; PLACED_WORDS];
    let mut out = Vec::with_capacity(entry.len());
    for i in 0..packed.len() {
        for (c, w) in row[..width].iter_mut().enumerate() {
            *w = packed.get(i, c);
        }
        let r = &row[..width];
        let Ok(atom) = u32::try_from(r[0]) else { return Ok(None) };
        let Some(path) = atoms.value(atom).map_err(|e| Error::Plan(e.to_string()))? else { return Ok(None) };
        let content = match r[2] {
            0 => super::memo::Content::Stored(crate::content_store::SourceId::from_raw(r[3])),
            1 => super::memo::Content::Unstored(words_digest(&r[3..7])),
            _ => return Ok(None),
        };
        out.push(((path, r[1], content), DocumentPlaces::of_row(r)));
    }
    Ok(Some(out))
}

/// A document of a documents' table, with its places where the writer found them (decision 0494 step 4 part 2).
pub(in crate::code_rule_dsl) type PlacedDocument = (BaseDocument, Option<crate::content_store::DocumentPlaces>);

/// Each base relation `rels`' statistics the store's frames in `space` keep for the cross part of `binds`, with the
/// digest of the rows they count (decision 0458: kept by file as the keeper's are, 0096 §3, but of the documents).
pub(in crate::code_rule_dsl) fn base_stats(store: &ContentStore, space: [u8; 32], binds: [u8; 32], rels: &[String]) -> Result<HashMap<String, (u64, crate::code_rule_dsl::logic::plan::statkeep::RelStats)>> {
    let Some(bound) = kept_read::Bound::of(store, &space).map_err(|e| Error::Plan(e.to_string()))? else { return Ok(HashMap::new()) };
    let ident = base_documents_ident(&binds);
    let mut out = HashMap::new();
    for rel in rels {
        let Some(entry) = bound.entry(&base_stats_name(rel), &ident, DOCS_PART) else { continue };
        // Its words read where they lie (decision 0353 step 8): the blob's length, then its bytes eight to a word.
        let words = entry.packed();
        if words.arity != 1 || words.is_empty() {
            continue;
        }
        let n = usize::try_from(words.get(0, 0)).unwrap_or(0).min(8 * (words.len() - 1));
        if let Some(stats) = crate::code_rule_dsl::logic::plan::statkeep::RelStats::decode_in(&WordBytes { words, n }, &mut 0) {
            out.insert(rel.clone(), (entry.digest(), stats));
        }
    }
    Ok(out)
}

/// A blob kept as words (`BlockFlat::of_bytes`: its length, then its bytes eight to a word) read as bytes where it
/// lies (`statkeep::ReadAt`): `n` bytes from word 1 of column 0.
struct WordBytes<'a> {
    words: &'a rerail_blocks::packed::Packed,
    n: usize,
}

impl crate::code_rule_dsl::logic::plan::statkeep::ReadAt for WordBytes<'_> {
    fn len(&self) -> usize {
        self.n
    }
    fn read_at(&self, at: usize, out: &mut [u8]) -> Option<()> {
        (at.checked_add(out.len())? <= self.n).then_some(())?;
        for (k, b) in out.iter_mut().enumerate() {
            let i = at + k;
            *b = (self.words.get(1 + i / 8, 0) >> (8 * (i % 8))) as u8;
        }
        Some(())
    }
}

/// A document's row of the counts kept beside the documents' table (decision 0494): its digest (`Document::digest`),
/// views' counts and facts' size as the corpus batch made or was given them, and the content they were counted of (a
/// file's own tree; none for a guest, or a content held in memory alone), and a host file's tree's or program's host and
/// file (decision 0512). Of the content a file has now, its documents are made when first read (`syntax::Pending`,
/// Q332): their digests and sizes are these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct BaseCount {
    pub(in crate::code_rule_dsl) digest: [u8; 32],
    pub(in crate::code_rule_dsl) counts: crate::code_rule_dsl::facts::vtab::syntax::made::ViewCounts,
    pub(in crate::code_rule_dsl) size: crate::code_rule_dsl::facts::vtab::syntax::FactsSize,
    pub(in crate::code_rule_dsl) content: Option<super::memo::Content>,
    pub(in crate::code_rule_dsl) guest: Option<KeptGuest>,
}

/// A host file's guest as the documents' table keeps it (decision 0512): its host and file, whether it is the file's
/// tree (`Made::File`) or a program of it (`Made::Host`), and its tape tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct KeptGuest {
    pub(in crate::code_rule_dsl) host: u32,
    pub(in crate::code_rule_dsl) file: u32,
    pub(in crate::code_rule_dsl) tree: bool,
    pub(in crate::code_rule_dsl) tag: u8,
}

impl KeptGuest {
    /// Its host and file as a pending guest is given them (`syntax::Pending::with_guest`).
    pub(in crate::code_rule_dsl) fn hosted(self) -> crate::code_rule_dsl::facts::vtab::syntax::Hosted {
        use crate::code_rule_dsl::facts::vtab::syntax::Made;
        crate::code_rule_dsl::facts::vtab::syntax::Hosted { host: self.host, file: self.file, made: if self.tree { Made::File } else { Made::Host } }
    }
}

/// The counts kept beside the documents' table, by document key.
pub(in crate::code_rule_dsl) type BaseCounts = HashMap<u32, BaseCount>;

const BASE_COUNTS: &str = "#base-counts";

/// A counts row's words: key, digest (4), views' counts (`VIEW_COUNTS`), facts' size (2), content (its kind, then 4 words: as the
/// documents' table's; 2 for none; 3 for a host file's guest, then its host, file, 1 for its tree and its tape tag).
const COUNTS_WIDTH: usize = KIND_AT + 5;
/// The views' counts in a row (`ViewCounts`' length), and where the facts' size and the content's kind are.
const VIEW_COUNTS: usize = std::mem::size_of::<crate::code_rule_dsl::facts::vtab::syntax::made::ViewCounts>() / 4;
const SIZE_AT: usize = 5 + VIEW_COUNTS;
const KIND_AT: usize = SIZE_AT + 2;

/// The counts the store's frames in `space` keep for the cross part of `binds`; empty when none are kept (or kept in
/// an earlier width, which a run counts again).
pub(in crate::code_rule_dsl) fn base_counts(store: &ContentStore, space: [u8; 32], binds: [u8; 32]) -> Result<BaseCounts> {
    let Some(bound) = kept_read::Bound::of(store, &space).map_err(|e| Error::Plan(e.to_string()))? else { return Ok(HashMap::new()) };
    let Some(entry) = bound.entry(BASE_COUNTS, &base_documents_ident(&binds), DOCS_PART) else { return Ok(HashMap::new()) };
    if entry.width() != COUNTS_WIDTH {
        return Ok(HashMap::new());
    }
    let mut words: Vector<i64> = rerail_blocks::vector::Vector::with_capacity(entry.len() * COUNTS_WIDTH);
    entry.each_rows(&mut |r| words.extend_from_slice(r));
    let mut out = HashMap::with_capacity(entry.len());
    let small = |w: i64| u32::try_from(w).ok();
    for r in words.as_chunks::<COUNTS_WIDTH>().0 {
        let (Some(key), Some(counts), [Some(nodes), Some(codes)]) = (small(r[0]), <[i64; VIEW_COUNTS]>::try_from(&r[5..SIZE_AT]).unwrap().map(small).into_iter().collect::<Option<Vec<u32>>>(), [r[SIZE_AT], r[SIZE_AT + 1]].map(small)) else { continue };
        let mut digest = [0u8; 32];
        for (i, w) in r[1..5].iter().enumerate() {
            digest[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        let k = KIND_AT;
        let content = match r[k] {
            0 => Some(super::memo::Content::Stored(crate::content_store::SourceId::from_raw(r[k + 1]))),
            1 => {
                let mut d = [0u8; 32];
                for (i, w) in r[k + 1..k + 5].iter().enumerate() {
                    d[i * 8..i * 8 + 8].copy_from_slice(&w.to_le_bytes());
                }
                Some(super::memo::Content::Unstored(d))
            }
            _ => None,
        };
        let guest = (r[k] == 3).then(|| (small(r[k + 1]), small(r[k + 2]), u8::try_from(r[k + 4]).ok())).and_then(|g| match g {
            (Some(host), Some(file), Some(tag)) => Some(KeptGuest { host, file, tree: r[k + 3] == 1, tag }),
            _ => None,
        });
        out.insert(key, BaseCount { digest, counts: counts.try_into().unwrap(), size: [nodes, codes], content, guest });
    }
    Ok(out)
}

/// Queues the counts `counts` of the corpus batch's documents (`Batch::kept_counts`), of digest `digest`, beside the
/// documents' table: each file's with its content now, `content_of` its path's.
pub(in crate::code_rule_dsl) fn put_base_counts(
    store: &ContentStore,
    was: &InKept,
    binds: [u8; 32],
    counts: &[crate::code_rule_dsl::facts::vtab::syntax::made::Kept],
    content_of: &dyn Fn(&str) -> Option<super::memo::Content>,
    digest: u64,
) {
    let mut rows = BlockFlat::new(counts.len(), COUNTS_WIDTH, true);
    let mut w = rows.writer();
    for k in counts {
        w.push(i64::from(k.key));
        w.extend(k.digest.as_chunks::<8>().0.iter().map(|w| i64::from_le_bytes(*w)));
        w.extend(k.counts.iter().map(|&c| i64::from(c)));
        w.extend(k.size.iter().map(|&c| i64::from(c)));
        match k.file.then(|| content_of(&k.path)).flatten() {
            Some(super::memo::Content::Stored(id)) => w.extend([0, id.raw(), 0, 0, 0]),
            // A content the store does not hold yet, by its digest (a later run finds it stored by it).
            Some(super::memo::Content::Unstored(d)) => {
                w.push(1);
                w.extend(d.as_chunks::<8>().0.iter().map(|w| i64::from_le_bytes(*w)));
            }
            None => match &k.guest {
                Some((g, tag)) => w.extend([3, i64::from(g.host), i64::from(g.file), i64::from(g.made == crate::code_rule_dsl::facts::vtab::syntax::Made::File), i64::from(*tag)]),
                None => w.extend([2, 0, 0, 0, 0]),
            },
        }
    }
    let name = KeptRel { name: BASE_COUNTS.to_owned(), ident: base_documents_ident(&binds) };
    store.queue_kept(was.space, vec![kept_flat(&name, DOCS_PART, digest, Vec::new(), vec![b'i'; COUNTS_WIDTH], rows)]);
}

fn base_stats_name(rel: &str) -> String {
    format!("#base-stats/{rel}")
}

/// Queues base relation `rel`'s statistics `stats`, of the rows of digest `digest`, beside the documents' table.
pub(in crate::code_rule_dsl) fn put_base_stats(store: &ContentStore, was: &InKept, binds: [u8; 32], rel: &str, digest: u64, stats: &crate::code_rule_dsl::logic::plan::statkeep::RelStats) {
    let mut bytes = Vec::new();
    stats.encode(&mut bytes);
    let name = KeptRel { name: base_stats_name(rel), ident: base_documents_ident(&binds) };
    store.queue_kept(was.space, vec![kept_flat(&name, DOCS_PART, digest, Vec::new(), vec![b'i'], BlockFlat::of_bytes(&bytes))]);
}

fn semijoin_rows_name(rel: &str) -> String {
    format!("#semijoin-at-held/{rel}")
}

/// The digest of the exports `exports` as `digests` gives them (each its kept input's, decision 0117 step 1), in
/// their order: the held nodes are their values, so exports of the same digest hold the same nodes. None when one
/// has no digest.
pub(in crate::code_rule_dsl) fn exports_digest(exports: &[String], digests: &HashMap<String, u64>) -> Option<u64> {
    let mut words = Vec::with_capacity(2 * exports.len());
    for rel in exports {
        words.push(crate::code_rule_dsl::logic::plan::row_digest(&rel.bytes().map(i64::from).collect::<Vec<_>>()) as i64);
        words.push(*digests.get(rel)? as i64);
    }
    Some(crate::code_rule_dsl::logic::plan::row_digest(&words))
}

/// A semi-join relation's rows at the nodes the exports held (Q396), kept beside the documents' table as the base
/// relations' statistics are (`base_stats`), with the digest of the documents they were read of: the nodes walked
/// (`held`, sorted), and each one's row (`rows`, `width` values a row, in `input_columns` order). A next run takes the
/// rows of its held nodes from them and walks only the nodes they do not answer for, where reading every held node's
/// document again made all of them (ep2's edit: 300 documents, 1.1 s). With the digest of the exports the nodes were
/// taken of (`exports_digest`): a next run whose exports have it takes its held nodes from `held`, where visiting
/// every export's rows for them took 1.6 s of ep2's edit. With the statistics counted of them (`stats`, each column's
/// rows, distinct values and the two sums of squares, in the order of the relation's columns): a run of the same
/// documents and exports takes them as they are, where counting every row again took 0.1 s of ep2's edit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct SemijoinRows {
    pub(in crate::code_rule_dsl) exports: Option<u64>,
    pub(in crate::code_rule_dsl) stats: Vec<[i64; 4]>,
    pub(in crate::code_rule_dsl) width: usize,
    pub(in crate::code_rule_dsl) held: Vec<i64>,
    pub(in crate::code_rule_dsl) rows: Vec<i64>,
}

impl SemijoinRows {
    /// As words: whether the exports' digest is known and the digest, the statistics' number and theirs, the width,
    /// the held nodes' number and the nodes, then the rows.
    fn encode(&self) -> Vec<i64> {
        let mut words = Vec::with_capacity(5 + 4 * self.stats.len() + self.held.len() + self.rows.len());
        words.extend([i64::from(self.exports.is_some()), self.exports.unwrap_or(0) as i64, self.stats.len() as i64]);
        words.extend(self.stats.iter().flatten());
        words.extend([self.width as i64, self.held.len() as i64]);
        words.extend_from_slice(&self.held);
        words.extend_from_slice(&self.rows);
        words
    }

    #[cfg(test)]
    fn decode(words: &[i64]) -> Option<Self> {
        let l = SemijoinLayout::of(&|i| words[i], words.len())?;
        Some(SemijoinRows { exports: l.exports, stats: l.stats, width: l.width, held: words[l.held].to_vec(), rows: words[l.rows].to_vec() })
    }
}

/// Where `SemijoinRows::encode`'s parts lie among `len` words, `get` reading one: its header read, the held nodes' and
/// the rows' places.
struct SemijoinLayout {
    exports: Option<u64>,
    stats: Vec<[i64; 4]>,
    width: usize,
    held: std::ops::Range<usize>,
    rows: std::ops::Range<usize>,
}

impl SemijoinLayout {
    fn of(get: &dyn Fn(usize) -> i64, len: usize) -> Option<SemijoinLayout> {
        if len < 3 {
            return None;
        }
        let exports = match get(0) {
            0 => None,
            1 => Some(get(1) as u64),
            _ => return None,
        };
        let at = usize::try_from(get(2)).ok()?.checked_mul(4)?.checked_add(3)?;
        if at + 2 > len {
            return None;
        }
        let stats = (3..at).step_by(4).map(|i| [get(i), get(i + 1), get(i + 2), get(i + 3)]).collect();
        let width = usize::try_from(get(at)).ok().filter(|&w| w > 0)?;
        let n = usize::try_from(get(at + 1)).ok()?;
        let end = (at + 2).checked_add(n).filter(|&e| e <= len)?;
        (len - end).is_multiple_of(width).then_some(SemijoinLayout { exports, stats, width, held: at + 2..end, rows: end..len })
    }
}

/// A semi-join relation's rows at the held nodes as kept (`semijoin_rows`): its header read, its held nodes and rows
/// read where they lie when asked, where copying out both relations' entries (1.4 M words each, the held nodes in each)
/// took 0.05 s of ep2's edit, which read only their statistics and one entry's held nodes.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct KeptSemijoin {
    pub(in crate::code_rule_dsl) exports: Option<u64>,
    pub(in crate::code_rule_dsl) stats: Vec<[i64; 4]>,
    pub(in crate::code_rule_dsl) width: usize,
    held: std::ops::Range<usize>,
    rows: std::ops::Range<usize>,
    words: SemijoinWords,
}

/// Where a kept semi-join's words lie: the store's entry beside the documents' table, or a session's (decision 0458:
/// a session holds its kept statistics and rows itself), its words as `SemijoinRows::encode` wrote them once.
#[derive(Clone)]
enum SemijoinWords {
    Entry(kept_read::Entry),
    Held(Arc<[i64]>),
}

impl SemijoinWords {
    /// Each word, read where it lies.
    fn read(&self, each: &mut dyn FnMut(&dyn Fn(usize) -> i64)) {
        match self {
            SemijoinWords::Entry(e) => {
                let d = e.packed().col(0).dec();
                each(&|i| d.get(i))
            }
            SemijoinWords::Held(w) => each(&|i| w[i]),
        }
    }
}

impl KeptSemijoin {
    /// `rows`, held as a session holds them (`HeldCorpus::semijoin`): their words, encoded once.
    pub(in crate::code_rule_dsl) fn of_rows(rows: &SemijoinRows) -> Option<KeptSemijoin> {
        let words: Arc<[i64]> = rows.encode().into();
        let l = SemijoinLayout::of(&|i| words[i], words.len())?;
        Some(KeptSemijoin { exports: l.exports, stats: l.stats, width: l.width, held: l.held, rows: l.rows, words: SemijoinWords::Held(words) })
    }
    /// The held nodes' number.
    pub(in crate::code_rule_dsl) fn held_len(&self) -> usize {
        self.held.len()
    }
    /// The held nodes, read where they lie.
    pub(in crate::code_rule_dsl) fn held(&self) -> Box<dyn Iterator<Item = i64> + '_> {
        match &self.words {
            SemijoinWords::Entry(e) => {
                let d = e.packed().col(0).dec();
                Box::new(self.held.clone().map(move |i| d.get(i)))
            }
            SemijoinWords::Held(w) => Box::new(w[self.held.clone()].iter().copied()),
        }
    }
    /// Each row, read where it lies.
    pub(in crate::code_rule_dsl) fn each_row(&self, each: &mut dyn FnMut(&[i64])) {
        let mut row = vec![0; self.width];
        self.words.read(&mut |get| {
            for at in self.rows.clone().step_by(self.width) {
                for (c, w) in row.iter_mut().enumerate() {
                    *w = get(at + c);
                }
                each(&row);
            }
        });
    }
}

/// Each semi-join relation `rels`' rows at the held nodes as kept beside the documents' table of `binds`
/// (`put_semijoin_rows`), with the digest of the documents they were read of.
pub(in crate::code_rule_dsl) fn semijoin_rows(store: &ContentStore, space: [u8; 32], binds: [u8; 32], rels: &[&str]) -> Result<HashMap<String, (u64, KeptSemijoin)>> {
    let Some(bound) = kept_read::Bound::of(store, &space).map_err(|e| Error::Plan(e.to_string()))? else { return Ok(HashMap::new()) };
    let ident = base_documents_ident(&binds);
    let mut out = HashMap::new();
    for rel in rels {
        let Some(entry) = bound.entry(&semijoin_rows_name(rel), &ident, DOCS_PART) else { continue };
        if entry.width() != 1 {
            continue;
        }
        let d = entry.packed().col(0).dec();
        let Some(l) = SemijoinLayout::of(&|i| d.get(i), entry.len()) else { continue };
        let kept = KeptSemijoin { exports: l.exports, stats: l.stats, width: l.width, held: l.held, rows: l.rows, words: SemijoinWords::Entry(entry.clone()) };
        out.insert((*rel).to_owned(), (entry.digest(), kept));
    }
    Ok(out)
}

/// Queues semi-join relation `rel`'s rows at the held nodes, read of documents of digest `digest`, beside the
/// documents' table (`semijoin_rows`).
pub(in crate::code_rule_dsl) fn put_semijoin_rows(store: &ContentStore, was: &InKept, binds: [u8; 32], rel: &str, digest: u64, rows: &SemijoinRows) {
    let words = rows.encode();
    let name = KeptRel { name: semijoin_rows_name(rel), ident: base_documents_ident(&binds) };
    store.queue_kept(was.space, vec![kept_flat(&name, DOCS_PART, digest, Vec::new(), vec![b'i'], crate::content_store::kept_facts::BlockFlat::of_rows(&words, 1, true))]);
}

/// Queues the documents' table `docs` for the cross part of `binds` into the space `was` was read from, in the keep's
/// wave, under the memo plan `plan` (`memo_plan_tag`), with `digest`, the digest of the documents it names (decision
/// 0458: the base relations' digests are of it): the writer finds each document's places as it writes it, its tape in
/// the grammar `tape` (grammar id, grammar version, tape format version) names (decision 0494 step 4 part 2). `false`
/// (nothing queued) when a path is no atom.
pub(in crate::code_rule_dsl) fn put_base_documents(store: &ContentStore, was: &InKept, binds: [u8; 32], plan: u64, docs: &[BaseDocument], digest: u64, tape: Option<[i64; 3]>) -> bool {
    let paths: Vec<&str> = docs.iter().map(|(p, _, _)| p.as_str()).collect();
    let Ok(atoms) = store.atoms_handle().intern_all(&paths) else { return false };
    let mut rows = BlockFlat::new(docs.len(), crate::content_store::DOCUMENT_WORDS, true);
    let mut w = rows.writer();
    for ((_, key, content), atom) in docs.iter().zip(atoms) {
        w.extend([i64::from(atom), *key]);
        match content {
            super::memo::Content::Stored(id) => w.extend([0, id.raw(), 0, 0, 0]),
            super::memo::Content::Unstored(d) => {
                w.push(1);
                w.extend(digest_words(d));
            }
        }
    }
    let name = KeptRel { name: BASE_DOCUMENTS.to_owned(), ident: base_documents_ident(&binds) };
    let mut types = vec![b'i'; crate::content_store::PLACED_WORDS];
    types[0] = b's';
    let what = crate::content_store::kept_facts::KeptWhat::Documents { rows, tape };
    let reads = vec![(MEMO_PLAN_READ.to_owned(), plan)];
    store.queue_kept(was.space, vec![crate::content_store::kept_facts::KeptPut { rel: name.name, ident: name.ident, part: DOCS_PART, digest, reads, types, what }]);
    true
}

/// Whether the store's frames in `space` keep a frame of documents for every input `rels` of the cross part `prog`
/// (key `key`): `Err` (why) when one has none. Asked before the first wave, so that a run whose cross part kept no
/// inputs (its first, or one after a rule edit) decodes its memo hits whole as it finds them: the waves read no entry
/// of the store (decision 0104), and a hit read for its outputs alone has nothing else to take its rows from.
pub(in crate::code_rule_dsl) fn kept_inputs_stored(store: &ContentStore, space: [u8; 32], binds: [u8; 32], prog: &Placed, rels: &[String]) -> Result<std::result::Result<(), String>> {
    // The space's last commit, past what an earlier run of the process bound (`Bound::of`).
    let bound = kept_read::Bound::of(store, &space).map_err(|e| Error::Plan(e.to_string()))?;
    let has = |rel: &String| bound.as_ref().is_some_and(|b| b.entry(rel, &input_ident(rel, prog, &binds), DOCS_PART).is_some());
    Ok(match rels.iter().find(|r| !has(r)) {
        Some(rel) => Err(format!("{rel}: no entry of its documents ({} of {} inputs have one)", rels.iter().filter(|r| has(r)).count(), rels.len())),
        None => Ok(()),
    })
}

/// The inputs `rels` of the cross part `prog` (key `key`) as the store's frames in `space` keep them: read as
/// `Retained::load` reads a changed input, each owner's rows a range. `Err` (why) when one is not kept, or its frames
/// do not match their document table.
pub(in crate::code_rule_dsl) fn kept_inputs(store: &ContentStore, space: [u8; 32], binds: [u8; 32], prog: &Placed, rels: &[String], stats_of: &BTreeSet<String>) -> Result<std::result::Result<HashMap<String, KeptInput>, String>> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the kept inputs, bound where they lie".into());
    let Some(bound) = kept_read::Bound::of(store, &space).map_err(|e| Error::Plan(e.to_string()))? else {
        return Ok(Err("the space keeps no blocks".into()));
    };
    let atoms = store.atoms_handle();
    let fields = prog.fields()?;
    let traced = std::env::var_os("RERAIL_KEPT_READS").is_some();
    let mut out = HashMap::new();
    for rel in rels {
        let ident = input_ident(rel, prog, &binds);
        // Its entries, its name searched once (a granule's entry each, and its documents' and statistics').
        let keyed = bound.keyed(rel, &ident);
        let Some(docs_entry) = keyed.entry(DOCS_PART) else {
            return Ok(Err(format!("{rel}: no entry of its documents")));
        };
        let Some(width) = fields.arity(rel) else { return Ok(Err(format!("{rel}: not declared by the cross part"))) };
        let digest = docs_entry.digest();
        // Its granules: those its documents' entry records (`GRANULES_READ`), none read; or, for an entry written
        // before the keeper recorded them, its documents laid out now.
        let recorded = docs_entry.reads().into_iter().find(|(r, _)| r == GRANULES_READ).map(|(_, mask)| mask).filter(|_| width > 0);
        let (ids, laid) = match recorded {
            Some(mask) => (granules_of(mask), None),
            None => match lay_out(rel, &docs_entry, digest, &atoms) {
                Ok((laid, granules)) => (granules.into_iter().map(|(g, _)| g).collect::<Vector<i64>>(), Some(laid)),
                Err(why) => return Ok(Err(why)),
            },
        };
        // Each granule's entry and its rows' count; the rows bound where they lie, one segment a granule, and the
        // documents read where an owner is first asked (`InputLayout`): an edit reads few.
        let mut granules: Vec<kept_read::Entry> = Vec::with_capacity(ids.len());
        let mut counts: Vec<(i64, usize)> = Vec::with_capacity(ids.len());
        for (k, id) in ids.iter().enumerate() {
            let Some(part) = keyed.entry(*id) else { return Ok(Err(format!("{rel}: a granule's entry is missing"))) };
            let n = part.len();
            if part.width() != width {
                return Ok(Err(format!("{rel}: a granule's entry does not hold its owners' rows")));
            }
            if let Some(l) = &laid
                && l.owners[k].iter().map(|o| o.1 as usize).sum::<usize>() != n
            {
                return Ok(Err(format!("{rel}: a granule's entry does not hold its owners' rows")));
            }
            granules.push(part);
            counts.push((*id, n));
        }
        let starts: Vector<usize> = counts.iter().scan(0usize, |at, (_, n)| { let s = *at; *at += n; Some(s) }).collect();
        let layout = Arc::new(InputLayout { rel: rel.clone(), frame: Some(docs_entry), digest, atoms: Some(Arc::clone(&atoms)), granules: counts, laid: laid.map(|l| std::sync::OnceLock::from(Ok(l))).unwrap_or_default() });
        let names: Vec<String> = fields.columns(rel).unwrap_or_default().into_iter().map(|(n, _)| n.to_owned()).collect();
        let Some(held) = kept_read::gathered(&granules, width, &atoms) else { return Ok(Err(format!("{rel}: its rows' atoms are not this process's"))) };
        let source: Arc<dyn Source> = held.source(names);
        // Each granule's first row, in the order the rows were laid out.
        let source: Arc<dyn Source> = if traced { Arc::new(Traced::new(rel.clone(), source, starts)) } else { source };
        // Its statistics at these rows: the entry under the documents' digest (an older entry is never matched).
        let stats = keyed
            .entry(kept_stats::PART)
            .filter(|p| kept_stats::on() && stats_of.contains(rel) && p.digest() == digest)
            .and_then(|p| {
                // Its words read where they lie (decision 0353 step 8): one column of them.
                let rows = p.packed();
                (rows.arity == 1).then(|| kept_stats::from_words(rows.len(), &|i| rows.get(i, 0))).flatten()
            })
            .and_then(|all| all.get(&(rel.clone(), digest)).map(|(columns, _, _)| columns.clone()));
        out.insert(rel.clone(), KeptInput { source, layout, digest, stats });
    }
    Ok(Ok(out))
}


/// `RERAIL_KEPT_READS`: a kept input that counts what the run reads of it, printed when it is dropped: the rows and
/// granules read one at a time, and the calls that read a whole column (a scan, an index, statistics).
struct Traced {
    rel: String,
    inner: Arc<dyn Source>,
    starts: Vector<usize>,
    rows: parking_lot::Mutex<rustc_hash::FxHashSet<usize>>,
    whole: parking_lot::Mutex<BTreeMap<&'static str, usize>>,
}
impl Traced {
    fn new(rel: String, inner: Arc<dyn Source>, starts: Vector<usize>) -> Self {
        Self { rel, inner, starts, rows: Default::default(), whole: Default::default() }
    }
    fn row(&self, r: usize) {
        let mut rows = self.rows.lock();
        if rows.is_empty() && std::env::var_os("RERAIL_KEPT_READS_STACK").is_some_and(|v| v == self.rel.as_str()) {
            eprintln!("RERAIL_KEPT_READS {}: first row read at\n{}", self.rel, std::backtrace::Backtrace::force_capture());
        }
        rows.insert(r);
    }
    fn rows_of(&self, got: &Option<rerail_blocks::vector::Vector<usize>>) {
        if let Some(g) = got {
            self.rows.lock().extend(g.iter().copied());
        }
    }
    fn whole(&self, what: &'static str) {
        *self.whole.lock().entry(what).or_default() += 1;
    }
}
impl Drop for Traced {
    fn drop(&mut self) {
        let rows = self.rows.lock();
        let granules: BTreeSet<usize> = rows.iter().map(|r| self.starts.partition_point(|s| s <= r)).collect();
        eprintln!("RERAIL_KEPT_READS {}: {} rows in {} granules; read {} rows in {} granules; whole {:?}", self.rel, self.inner.len(), self.starts.len(), rows.len(), granules.len(), self.whole.lock());
    }
}
impl Source for Traced {
    fn columns(&self) -> Vec<String> {
        self.inner.columns()
    }
    fn len(&self) -> usize {
        self.inner.len()
    }
    fn row_stream(&self, range: std::ops::Range<usize>, reverse: bool) -> Option<Box<dyn crate::code_rule_dsl::facts::vtab::RowStream>> {
        self.whole("row_stream");
        self.inner.row_stream(range, reverse)
    }
    fn direct_all(&self, equal: &[(usize, i64)]) -> Option<Vec<crate::code_rule_dsl::facts::vtab::ReadRow>> {
        self.whole("direct_all");
        self.inner.direct_all(equal)
    }
    fn direct_one(&self, column: usize, values: &[i64]) -> Option<Vec<crate::code_rule_dsl::facts::vtab::ReadRow>> {
        self.whole("direct_one");
        self.inner.direct_one(column, values)
    }
    fn caches_rows(&self) -> bool {
        self.inner.caches_rows()
    }
    fn row_cells(&self, row: usize) -> Option<(i64, Arc<dyn crate::code_rule_dsl::facts::vtab::RowCells>)> {
        self.row(row);
        self.inner.row_cells(row)
    }
    fn frozen(&self) -> Option<Arc<dyn Source>> {
        self.whole("frozen");
        self.inner.frozen()
    }
    fn revision(&self) -> Option<u64> {
        self.inner.revision()
    }
    fn cell(&self, row: usize, column: usize) -> rusqlite::types::Value {
        self.row(row);
        self.inner.cell(row, column)
    }
    fn row_id(&self, row: usize) -> i64 {
        self.inner.row_id(row)
    }
    fn text_column(&self, column: usize) -> bool {
        self.inner.text_column(column)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        self.row(row);
        self.inner.row_at(row, columns, out)
    }
    fn estimated_len(&self) -> usize {
        self.inner.estimated_len()
    }
    fn known_len(&self) -> Option<usize> {
        self.inner.known_len()
    }
    fn may_hold_rows(&self) -> bool {
        self.inner.may_hold_rows()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.inner.estimated_matches(column, batch)
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        self.inner.estimated_matches_all(columns)
    }
    fn composite(&self) -> bool {
        self.inner.composite()
    }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.whole("lookup_all (index)");
        let got = self.inner.lookup_all(equal);
        self.rows_of(&got);
        got
    }
    fn rows_of_documents(&self, docs: &BTreeSet<u32>) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.inner.rows_of_documents(docs)
    }
    fn doc_column(&self) -> Option<usize> {
        self.inner.doc_column()
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.whole("lookup_in (index)");
        let got = self.inner.lookup_in(column, values, doc);
        self.rows_of(&got);
        got
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.inner.kept_to_doc(column)
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vector<u32> {
        self.whole("filter_eq (scan)");
        self.inner.filter_eq(eqs)
    }
    fn made_on_lookup(&self) -> bool {
        self.inner.made_on_lookup()
    }
    fn complete_rows(&self) -> bool {
        self.inner.complete_rows()
    }
    fn rows_owned(&self, g: usize, owned: &dyn Fn(i64) -> bool) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.inner.rows_owned(g, owned)
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        self.inner.expect_lookups(column, keys)
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.whole("statistics");
        self.inner.statistics()
    }
    fn estimated_statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.whole("statistics");
        self.inner.estimated_statistics()
    }
    fn value_counts(&self, wanted: &[(usize, i64)]) -> Option<Vector<i64>> {
        self.whole("value_counts");
        self.inner.value_counts(wanted)
    }
    fn failure(&self) -> Option<String> {
        self.inner.failure()
    }
    fn failed(&self) -> Option<String> {
        self.inner.failed()
    }
    fn indexed(&self, column: usize) -> bool {
        self.inner.indexed(column)
    }
    fn range_upper_compatible(&self, lower: usize, upper: usize) -> bool {
        self.inner.range_upper_compatible(lower, upper)
    }
    fn range_indexed(&self, column: usize) -> bool {
        self.inner.range_indexed(column)
    }
    fn column_counts(&self, c: usize) -> Option<Vec<(i64, u64)>> {
        self.inner.column_counts(c)
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.whole("range_lookup (index)");
        let got = self.inner.range_lookup(column, lower, upper);
        self.rows_of(&got);
        got
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.whole("lookup (index)");
        let got = self.inner.lookup(column, values);
        self.rows_of(&got);
        got
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        self.row(row);
        self.inner.value(row, column)
    }
    fn sorted_rows(&self, column: usize, value: i64) -> Option<&[u32]> {
        self.whole("sorted_rows (index)");
        self.inner.sorted_rows(column, value)
    }
    fn sorted_range(&self, column: usize, value: i64) -> Option<std::ops::Range<u32>> {
        self.whole("sorted_rows (index)");
        self.inner.sorted_range(column, value)
    }
    fn sorts_rows(&self, column: usize) -> bool {
        self.inner.sorts_rows(column)
    }
    fn prepare_lookup(&self, columns: &[usize]) {
        self.whole("prepare_lookup");
        self.inner.prepare_lookup(columns)
    }
    fn prepare_index(&self, columns: &[usize]) {
        self.whole("prepare_index");
        self.inner.prepare_index(columns)
    }
    fn primary_key_len(&self) -> Option<usize> {
        self.inner.primary_key_len()
    }
    fn lookup_primary_batch(&self, values: &[Vector<i64>]) -> Option<rerail_blocks::vector::Vector<usize>> {
        self.whole("lookup_primary_batch");
        self.inner.lookup_primary_batch(values)
    }
    fn ordered(&self, equal: &[usize], order: &[(usize, bool)]) -> bool {
        self.inner.ordered(equal, order)
    }
    fn ordered_range(&self, equal: &[(usize, i64)]) -> Option<std::ops::Range<usize>> {
        self.whole("ordered_range");
        self.inner.ordered_range(equal)
    }
}

/// `source` with its columns named `names` (as the run binds it), its rows as they are.
pub(in crate::code_rule_dsl) fn renamed(source: Arc<dyn Source>, names: Vec<String>) -> Arc<dyn Source> {
    Arc::new(Renamed(source, names))
}

/// The frame part that holds owner `doc`'s rows (decision 0212): by the owner's content key where owners are atoms
/// (`atoms`, `Owners`), so every process parts an input's rows alike; a document (a node's owner) by its number, the
/// store's own.
fn granule(doc: i64, atoms: Option<&crate::content_store::atoms::Atoms>) -> i64 {
    let key = match atoms {
        Some(atoms) => {
            let mut k = [doc];
            atoms.content_keys(&mut k);
            k[0]
        }
        None => doc,
    };
    1 + key.rem_euclid(GRANULES)
}

/// Whether relation `rel`'s owners are atoms (its group column `g` is one, `types`): its documents' table types its
/// owner column `s`, which the store maps to its own ids and back (decision 0170), and its granules are by content.
fn atom_owners(types: &[u8], g: usize) -> bool {
    types.get(g) == Some(&b's')
}

/// A documents' table's column types: its owners atoms or numbers (`atom_owners`), then each owner's sum and count.
fn docs_types(atom_owner: bool) -> Vec<u8> {
    vec![if atom_owner { b's' } else { b'i' }, b'i', b'i']
}

impl Retained {
    /// The part of program `prog` (key `key`) as the store's frames in `space` hold it (t33 K4): what a session
    /// keeps in memory, kept across processes. A derived relation is taken when its frame is current: each relation
    /// it read has the digest it had then, and a derived read is taken too. So the relations taken are the rows one
    /// run left, whichever programs ran in between. Their rows are read when the run needs them (`load`).
    /// `sccs`: `prog`'s strata, computed once by the caller.
    /// `binds`: the key of the part's program alone (`keys`), which names its inputs' frames.
    pub(in crate::code_rule_dsl) fn bind_kept(store: &ContentStore, space: [u8; 32], key: [u8; 32], binds: [u8; 32], prog: &Placed, sccs: &crate::code_rule_dsl::logic::front::Components, bound: &HashMap<String, u64>) -> Result<Retained> {
        Self::bind_kept_named(store, space, key, binds, prog, sccs, bound, None)
    }

    /// `bind_kept`, the program's strata identities handed in where the run read them before the first wave
    /// (`frame_rels_kept`).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn bind_kept_named(store: &ContentStore, space: [u8; 32], key: [u8; 32], binds: [u8; 32], prog: &Placed, sccs: &crate::code_rule_dsl::logic::front::Components, bound: &HashMap<String, u64>, strata: Option<&StrataIdents>) -> Result<Retained> {
        // The program's version, for its identities and its leaves' (`from_frame_snapshot`).
        let leaves = crate::code_rule_dsl::logic::plan::Leaves::of_placed(prog.clone());
        let idents: Vec<(String, [u8; 32])> = match strata {
            Some(held) => held.iter().map(|(name, ident)| (name.clone(), *ident)).collect(),
            None => {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "the kept part's identities".into());
                leaves.identities_by_heads(sccs)?.into_iter().collect()
            }
        };
        let derived: BTreeSet<&str> = sccs.iter().flatten().collect();
        let mut names: HashMap<String, KeptRel> = idents.into_iter().map(|(name, ident)| (name.clone(), KeptRel { name, ident })).collect();
        // The inputs are what the run binds (`bound`, by their digests): the program's, and an earlier phase's export
        // as phase 1's `doc` or the types phase's `types__wgen`, which no rule may name (a pattern reads `node`).
        let read: BTreeSet<String> = prog.fields()?.inputs().map(str::to_owned).chain(bound.keys().cloned()).filter(|r| !derived.contains(r.as_str())).collect();
        for rel in &read {
            names.insert(rel.clone(), KeptRel { name: rel.clone(), ident: input_ident(rel, prog, &binds) });
        }
        // The space's kept blocks, read once before the first wave (decision 0401 F3): each entry's digest and reads.
        let kept = kept_read::Bound::of(store, &space).map_err(|e| Error::Plan(e.to_string()))?;
        let stored: HashMap<KeptRel, KeptStored> = match &kept {
            Some(b) => names.values().filter_map(|n| b.entry(&n.name, &n.ident, DOCS_PART).map(|e| (n.clone(), KeptStored { digest: e.digest(), reads: e.reads() }))).collect(),
            None => HashMap::new(),
        };
        Self::from_frame_snapshot(space, kept, key, prog, &leaves, sccs, names, stored, bound)
    }

    fn empty_kept(key: [u8; 32], mut frames: InKept) -> Self {
        frames.inputs.clear();
        frames.derived.clear();
        Self { key, docs: (Default::default(), [0xff; 32]), frames: Some(frames), ..Default::default() }
    }

    #[allow(clippy::too_many_arguments)]
    fn from_frame_snapshot(
        space: [u8; 32], kept: Option<Arc<kept_read::Bound>>, key: [u8; 32], prog: &Placed, leaves: &crate::code_rule_dsl::logic::plan::Leaves,
        sccs: &crate::code_rule_dsl::logic::front::Components, names: HashMap<String, KeptRel>, stored: HashMap<KeptRel, KeptStored>, bound: &HashMap<String, u64>,
    ) -> Result<Self> {
        let derived: BTreeSet<&str> = sccs.iter().flatten().collect();
        let mut frames = InKept { space, names, bound: kept.clone(), ..Default::default() };
        // The inputs are what the run binds (`bound`): the program's, and an earlier phase's export (`bind_kept`).
        let f = prog.fields()?;
        let read: BTreeSet<&str> = f.inputs().filter_map(|r| frames.names.get_key_value(r).map(|(k, _)| k)).chain(bound.keys().filter(|r| frames.names.contains_key(*r))).filter(|r| !derived.contains(r.as_str())).map(|r| r.as_str()).collect();
        let asked: Vec<_> = read.iter().filter(|r| stored.contains_key(&frames.names[**r])).map(|r| frames.names[*r].clone()).collect();
        let mut inputs = HashMap::new();
        for frame in &asked {
            let at = &stored[frame];
            let Some(docs_frame) = kept.as_ref().and_then(|b| b.entry(&frame.name, &frame.ident, DOCS_PART)) else { return Ok(Self::empty_kept(key, frames)) };
            // Its shape from its entry; its documents read where first asked (`FramedDocs`, Q91).
            let (types, n) = (docs_frame.types(), docs_frame.len());
            if types.len() != 3 { return Ok(Self::empty_kept(key, frames)); }
            // A relation of no columns is its count (`write_kept`): one entry, its digest the input's.
            let counted = f.arity(&frame.name) == Some(0) && n == 1;
            if types.first() == Some(&b's') {
                frames.atom_owned.insert(frame.name.clone());
            }
            if docs_frame.reads().iter().any(|(r, _)| r == DIGEST_ONLY_READ) {
                frames.digest_only.insert(frame.name.clone());
            }
            inputs.insert(frame.name.clone(), at.digest);
            frames.inputs.insert(frame.name.clone(), (at.digest, Arc::new(FramedDocs { frame: Some(docs_frame), digest: at.digest, counted, docs: std::sync::OnceLock::new() })));
        }
        // In stratum order: a relation is current when each of its reads is as it was when it was derived, in the
        // state the frames hold: an input as its frame has it (its old rows, which a changed stratum is maintained
        // from), or, with no frame, as the run binds it. A relation with no frame current is named by its reads as
        // `write_kept` names it, so a view of unchanged reads leaves its readers current.
        let mut known: HashMap<String, u64> = bound.clone();
        known.extend(frames.inputs.iter().map(|(rel, (d, _))| (rel.clone(), *d)));
        let all_reads = Reads::of(prog)?;
        for scc in sccs {
            // Each relation's entry as current: its reads as they are now. A run that found its rows unchanged under
            // other reads retagged it with them (`KeptWhat::Reads`), so its reads are the last run's.
            let current: Option<Vec<KeptStored>> = scc.iter().map(|rel| {
                let n = frames.names.get(rel)?;
                let at = stored.get(n)?;
                let now: Vec<(String, u64)> = at.reads.iter().map(|(read, _)| read_digest(read, &known, &derived, prog, leaves).map(|d| (read.clone(), d))).collect::<Option<_>>()?;
                (now == at.reads).then(|| at.clone())
            }).collect();
            if let Some(current) = current {
                for (rel, at) in scc.iter().zip(current) {
                    known.insert(rel.to_string(), at.digest);
                    frames.derived.insert(rel.to_string(), at);
                }
            } else {
                let reads: Vec<String> = all_reads.of_scc(scc).into_iter().filter(|r| !scc.contains(r)).collect();
                if let Some(rd) = reads.iter().map(|r| read_digest(r, &known, &derived, prog, leaves).map(|d| (r.clone(), d))).collect::<Option<Vec<_>>>() {
                    for rel in scc {
                        known.insert(rel.to_owned(), unkept_digest(rel, &rd));
                    }
                }
            }
        }
        // An input bound with no frame of this program's (its first run, where another program's relations are
        // shared, decision 0103) is as a current relation read it: unchanged where one read the digest the run binds.
        // It has no old rows to maintain from; a relation that read other rows is not current, and runs again.
        for (rel, d) in bound {
            if !inputs.contains_key(rel) && frames.derived.values().any(|at| at.reads.iter().any(|(r, x)| r == rel && x == d)) {
                inputs.insert(rel.clone(), *d);
            }
        }
        Ok(Retained { key, inputs, docs: (Default::default(), [0xff; 32]), frames: Some(frames), ..Default::default() })
    }

    /// Whether the last run's rows of `rel` are kept, as a cut takes them: held, or in the frames. Ordinary
    /// outputs are decoded from their retained rows; missing decoded text does not invalidate their derivation.
    /// `finding` retains its existing SQL decoding path, which also reads the current node and document facts.
    pub(in crate::code_rule_dsl) fn holds(&self, rel: &str, prog: &Placed) -> bool {
        self.rels.contains_key(rel) || (!(rel == "finding" && prog.fields().is_ok_and(|f| f.is_output(rel))) && self.frames.as_ref().is_some_and(|f| f.derived.contains_key(rel)))
    }

    /// Reads from the frames the rows this run reads of what they hold (t33 K4): the clean relations a stratum that
    /// runs reads, the relations of the strata that run (their old rows, to be maintained from), and the old rows of
    /// the inputs that changed. What no stratum that runs reads stays in the store.
    /// With `news`, a changed input whose documents are mostly the last run's is an `OldInput`: its changed documents'
    /// granules alone are bound, and the rest are its new rows, read in place.
    pub(in crate::code_rule_dsl) fn load(&mut self, store: &ContentStore, cut: &Cut, prog: &Placed, locations: &Locations, lazy: bool, news: Option<&NewInputs<'_>>) -> Result<Option<(usize, usize)>> {
        let Some(frames) = self.frames.as_ref() else { return Ok(Some((0, 0))) };
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the kept part's rows, from the frames".into());
        let (v, f) = (&prog.v, prog.fields()?);
        let running: Vec<RuleRef<'_>> = v.rules().filter(|r| !cut.clean.contains(v.rel_name(r.head().rel()))).collect();
        // A maintained run reads the rest where a stratum it reaches reads them (`admit`, decision 0117 step 4).
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        if !lazy {
            wanted.extend(running.iter().flat_map(|r| r.reads()).map(|a| v.rel_name(a.rel())).filter(|r| cut.clean.contains(*r)).map(str::to_owned));
            wanted.extend(running.iter().map(|r| v.rel_name(r.head().rel()).to_owned()));
        }
        // Outputs are consumers too. Admit their raw facts in the same batch as other demanded relations,
        // then decode them at the output boundary without scheduling their producing strata again.
        wanted.extend(f.outputs().filter(|r| cut.clean.contains(*r)).map(str::to_owned));
        let wanted: Vec<String> = wanted.into_iter().filter(|r| frames.derived.contains_key(r) && !self.rels.contains_key(r)).collect();
        let inputs: Vec<String> = cut.changed.iter().filter(|r| frames.inputs.contains_key(*r) && !frames.digest_only.contains(*r) && !self.input_srcs.contains_key(*r)).cloned().collect();
        let Some(bound) = frames.bound.clone() else { return Ok(Some((0, 0))) };
        // Each input read as an `OldInput`: the runs of its new rows of the unchanged documents, which hold as many
        // rows of each as the last run's did (else it is read whole).
        struct Partial {
            new: Arc<dyn Source>,
            runs: Vec<(u32, u32)>,
            kept: usize,
        }
        let partial: HashMap<&String, Partial> = news.map_or_else(HashMap::new, |news| {
            inputs
                .iter()
                .filter_map(|r| {
                    let new = (news.source)(r)?;
                    let was = frames.inputs[r].1.get()?;
                    let width = f.arity(r)?;
                    let g = group_col(prog, locations, r);
                    if width == 0 || g >= width || new.columns().len() != width {
                        return None;
                    }
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "load: an input's unchanged documents, in its new rows".into());
                    let mut counts: rustc_hash::FxHashMap<i64, u64> = rustc_hash::FxHashMap::default();
                    let (mut runs, mut kept, mut last): (Vec<(u32, u32)>, usize, Option<usize>) = (Vec::new(), 0, None);
                    new.documents_of(g, &mut |row, d| {
                        if news.same.contains(&d) {
                            *counts.entry(d).or_default() += 1;
                            if last.is_none_or(|l| l + 1 != row) {
                                runs.push((row as u32, kept as u32));
                            }
                            last = Some(row);
                            kept += 1;
                        }
                    });
                    if news.same.iter().any(|d| was.get(d).map_or(0, |w| w.1) != counts.get(d).copied().unwrap_or(0)) {
                        return None;
                    }
                    Some((r, Partial { new, runs, kept }))
                })
                .collect()
        });
        let changed_doc = |r: &String, d: &i64| !partial.contains_key(r) || news.is_some_and(|n| !n.same.contains(d));
        let atoms = store.atoms_handle();
        // Every row bound where it lies (decision 0401 F3), all of one commit of the space's blocks: the relations'
        // entries, and each changed input's granules as segments of one relation (of an `OldInput`, its changed
        // documents' granules alone).
        let decoded = (|| -> Option<_> {
            let mut rels = Vec::new();
            let mut sources = Vec::new();
            let mut rows = 0;
            let mut ranges_out: Vec<(String, Arc<dyn Source>, Arc<Ranges>)> = Vec::new();
            let _derived = crate::code_rule_dsl::logic::profile::span(|| "load: the derived relations' rows".into());
            for rel in &wanted {
                let name = &frames.names[rel];
                let entry = bound.entry(&name.name, &name.ident, DOCS_PART)?;
                let width = f.arity(rel)?;
                if entry.width() != width || Some(entry.digest()) != frames.derived.get(rel).map(|s| s.digest) {
                    return None;
                }
                rows += entry.len();
                rels.push((rel.clone(), Kept { rel: entry.rel(&atoms)?, width, output: None }));
            }
            drop(_derived);
            for rel in &inputs {
                let width = f.arity(rel)?;
                let name = &frames.names[rel];
                let owners = frames.atom_owned.contains(rel).then_some(&*atoms);
                let granules: BTreeSet<i64> = frames.inputs[rel].1.get().into_iter().flat_map(|docs| docs.keys()).filter(|d| changed_doc(rel, d)).map(|d| granule(d, owners)).collect();
                let parts: Vec<kept_read::Entry> = granules.iter().map(|g| bound.entry(&name.name, &name.ident, *g)).collect::<Option<_>>()?;
                if parts.iter().any(|p| p.width() != width) {
                    return None;
                }
                // Named as the program declares them, as the run's own rows are: the session's input delta by
                // document (`document_delta`) takes an old and a new source of the same columns, and with other names
                // every input fell back to the diff by rules (`r__old, !r`), a scan per row (192 s on 20 files).
                let names = f.columns(rel)?.into_iter().map(|(n, _)| n.to_owned()).collect();
                let held = kept_read::gathered(&parts, width, &atoms)?;
                let source = held.source(names);
                if let Some(p) = partial.get(rel) {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "load: an input's changed documents' old rows".into());
                    let g = group_col(prog, locations, rel);
                    let same = news?.same;
                    // The changed documents' rows in their granules (which hold other documents' too), each
                    // document's together (its range), in the order first met: picked by row, read where they lie.
                    let mut order: Vector<i64> = rerail_blocks::vector::Vector::new();
                    let mut by: rustc_hash::FxHashMap<i64, Vector<u32>> = rustc_hash::FxHashMap::default();
                    source.documents_of(g, &mut |row, d| {
                        if !same.contains(&d) {
                            by.entry(d).or_insert_with(|| {
                                order.push(d);
                                rerail_blocks::vector::Vector::new()
                            }).push(row as u32);
                        }
                    });
                    let mut pick: Vector<u32> = rerail_blocks::vector::Vector::new();
                    let mut ranges = Ranges::default();
                    for d in &order {
                        let ids = &by[d];
                        ranges.insert(*d, ((p.kept + pick.len()) as u32, ids.len() as u32));
                        pick.extend_from_slice(ids);
                    }
                    let was = frames.inputs[rel].1.get()?;
                    if was.iter().any(|(d, w)| !same.contains(&d) && by.get(&d).map_or(0, |ids| ids.len() as u64) != w.1) {
                        return None;
                    }
                    rows += pick.len();
                    let source: Arc<dyn Source> = Arc::new(OldInput { names: source.columns(), new: Arc::clone(&p.new), runs: p.runs.clone(), kept: p.kept, changed: source, pick, idx: (0..width).map(|_| std::sync::OnceLock::new()).collect() });
                    ranges_out.push((rel.clone(), Arc::clone(&source), Arc::new(ranges)));
                    sources.push((rel.clone(), source));
                    continue;
                }
                super::maintain::shape::input_read_whole();
                rows += held.rows();
                sources.push((rel.clone(), source));
            }
            super::maintain::shape::frames_read(wanted.len() + inputs.len());
            Some((rels, sources, rows, ranges_out))
        })();
        let Some((rels, sources, rows, ranges_out)) = decoded else {
            let frames = self.frames.take().expect("admitting frames");
            *self = Self::empty_kept(self.key, frames);
            crate::code_rule_dsl::logic::profile::decision("fact admission", || "frames changed during admission; recompute the cut and derive again".into());
            return Ok(None);
        };
        let count = rels.len() + sources.len();
        // A relation whose statistics part was stored for the rows it has now takes them, under the address it has
        // in this process: it is not counted or measured again (decision 0120 §3).
        if kept_stats::on() {
            let mut measured = 0;
            // `RERAIL_KEPT_STATS_TRACE`: why a relation admitted took no statistics.
            let mut none = 0;
            for (rel, k) in &rels {
                let Some(all) = kept_stats_of(frames, rel) else { none += 1; continue };
                let at = Arc::as_ptr(&k.rel) as *const u8 as usize;
                for ((name, _), m) in all {
                    measured += 1;
                    self.measured.insert((name, at), m);
                }
            }
            if std::env::var_os("RERAIL_KEPT_STATS_TRACE").is_some() {
                eprintln!("rerail: kept statistics: {} admitted, {measured} measured; {none} with no entry at their rows' digest", rels.len());
            }
            if let Some(f) = self.frames.as_mut() {
                f.stats_kept.extend(rels.iter().filter(|(_, k)| {
                    let at = Arc::as_ptr(&k.rel) as *const u8 as usize;
                    self.measured.keys().any(|(_, a)| *a == at)
                }).map(|(rel, _)| rel.clone()));
            }
            crate::code_rule_dsl::logic::profile::decision("kept statistics from the kept blocks (decision 0120 §3)", || {
                format!("{measured} measured, of {} relations admitted", rels.len())
            });
        }
        self.rels.extend(rels);
        self.input_srcs.extend(sources);
        self.framed_ranges.extend(ranges_out.into_iter().map(|(rel, s, r)| (rel, (s, r))));
        crate::code_rule_dsl::logic::profile::decision("inputs' old rows, beside their new ones (0353)", || format!("{} of {} changed inputs read as their new rows of the unchanged documents and the changed documents' old rows", partial.len(), inputs.len()));
        Ok(Some((count, rows)))
    }

    /// The inputs `kept` (`kept_inputs`, read at the waves' end) as this part's old sources, where its frames have
    /// them at the same digest: `load` does not admit them again.
    /// The old versions of the inputs its frames keep as their digest alone (decision 0458: a base relation's, made
    /// from the changed documents' old tapes), as the maintain mode reads an input's old rows (`old_source`).
    pub(in crate::code_rule_dsl) fn adopt_old_inputs(&mut self, olds: &HashMap<String, Arc<dyn Source>>) {
        let Some(frames) = self.frames.as_ref() else { return };
        for (rel, old) in olds.iter().filter(|(rel, _)| frames.digest_only.contains(*rel)) {
            self.input_srcs.entry(rel.clone()).or_insert_with(|| Arc::clone(old));
        }
    }

    pub(in crate::code_rule_dsl) fn adopt_inputs(&mut self, kept: &HashMap<String, KeptInput>) {
        let Some(frames) = self.frames.as_ref() else { return };
        let (mut taken, mut missing, mut other) = (0, 0, 0);
        for (rel, k) in kept {
            match frames.inputs.get(rel) {
                Some((d, _)) if *d == k.digest => {
                    if !self.input_srcs.contains_key(rel) {
                        self.input_srcs.insert(rel.clone(), Arc::clone(&k.source));
                        self.input_layouts.insert(rel.clone(), (Arc::clone(&k.source), Arc::clone(&k.layout)));
                    }
                    taken += 1;
                }
                Some(_) => other += 1,
                None => missing += 1,
            }
        }
        if std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some() {
            eprintln!("rerail: kept inputs adopted: {taken}; not in the part's frames: {missing}; at another digest: {other}; the part's frames hold {} inputs", frames.inputs.len());
        }
    }

    /// The frames this part was read from, taken out: a run of rounds carries them from one round's part to the
    /// next (`with_frames`), so a relation no round read stays in the store and is still held.
    pub(in crate::code_rule_dsl) fn take_frames(&mut self) -> Option<InKept> {
        self.frames.take()
    }

    /// `frames` carried into this part, which `cut` made: a relation it derived again is held only as derived (its
    /// stored rows are of the reads before), the others as stored.
    pub(in crate::code_rule_dsl) fn with_frames(&mut self, mut frames: InKept, cut: &Cut) {
        frames.derived.retain(|rel, _| cut.clean.contains(rel) && !self.rels.contains_key(rel));
        self.frames = Some(frames);
    }

    /// Whether the part knows input `rel`'s rows: bound by its run, or current in the frames.
    pub(in crate::code_rule_dsl) fn has_input(&self, rel: &str) -> bool {
        self.input_srcs.contains_key(rel) || self.frames.as_ref().is_some_and(|f| f.inputs.contains_key(rel))
    }

    /// Input `rel`'s rows, as bound or read back from the frames; with `docs`, only the rows of the owners it takes
    /// (`group_col`), and only their granules read.
    /// The inner `Err` says which granule the store no longer holds (swept, or never written): the part is not as it
    /// was kept, and the caller derives it again (a miss, not a failed run).
    pub(in crate::code_rule_dsl) fn read_input(&self, store: &ContentStore, prog: &Placed, locations: &Locations, rel: &str, docs: Option<&dyn Fn(i64) -> bool>) -> Result<std::result::Result<Option<Vec<Vector<i64>>>, String>> {
        let width = prog.fields()?.arity(rel).unwrap_or(0);
        let g = group_col(prog, locations, rel);
        let take = |row: &[i64]| docs.is_none_or(|d| d(owner_of(row, g)));
        if let Some(src) = self.input_srcs.get(rel) {
            let all: Vector<usize> = (0..width).collect();
            let mut out = Vec::new();
            let mut row = rerail_blocks::vector![0i64; width];
            for i in 0..src.len() {
                src.row_at(i, &all, &mut row);
                if width > 0 && take(&row) {
                    out.push(row.clone());
                }
            }
            return Ok(Ok(Some(out)));
        }
        let Some(frames) = self.frames.as_ref() else { return Ok(Ok(None)) };
        let Some(by_doc) = frames.inputs.get(rel).and_then(|(_, d)| d.get()) else { return Ok(Ok(None)) };
        if width == 0 {
            return Ok(Ok(Some(vec![rerail_blocks::vector::Vector::new(); by_doc.iter().map(|(_, (_, n))| n as usize).sum()])));
        }
        let atoms = store.atoms_handle();
        let owners = frames.atom_owned.contains(rel).then_some(&*atoms);
        let granules: BTreeSet<i64> = by_doc.keys().filter(|d| docs.is_none_or(|f| f(*d))).map(|d| granule(d, owners)).collect();
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("{rel}: {} granules, from the kept blocks", granules.len()));
        let Some(bound) = frames.bound.as_ref() else { return Ok(Ok(None)) };
        let name = &frames.names[rel];
        let mut out = Vec::new();
        for g in granules {
            let Some(entry) = bound.entry(&name.name, &name.ident, g) else {
                return Ok(Err(format!("{rel}: granule {g} of its rows is not kept")));
            };
            entry.each_rows(&mut |rows| out.extend(rows.chunks_exact(width.max(1)).filter(|r| take(r)).map(Vector::<i64>::from)));
        }
        Ok(Ok(Some(out)))
    }

    /// Another fact space needs the rows that the current space kept without reading.
    pub(super) fn admit_for_copy(&mut self, store: &ContentStore, program: &Placed, relations: &[String]) -> Result<bool> {
        if !self.admit(store, program, relations)? { return Ok(false); }
        self.rels.extend(self.late.take());
        Ok(true)
    }

    /// Derived relation `rel`'s rows, as held or read back from the frames.
    pub(in crate::code_rule_dsl) fn read_derived(&self, store: &ContentStore, rel: &str) -> Result<Option<Vec<Vector<i64>>>> {
        if let Some(k) = self.rels.get(rel) {
            let Some(flat) = k.rel.live_rows() else { return Ok(None) };
            return Ok(Some(flat.chunks_exact(k.width.max(1)).map(Vector::<i64>::from).collect()));
        }
        let Some(frames) = self.frames.as_ref().filter(|f| f.derived.contains_key(rel)) else { return Ok(None) };
        let name = &frames.names[rel];
        let Some(entry) = frames.bound.as_ref().and_then(|b| b.entry(&name.name, &name.ident, DOCS_PART)) else { return Ok(None) };
        let Some(held) = entry.rel(&store.atoms_handle()) else { return Ok(None) };
        let Some(flat) = held.live_rows() else { return Ok(None) };
        Ok(Some(flat.chunks_exact(entry.width().max(1)).map(Vector::<i64>::from).collect()))
    }

    /// What the frames held of this part before the run (`bind_kept`).
    /// The space its kept entries were bound from before the first wave (decision 0401 F3), if any.
    pub(in crate::code_rule_dsl) fn bound_space(&self) -> Option<&Arc<kept_read::Bound>> {
        self.frames.as_ref()?.bound.as_ref()
    }
    pub(in crate::code_rule_dsl) fn in_kept(&self) -> Option<&InKept> {
        self.frames.as_ref()
    }

    /// Keeps what this run changed of the part (t33 K4; decision 0401 F6), behind the run (the store's writer): each
    /// relation it derived whose rows or reads differ from its entry, and each input's granules whose documents' rows
    /// changed, with the input's digests by document. The relations kept as they were stay as stored.
    /// `sccs`: `prog`'s strata, computed once by the caller.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn write_kept(&self, store: &ContentStore, was: &InKept, prog: &Placed, domains: &lower::Domains, locations: &Locations, sccs: &crate::code_rule_dsl::logic::front::Components, cut: &Cut, inputs: &[(String, Arc<dyn Source>)], digests: &HashMap<String, u64>, unchanged: &BTreeSet<String>, deltas: &HashMap<String, crate::code_rule_dsl::logic::plan::Delta>, mut handed: HashMap<String, Handed>, inputs_done: Option<InputsWritten>, input_stats: &HashMap<String, Vec<(String, i64, i64, i64, i64)>>, digest_only: &BTreeMap<String, Option<usize>>) -> Result<usize> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the kept part's changes, put".into());
        let leaves = crate::code_rule_dsl::logic::plan::Leaves::of_placed(prog.clone());
        // Literal-only leaves are part of the derived relation's identity (leaf_identity), not runtime
        // inputs requiring a digest. Without this, a read of gproto("Generic") or an env fact makes
        // publication skip its consumer and forces that consumer to run in every new process.
        let f = prog.fields()?;
        let produced: BTreeSet<&str> = f.answers().iter().map(|(_, p)| f.rel_name(*p)).collect();
        let dynamic: BTreeSet<&str> = f.inputs().chain(sccs.iter().flatten()).chain(f.natives().iter().map(|n| f.rel_name(n.rel)))
            .chain(inputs.iter().map(|(r,_)|r.as_str())).chain(produced.iter().copied()).collect();
        let literals: BTreeSet<&str> = f.fact_rels().into_iter().map(|r| f.text(r.name())).filter(|r| !dynamic.contains(r)).collect();
        let atoms = store.atoms_handle();
        // Inputs first: their digests are what the relations derived from them record (put before the run when it
        // lets its inputs go, `inputs_done`).
        let InputsWritten { mut now, mut written } = match inputs_done {
            Some(done) => done,
            None => {
                let held = &self.input_docs;
                write_input_kept(store, was, prog, domains, locations, inputs, digests, &|rel: &str| held.get(rel).and_then(|d| d.get()).cloned(), input_stats, digest_only)
            }
        };
        // Every input the run bound, kept or not (an option's `env`, an earlier phase's export): its digest is what a
        // relation derived from it records.
        for (rel, d) in digests {
            now.entry(rel.clone()).or_insert(*d);
        }
        let derived: BTreeSet<&str> = sccs.iter().flatten().collect();
        let all_reads = Reads::of(prog)?;
        let trace = std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some();
        let mut tally = [(0usize, 0usize); 2];
        let (mut retagged, mut patched) = (0usize, 0usize);
        // The space's entries put (decision 0401 F6); a relation handed off with no entry (its rows dropped) counted.
        let mut kept: Vec<KeptPut> = Vec::new();
        let mut unkept = 0usize;
        // The relations put whole, each one's rows digested in a job of its own (MLDB's per-item `parallelMap`): the
        // loop below decides in its order, with each one's digest at hand. A relation whose digest a delta patches
        // (`by_delta`) is done by the loop.
        let whole: Vec<(&str, &Kept)> = derived.iter().copied()
            .filter(|rel| was.names.contains_key(*rel) && !cut.clean.contains(*rel) && !(unchanged.contains(*rel) && was.derived.contains_key(*rel)))
            .filter(|rel| !self.by_delta(was, rel, deltas))
            .filter_map(|rel| self.framed(rel).map(|k| (rel, k)))
            .collect();
        let mut made: HashMap<&str, u64> = {
            use rayon::prelude::*;
            let atoms = &atoms;
            // In the run and under the span open here, on whichever thread runs it: rayon's loops carry neither.
            let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
            crate::code_rule_dsl::logic::profile::on_cores(|| {
                whole.into_par_iter().filter_map(|(rel, k)| {
                    let _in = carry.enter();
                    let _span = crate::code_rule_dsl::logic::profile::span_at(carry.under(), || "a relation's rows, digested".into());
                    if k.rel.levels_shape().2 {
                        return None;
                    }
                    let types = frame_types(prog, domains, rel, k.rel.types());
                    held_digest(&Cells::new(atoms, &types), &k.rel).map(|d| (rel, d))
                }).collect()
            })
        };
        for scc in sccs {
            let reads: Vec<String> = all_reads.of_scc(scc).into_iter().filter(|r| !scc.contains(r) && !literals.contains(r.as_str())).collect();
            for rel in scc {
                let Some(name) = was.names.get(rel) else { continue };
                if cut.clean.contains(rel) {
                    if let Some(stored) = was.derived.get(rel) {
                        now.insert(rel.to_owned(), stored.digest);
                    } else if let Some(rd) = reads.iter().map(|r| read_digest(r, &now, &derived, prog, &leaves).map(|d| (r.clone(), d))).collect::<Option<Vec<_>>>() {
                        // Clean with no entry (a relation no round kept rows of, a view held in place): named by its
                        // reads, as `from_frame_snapshot` names it, so its readers' entries are written and current.
                        now.insert(rel.to_owned(), unkept_digest(rel, &rd));
                    }
                    continue;
                }
                let read_digests = reads.iter().map(|r| read_digest(r, &now, &derived, prog, &leaves).map(|d| (r.clone(), d))).collect::<Option<Vec<_>>>();
                // Its rows are as they were (decision 0117 step 4): its digest is the stored one, and its rows are
                // neither read nor put; its reads are told again if they changed.
                if unchanged.contains(rel)
                    && let Some(stored) = was.derived.get(rel)
                {
                    now.insert(rel.to_owned(), stored.digest);
                    if let Some(rd) = read_digests.filter(|rd| *rd != stored.reads) {
                        kept.push(kept_reads(name, DOCS_PART, rd));
                        retagged += 1;
                    }
                    continue;
                }
                // Changed by a delta (decision 0117 step 4) few against its rows: its digest patched by the delta's
                // rows, not taken over every row again; its rows put whole.
                if let (Some(delta), Some(stored), Some(rd)) = (deltas.get(rel), was.derived.get(rel), read_digests.as_ref())
                    && let Some(k) = self.framed(rel)
                    && self.by_delta(was, rel, deltas)
                    && let Some((gone, added)) = delta.rows(&k.rel)
                {
                    let (ng, na) = (gone.len() / k.width, added.len() / k.width);
                    let n_was = (k.rel.rows() + ng).saturating_sub(na);
                    let types = frame_types(prog, domains, rel, k.rel.types());
                    let d = crate::code_rule_dsl::logic::plan::rows_digest_patched(&Cells::new(&atoms, &types), stored.digest, n_was, &gone, &added);
                    now.insert(rel.to_owned(), d);
                    written += k.rel.rows();
                    patched += 1;
                    kept.push(kept_put(name, d, rd.clone(), types, &k.rel));
                    continue;
                }
                // Handed off as the run went (`Handed`): its entry put then, its reads told now.
                if let Some(h) = handed.remove(rel) {
                    now.insert(rel.to_owned(), h.d);
                    let Some(read_digests) = read_digests else { continue };
                    if was.derived.get(rel).is_some_and(|s| s.digest == h.d && s.reads == read_digests) {
                        continue;
                    }
                    // Its rows are the stored ones, or were put at the hand-off with no reads: its reads, told now,
                    // are merged into that entry.
                    if !h.changed || h.kept {
                        kept.push(kept_reads(name, DOCS_PART, read_digests));
                        retagged += usize::from(!h.changed);
                        written += if h.changed { h.n } else { 0 };
                    } else {
                        unkept += 1;
                    }
                    continue;
                }
                // A relation with no rows to keep (a view) is named by its reads' digests: derived again from the
                // same reads, it has the same rows, so its readers' entries stay current.
                let Some((k, d)) = self.framed(rel).and_then(|k| match made.remove(rel) {
                    Some(d) => Some((k, d)),
                    None => held_digest(&Cells::new(&atoms, &frame_types(prog, domains, rel, k.rel.types())), &k.rel).map(|d| (k, d)),
                }) else {
                    if let Some(rd) = &read_digests {
                        now.insert(rel.to_owned(), unkept_digest(rel, rd));
                    }
                    continue;
                };
                let n = k.rel.rows();
                now.insert(rel.to_owned(), d);
                let Some(read_digests) = read_digests else { continue };
                if was.derived.get(rel).is_some_and(|s| s.digest == d && s.reads == read_digests) {
                    continue;
                }
                let types = frame_types(prog, domains, rel, k.rel.types());
                if trace {
                    let i = usize::from(was.derived.get(rel).is_some_and(|s| s.digest == d));
                    tally[i].0 += 1;
                    tally[i].1 += n;
                    eprintln!("rerail: derived entry put ({}): {rel}, {n} rows, types {}", if i == 0 { "rows changed" } else { "reads changed" }, String::from_utf8_lossy(&types));
                }
                written += n;
                kept.push(kept_put(name, d, read_digests, types, &k.rel));
            }
        }
        if trace {
            eprintln!("rerail: derived entries put: {} relations ({} rows) whose rows changed; {} ({} rows) whose rows are the same and whose reads changed; {retagged} retagged; {patched} digests patched by their deltas", tally[0].0, tally[0].1, tally[1].0, tally[1].1);
        }
        if kept_stats::on() {
            self.stats_puts(was, &now, &mut kept);
        }
        crate::code_rule_dsl::logic::profile::decision("the kept facts as blocks (decision 0401)", || format!("this program: {} entries put; {unkept} relations handed off with no entry (their rows dropped)", kept.len()));
        // Behind the run (decision 0104): the part's fixpoint is reached, and these are its results for the next run;
        // a wait for the writer's room here holds no wave. Nothing changed: nothing written (decision 0227, F-R3).
        // Mapped (decision 0212): every atom an entry holds is in an `s` column (a text-owned input's owners too, in
        // its documents' table), its digests and granules are of content, so a forked process's entries are written as
        // the store numbers them (`KeptWrite`), and read back as the reader does.
        if !kept.is_empty() {
            crate::code_rule_dsl::logic::profile::behind(|| store.queue_kept(was.space, kept));
        }
        Ok(written)
    }
}

/// What `write_input_kept` put: each input's digest (what the relations derived from it record), and the rows put.
pub(in crate::code_rule_dsl) struct InputsWritten {
    now: HashMap<String, u64>,
    written: usize,
}

/// The inputs' part of `Retained::write_kept`: each input's granules whose documents' rows changed, and its digests
/// by document, queued as each input's job makes them. They depend on the inputs alone, so a run that lets its inputs
/// go after their last reader (Q74) writes them before it runs, and hands what it wrote to `write_kept`.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn write_input_kept(store: &ContentStore, was: &InKept, prog: &Placed, domains: &lower::Domains, locations: &Locations, inputs: &[(String, Arc<dyn Source>)], digests: &HashMap<String, u64>, input_docs: &(dyn Fn(&str) -> Option<Arc<Docs>> + Sync), input_stats: &HashMap<String, Vec<(String, i64, i64, i64, i64)>>, digest_only: &BTreeMap<String, Option<usize>>) -> InputsWritten {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the inputs' frames".into());
    let mut now: HashMap<String, u64> = HashMap::new();
    let mut written = 0;
    // Digests of content and granules by content (decision 0212): what every process reads back alike.
    let atoms = store.atoms_handle();
    // Inputs first: their digests are what the relations derived from them record. Each input is a job of its
    // own (its rows' digests by document, its changed granules' rows, each encoded); their frames are taken in
    // the inputs' order.
    let atoms_ref = &atoms;
    // Each input's frames are queued as its job makes them (Q74), into the run's wave (`Carry`: the writer takes
    // the wave whole, in one transaction, with the derived relations' frames when they are written together; before
    // the run, in a wave of their own, `Boundary::CrossInputs`, sealed once they are made): placed in segments of
    // their own (decision 0295 §1), their bytes leave the run's heap then. Collected to be queued at the end, every
    // input's encoded frames were held at once beside the inputs' rows (270 MB at a cold run's peak, 300 files).
    let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
    let carry = &carry;
    let one_input = |rel: &String, source: &Arc<dyn Source>| -> Option<(u64, usize)> {
        let _in = carry.enter();
        let _span = crate::code_rule_dsl::logic::profile::span(|| "an input's frames (job)".into());
        let queue = |kept: Vec<KeptPut>| {
            if !kept.is_empty() {
                store.queue_kept(was.space, kept);
            }
        };
        let atoms: &crate::content_store::atoms::Atoms = atoms_ref;
        // Its entries in the space's kept facts (decision 0401 F6).
        let mut kept: Vec<KeptPut> = Vec::new();
        let mut written = 0;
        let name = was.names.get(rel)?;
        let width = source.columns().len();
        let types = types_of(prog, domains, rel);
        let cells = if types.len() == width { Cells::new(atoms, &types) } else { Cells::plain(width) };
        let digest = digests.get(rel).copied().unwrap_or_else(|| digest(source.as_ref(), &cells));
        // Its statistics this run measured (an input no keeper counts, a semi-join's: `kept_inputs`' `stats_of`),
        // kept beside its frame at these rows (decision 0120 §3's part), unchanged rows or not, unless an entry of
        // these rows is kept already: the statistics of the same rows are the same, and a warm run writes nothing.
        if let Some(columns) = input_stats.get(rel).filter(|_| kept_stats::on() && !was.holds_stats(name, digest)) {
            let all: kept_stats::ByDigest = [((rel.clone(), digest), (columns.clone(), Vec::new(), BTreeSet::new()))].into_iter().collect();
            let words = kept_stats::to_words(&kept_stats::encode(&all));
            kept.push(kept_flat(name, kept_stats::PART, digest, Vec::new(), vec![b'i'], crate::content_store::kept_facts::BlockFlat::of_rows(&words, 1, true)));
        }
        if was.inputs.get(rel).is_some_and(|(d, _)| *d == digest) {
            queue(kept);
            return Some((digest, written));
        }
        // A base relation is kept as its documents' digest (decision 0458): its documents' entry alone, one row, no
        // granules; its rows are its documents' tapes.
        if let Some(bound) = digest_only.get(rel) {
            let rows = BlockFlat::of_row(&[0, digest as i64, bound.unwrap_or_else(|| source.len()) as i64]);
            kept.push(kept_flat(name, DOCS_PART, digest, vec![(DIGEST_ONLY_READ.to_owned(), 1)], vec![b'i'; 3], rows));
            queue(kept);
            return Some((digest, written));
        }
        // A relation of no columns is its count: one document's entry, its digest the input's (`digest`).
        if width == 0 {
            let n = source.len() as u64;
            let rows = BlockFlat::of_row(&[0, (digest ^ n.wrapping_mul(0xff51_afd7_ed55_8ccd)) as i64, n as i64]);
            kept.push(kept_flat(name, DOCS_PART, digest, Vec::new(), vec![b'i'; 3], rows));
            queue(kept);
            return Some((digest, written));
        }
        let g = group_col(prog, locations, rel).min(width - 1);
        let atom_owner = types.len() == width && atom_owners(&types, g);
        let owners = atom_owner.then_some(atoms);
        let granule = |d: i64| granule(d, owners);
        let docs = input_docs(rel).filter(|d| d.digest() == digest).unwrap_or_else(|| Arc::new(Docs::Made(docs_of(source.as_ref(), g, &cells))));
        let old = was.inputs.get(rel).and_then(|(_, d)| d.get()).map(Arc::clone);
        // The granules whose documents differ, or that hold a document come or gone.
        let differ = |doc: &i64| old.as_ref().is_none_or(|o| o.get(doc) != docs.get(doc));
        let mut changed: BTreeSet<i64> = docs.keys().filter(|d| differ(d)).map(granule).collect();
        if let Some(o) = &old {
            changed.extend(o.keys().filter(|d| !docs.contains_key(d)).map(granule));
        }
        // Each changed granule's rows, grouped by owner, owners ascending (`kept_inputs` reads each owner's rows as
        // one range), each in the source's order, read where they lie by the store's writer as it packs them
        // (`KeptWhat::Picked`, decision 0353 step 8), never gathered: a maintained input (`Layered`) gives each owner's
        // rows as a range of the source that holds them (`GranuleRows`); another's row ids are an index written in a
        // block of the run (`picked_ids`), read by them (`PickedRows`). Gathered into flat vectors, every changed
        // granule's rows were held on the heap until written (1.7 GB on 3,089 files), a copy of the input's blocks.
        let layered = source.as_any().and_then(|a| a.downcast_ref::<Layered>());
        match layered {
            Some(l) => {
                let mut owners_of: BTreeMap<i64, Vec<i64>> = changed.iter().map(|g| (*g, Vec::new())).collect();
                for owner in docs.keys().filter(|d| changed.contains(&granule(*d))) {
                    owners_of.get_mut(&granule(owner)).expect("a changed granule").push(owner);
                }
                for (part, mut owners) in owners_of {
                    owners.sort_unstable();
                    let mut rows = GranuleRows { sources: Vec::new(), runs: Vec::new(), starts: Vec::new(), memos: Vec::new(), n: 0, width };
                    for o in &owners {
                        let (src, start, n) = l.owner_range(*o);
                        rows.push(src, start, n);
                    }
                    written += rows.n;
                    kept.push(kept_picked(name, part, types.clone(), Box::new(rows)));
                }
            }
            None if !changed.is_empty() => {
                let (ids, ranges) = picked_ids(source.as_ref(), g, &changed, &granule);
                for (part, (lo, hi)) in ranges {
                    written += (hi - lo) as usize;
                    let picked = PickedRows { source: Arc::clone(source), ids: Arc::clone(&ids), lo, hi, width, memo: std::sync::atomic::AtomicU32::new(crate::code_rule_dsl::facts::vtab::NO_MEMO) };
                    kept.push(kept_picked(name, part, types.clone(), Box::new(picked)));
                }
            }
            None => {}
        }
        // The granules its rows lie in, recorded with its documents' frame (`GRANULES_READ`): a run that reads none of
        // its owners admits them from this, decoding no frame (`kept_inputs`, Q91).
        let mask = docs.keys().fold(0u64, |m, d| m | granule_bit(granule(d)));
        // Its documents read where they lie as the writer packs them, in its canonical order (decision 0353 step 8):
        // a kept entry's by position, others by document (`DocRows`).
        let what = KeptWhat::Picked(Box::new(DocRows::of(Arc::clone(&docs))));
        kept.push(KeptPut { rel: name.name.clone(), ident: name.ident, part: DOCS_PART, digest, reads: vec![(GRANULES_READ.to_owned(), mask)], what, types: docs_types(atom_owner) });
        queue(kept);
        Some((digest, written))
    };
    // An input whose rows are those its entry holds (its digest known, the same as the entry's) puts its statistics
    // at most, a few words: its job is run here, not on the pool. A warm run's every input is such, and as 1,146
    // jobs of a microsecond or two each they woke the pool's 17 threads, which spun 0.1 s of CPU between them.
    let unchanged = |rel: &String| was.names.contains_key(rel) && digests.get(rel).is_some_and(|d| was.inputs.get(rel).is_some_and(|(k, _)| k == d));
    let mut framed_inputs: Vec<Option<(u64, usize)>> = inputs.iter().map(|(rel, source)| if unchanged(rel) { one_input(rel, source) } else { None }).collect();
    let rest: Vec<usize> = (0..inputs.len()).filter(|&k| !unchanged(&inputs[k].0)).collect();
    if !rest.is_empty() {
        use rayon::prelude::*;
        // In the run, on whichever thread runs it: its frames' placement reads the machine's device memory.
        let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
        let done: Vec<Option<(u64, usize)>> = crate::code_rule_dsl::logic::profile::on_cores(|| {
            rest.par_iter().map(|&k| {
                let _in = carry.enter();
                let (rel, source) = &inputs[k];
                one_input(rel, source)
            }).collect()
        });
        for (k, d) in rest.into_iter().zip(done) {
            framed_inputs[k] = d;
        }
    }
    for ((rel, _), done) in inputs.iter().zip(framed_inputs) {
        let Some((digest, n)) = done else { continue };
        now.insert(rel.clone(), digest);
        written += n;
    }
    InputsWritten { now, written }
}

/// The row ids of `source`'s rows in the granules `changed`, an index in a block of the run: granule by granule
/// (ascending), owner by owner (ascending) within it, each owner's rows in the source's order; and each granule's
/// range of it. Two passes over the owner column `g`, read in place: one counts each owner's rows, one places them.
fn picked_ids(source: &dyn Source, g: usize, changed: &BTreeSet<i64>, granule: &dyn Fn(i64) -> i64) -> (Arc<crate::code_rule_dsl::facts::rows::Part>, Vec<(i64, (u32, u32))>) {
    use crate::code_rule_dsl::facts::rows::{Life, Part};
    let n = source.len();
    // Per owner of a changed granule: its rows' count, then where its next row goes.
    let mut at: BTreeMap<(i64, i64), u32> = BTreeMap::new();
    for r in 0..n {
        let owner = super::memo::document_of(source.value(r, g));
        let gr = granule(owner);
        if changed.contains(&gr) {
            *at.entry((gr, owner)).or_insert(0) += 1;
        }
    }
    let mut ranges: Vec<(i64, (u32, u32))> = Vec::new();
    let mut next = 0u32;
    for ((gr, _), count) in at.iter_mut() {
        match ranges.last_mut() {
            Some((g0, (_, hi))) if g0 == gr => *hi += *count,
            _ => ranges.push((*gr, (next, next + *count))),
        }
        let c = *count;
        *count = next;
        next += c;
    }
    let ids = Part::with_index_room(Life::Run, 1, next as usize);
    for r in 0..n {
        let owner = super::memo::document_of(source.value(r, g));
        if let Some(slot) = at.get_mut(&(granule(owner), owner)) {
            // SAFETY: slot `*slot` is below the `next` rows reserved, each written once, none read before `written`.
            unsafe { ids.put(*slot, 0, r as i64) };
            *slot += 1;
        }
    }
    // SAFETY: rows `0..next` are written.
    unsafe { ids.written(next) };
    (Arc::new(ids), ranges)
}

/// A granule's rows, picked from its input where they lie by their ids (`picked_ids`): the store's writer reads them
/// there as it packs them (`KeptPicked`), a row's search starting from the last one's (`Source::value_memo`: a
/// granule's rows run by owner, so a view finds their document once; any memo is correct, so threads share it).
struct PickedRows {
    source: Arc<dyn Source>,
    ids: Arc<crate::code_rule_dsl::facts::rows::Part>,
    lo: u32,
    hi: u32,
    width: usize,
    memo: std::sync::atomic::AtomicU32,
}

impl crate::content_store::kept_facts::KeptPicked for PickedRows {}

impl rerail_blocks::packed::RowsIn for PickedRows {
    fn n(&self) -> usize {
        (self.hi - self.lo) as usize
    }
    fn arity(&self) -> usize {
        self.width
    }
    fn get(&self, r: usize, c: usize) -> i64 {
        use std::sync::atomic::Ordering::Relaxed;
        let was = self.memo.load(Relaxed);
        let mut memo = was;
        let x = self.source.value_memo(self.ids.get(self.lo + r as u32, 0) as usize, c, &mut memo);
        if memo != was {
            self.memo.store(memo, Relaxed);
        }
        x
    }
    /// A chunk with the search's memo read once and kept once, not a value at a time.
    fn read(&self, c: usize, at: rerail_blocks::packed::At<'_>, out: &mut [i64]) {
        use rerail_blocks::packed::At;
        use std::sync::atomic::Ordering::Relaxed;
        let was = self.memo.load(Relaxed);
        let mut memo = was;
        let mut one = |r: usize| self.source.value_memo(self.ids.get(self.lo + r as u32, 0) as usize, c, &mut memo);
        match at {
            At::From(r) => {
                for (j, o) in out.iter_mut().enumerate() {
                    *o = one(r + j);
                }
            }
            At::Rows(rows) => {
                for (o, &r) in out.iter_mut().zip(rows) {
                    *o = one(r as usize);
                }
            }
        }
        if memo != was {
            self.memo.store(memo, Relaxed);
        }
    }
}

/// A derived relation lent to the keep (decision 0401 F6): its rows as one relation in its block, its dead bits and
/// its levels, held until the store's writer has copied them.
struct LentRel {
    rel: Arc<InRel>,
    block: crate::code_rule_dsl::facts::rows::InBlock,
}

impl KeptRows for LentRel {
    fn relation(&self) -> &rerail_blocks::relation::Relation {
        self.block.relation()
    }
    fn dead(&self) -> &[u64] {
        self.rel.dead_bits()
    }
    fn levels(&self) -> &[u32] {
        self.rel.levels()
    }
}

/// `rel`'s entry: its rows lent whole, dead ones too, its dead bits and levels beside them (`LentRel`). `None` for a
/// relation with no block of its rows (a view, no columns). Not its rows with their kills written into their block
/// (`InRel::in_block`): a maintained relation whose old rows are bound over the read-only mapping took none, and went
/// unkept, so the next run derived it again.
pub(in crate::code_rule_dsl) fn kept_rows(name: &KeptRel, digest: u64, reads: Vec<(String, u64)>, types: Vec<u8>, rel: &Arc<InRel>) -> Option<KeptPut> {
    let block = rel.rows_in_block()?;
    Some(KeptPut { rel: name.name.clone(), ident: name.ident, part: DOCS_PART, digest, reads, types, what: KeptWhat::Rows(Box::new(LentRel { rel: Arc::clone(rel), block })) })
}

/// `rel`'s entry, whatever it holds: its rows lent whole where they lie in a block (`kept_rows`), or, for a relation
/// with none (no rows, or a view over a source), its live rows flat: the derived fact written once. An empty relation
/// or a view went unkept, and its stratum ran again in every next run (a phase's unchanged strata, 12 of 19).
pub(in crate::code_rule_dsl) fn kept_put(name: &KeptRel, digest: u64, reads: Vec<(String, u64)>, types: Vec<u8>, rel: &Arc<InRel>) -> KeptPut {
    if let Some(put) = kept_rows(name, digest, reads.clone(), types.clone(), rel) {
        return put;
    }
    // A relation with no block of its rows (a view's rows read from its source, or none): its live rows read where
    // they lie as the writer packs them (decision 0353 step 8), in canonical order, with their levels.
    let ids = (rel.rows() != rel.len()).then(|| (0..rel.len() as u32).filter(|&id| !crate::code_rule_dsl::logic::native::component::is_dead(&rel.dead, id)).collect());
    let levels = rel.live_levels();
    KeptPut { rel: name.name.clone(), ident: name.ident, part: DOCS_PART, digest, reads, what: KeptWhat::Picked(Box::new(LiveRows { rel: Arc::clone(rel), ids, levels })), types }
}

/// A relation's live rows where they lie (`InRel::value`, a view's read from its source through it): every row, or
/// those of `ids`; with each live row's level where it has them.
struct LiveRows {
    rel: Arc<InRel>,
    ids: Option<Vector<u32>>,
    levels: Option<Vector<u32>>,
}

impl crate::content_store::kept_facts::KeptPicked for LiveRows {
    fn given(&self) -> bool {
        false
    }
    fn levels(&self) -> Option<std::borrow::Cow<'_, [u32]>> {
        self.levels.as_deref().map(std::borrow::Cow::Borrowed)
    }
}

impl rerail_blocks::packed::RowsIn for LiveRows {
    fn n(&self) -> usize {
        self.ids.as_ref().map_or(self.rel.len(), Vector::len)
    }
    fn arity(&self) -> usize {
        self.rel.k
    }
    fn get(&self, r: usize, c: usize) -> i64 {
        self.rel.value(self.ids.as_ref().map_or(r, |ids| ids[r] as usize), c)
    }
}

/// Rows not held as a relation, as an entry of their own (an input's granule or documents, statistics' words): `n`
/// rows of `types.len()` columns, flat, kept as given (decision 0456): read by position, in their order, every row.
use crate::content_store::kept_facts::BlockFlat;

fn kept_flat(name: &KeptRel, part: i64, digest: u64, reads: Vec<(String, u64)>, types: Vec<u8>, rows: BlockFlat) -> KeptPut {
    KeptPut { rel: name.name.clone(), ident: name.ident, part, digest, reads, what: KeptWhat::Picked(Box::new(rows)), types }
}

/// An entry of rows read where they lie (`KeptWhat::Picked`), kept in the order given: an input's granule.
fn kept_picked(name: &KeptRel, part: i64, types: Vec<u8>, rows: Box<dyn crate::content_store::kept_facts::KeptPicked>) -> KeptPut {
    KeptPut { rel: name.name.clone(), ident: name.ident, part, digest: 0, reads: Vec::new(), what: KeptWhat::Picked(rows), types }
}

/// An input's documents as rows of (document, sum, count), read where they lie (`Docs`): a kept entry's by position;
/// otherwise by document, `keys` naming each row's.
struct DocRows {
    docs: Arc<Docs>,
    keys: Option<Vec<i64>>,
}

impl DocRows {
    fn of(docs: Arc<Docs>) -> DocRows {
        let keys = (!matches!(*docs, Docs::Kept(_))).then(|| docs.keys().collect());
        DocRows { docs, keys }
    }
}

impl crate::content_store::kept_facts::KeptPicked for DocRows {
    fn given(&self) -> bool {
        false
    }
}

impl rerail_blocks::packed::RowsIn for DocRows {
    fn n(&self) -> usize {
        self.docs.len()
    }
    fn arity(&self) -> usize {
        3
    }
    fn get(&self, r: usize, c: usize) -> i64 {
        let (d, (sum, n)) = match (&self.keys, &*self.docs) {
            (None, Docs::Kept(e)) => return e.packed().get(r, c),
            (Some(k), docs) => (k[r], docs.get(&k[r]).expect("a document of its keys")),
            (None, _) => unreachable!("keys for documents not kept"),
        };
        [d, sum as i64, n as i64][c]
    }
}

/// An input's granule's rows where they lie (decision 0353 step 8): runs of rows, each a range of one of its
/// sources (`sources`, each held once), the granule's rows in the runs' order. A row is read by a search of the
/// runs' starts, and its source's search for it starts from its run's memo (`Source::value_memo`: a run is an
/// owner's rows, so a view finds their document once; any memo is correct, so readers on several threads share it).
struct GranuleRows {
    sources: Vec<Arc<dyn Source>>,
    /// Each run's source, its first row there, its rows.
    runs: Vec<(u32, u32, u32)>,
    /// Each run's first row in the granule.
    starts: Vec<u32>,
    memos: Vec<std::sync::atomic::AtomicU32>,
    n: usize,
    width: usize,
}

impl GranuleRows {
    /// Rows `start..start + n` of `src` after the rows held, joined to the last run where they follow it.
    fn push(&mut self, src: &Arc<dyn Source>, start: u32, n: u32) {
        if n == 0 {
            return;
        }
        let k = match self.sources.iter().position(|s| Arc::ptr_eq(s, src)) {
            Some(k) => k as u32,
            None => {
                self.sources.push(Arc::clone(src));
                (self.sources.len() - 1) as u32
            }
        };
        match self.runs.last_mut() {
            Some((w, at, m)) if *w == k && *at + *m == start => *m += n,
            _ => {
                self.runs.push((k, start, n));
                self.starts.push(self.n as u32);
                self.memos.push(std::sync::atomic::AtomicU32::new(crate::code_rule_dsl::facts::vtab::NO_MEMO));
            }
        }
        self.n += n as usize;
    }
}

impl crate::content_store::kept_facts::KeptPicked for GranuleRows {}

impl rerail_blocks::packed::RowsIn for GranuleRows {
    fn n(&self) -> usize {
        self.n
    }
    fn arity(&self) -> usize {
        self.width
    }
    fn get(&self, r: usize, c: usize) -> i64 {
        let k = self.starts.partition_point(|&s| s as usize <= r) - 1;
        use std::sync::atomic::Ordering::Relaxed;
        let (w, at, _) = self.runs[k];
        let was = self.memos[k].load(Relaxed);
        let mut memo = was;
        let x = self.sources[w as usize].value_memo(at as usize + (r - self.starts[k] as usize), c, &mut memo);
        if memo != was {
            self.memos[k].store(memo, Relaxed);
        }
        x
    }
}

/// The entry's rows as they are, with `reads` (a frame's retag).
fn kept_reads(name: &KeptRel, part: i64, reads: Vec<(String, u64)>) -> KeptPut {
    KeptPut { rel: name.name.clone(), ident: name.ident, part, digest: 0, reads, types: Vec::new(), what: KeptWhat::Reads }
}

/// A derived relation the cross-file run handed off once no later stratum read it (`plan::Hooks::hand_off`), with
/// its entry put then: its rows' digest, and its rows lent to the store's writer. The run drops its rows, so the
/// part's relations are not all held to its end to be written (Q74: 2,700 of them, the cross part's peak on 300
/// files). Not held for this process's next run either: it binds them in the kept blocks, as a new process does.
pub(in crate::code_rule_dsl) struct Handed {
    n: usize,
    d: u64,
    /// Its rows are not the stored ones (`was`); else its entry stays, its reads retagged if they changed.
    changed: bool,
    /// Its kept entry was put at the hand-off (`of`); its reads are told at the write.
    kept: bool,
}

impl Handed {
    /// `held`'s entry as `write_kept` puts one written whole; `None` for a relation of no rows of its own (a view).
    /// `was`: its stored entry's digest. Its rows changed, its kept entry is put now (decision 0401 F6), its reads
    /// given at the part's write (`write_kept`, a retag the writer merges into it): the writer holds its rows until
    /// its copy is made.
    pub(in crate::code_rule_dsl) fn of(store: &ContentStore, prog: &Placed, domains: &lower::Domains, rel: &str, held: &Arc<InRel>, was: Option<&InKept>) -> Option<Handed> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "a relation handed off, its entry put".into());
        let n = held.rows();
        let types = frame_types(prog, domains, rel, held.types());
        let atoms = store.atoms_handle();
        let d = held_digest(&Cells::new(&atoms, &types), held)?;
        let changed = was.and_then(|w| w.derived_digest(rel)) != Some(d);
        let mut kept = false;
        if changed
            && let Some(w) = was
            && let Some(name) = w.names.get(rel)
        {
            store.queue_kept(w.space, vec![kept_put(name, d, Vec::new(), types.clone(), held)]);
            kept = true;
        }
        Some(Handed { n, d, changed, kept })
    }
}

impl Retained {
    /// Whether `rel`'s digest is patched by its delta (decision 0117 step 4), not taken over its rows again: its
    /// delta few against its rows, and no levels (whose order a delta moves: decision 0090).
    fn by_delta(&self, was: &InKept, rel: &str, deltas: &HashMap<String, crate::code_rule_dsl::logic::plan::Delta>) -> bool {
        let (Some(delta), Some(_), Some(_), Some(k)) = (deltas.get(rel), was.derived.get(rel), was.names.get(rel), self.framed(rel)) else { return false };
        if k.width == 0 || k.rel.has_levels() {
            return false;
        }
        let (ng, na) = delta.counts(k.width);
        (ng + na) * 4 <= k.rel.rows().max(4096)
    }

    /// Each kept relation's statistics of its rows now, as a part of its frame under the rows' digest (`now`, as
    /// the frames name them; decision 0120 §3).
    fn stats_puts(&self, was: &InKept, now: &HashMap<String, u64>, kept: &mut Vec<KeptPut>) {
        let mut by: HashMap<&str, kept_stats::ByDigest> = HashMap::new();
        let at_of = |rel: &str| self.framed(rel).map(|k| Arc::as_ptr(&k.rel) as *const u8 as usize);
        let late = self.late_stats.borrow();
        for ((name, at), m) in self.measured.iter().chain(&*late) {
            if let Some(d) = now.get(name.as_str()).filter(|_| at_of(name) == Some(*at)) {
                by.entry(name.as_str()).or_default().insert((name.clone(), *d), m.clone());
            }
        }
        for (rel, all) in by {
            let Some(name) = was.names.get(rel) else { continue };
            let digest = now[rel];
            // Admitted with them at these rows: the stored part is these statistics already.
            if was.stats_kept.contains(rel) && was.derived.get(rel).is_some_and(|s| s.digest == digest) {
                continue;
            }
            let words = BlockFlat::of_bytes(&kept_stats::encode(&all));
            if std::env::var_os("RERAIL_KEPT_STATS_TRACE").is_some() {
                eprintln!("rerail: kept statistics written: {rel} ({} entries)", all.len());
            }
            kept.push(kept_flat(name, kept_stats::PART, digest, Vec::new(), vec![b'i'], words));
        }
    }
}

mod kept_source;
mod kept_stats;
pub(in crate::code_rule_dsl) mod kept_read;

#[cfg(test)]
pub mod kept_tests;

/// The frame digest of `held`'s live rows (`rows_digest`), read where they lie a batch at a time: no copy of the
/// relation (it was copied whole for each frame written, 460 MB of a 300-file run's peak); a view's through it.
pub(in crate::code_rule_dsl) fn held_digest(cells: &Cells, held: &InRel) -> Option<u64> {
    crate::code_rule_dsl::logic::plan::source_rows::held_digest(cells, held)
}
