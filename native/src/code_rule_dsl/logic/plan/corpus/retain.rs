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

use super::{Error, Result, lower};
use crate::code_rule_dsl::logic::plan::Cells;
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front;
use crate::code_rule_dsl::logic::plan::body::Placed;
use crate::code_rule_dsl::logic::program_image::{Lit, RuleRef};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use crate::code_rule_dsl::logic::native::component::InRel;
use crate::content_store::{ContentStore, EncodedFrame, FramePut, FrameRel, FrameRows, FrameStored};
use crate::content_store::kept_facts::{KeptPut, KeptRows, KeptWhat};
use crate::code_rule_dsl::logic::program_image::passes::Parts as _;

/// A relation as the last run derived it: the engine's relation, and its decoded rows when it is an output.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Kept {
    pub(in crate::code_rule_dsl) rel: Arc<InRel>,
    pub(in crate::code_rule_dsl) width: usize,
    pub(in crate::code_rule_dsl) output: Option<(std::sync::Arc<crate::code_rule_dsl::logic::plan::output::Rows>, Vec<usize>)>,
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
    frames: Option<InFrames>,
    /// The relations' rows read from the frames during a maintained run, when a stratum the run reached reads them
    /// (decision 0117 step 4, `admit`): taken with `rels` from then on. Rows read, and relations.
    late: std::cell::RefCell<HashMap<String, Kept>>,
    admitted: std::cell::Cell<(usize, usize)>,
    /// The statistics stored with the rows `admit` read (decision 0120 §3), under the address each has in this
    /// process. Taken as `measured` is (smoking gun 9, t34).
    late_stats: std::cell::RefCell<HashMap<(String, usize), Measured>>,
    /// The relations this run derived past the budget: written to the frames with the kept ones (`write_frames`,
    /// `parts`), then let go (`let_go_past`), never held for the next run of this process.
    past: HashMap<String, Kept>,
    /// The relations the run holds as views of an input's columns (`maintain::held_views`): never kept, their old
    /// rows the view over their input's old rows (decision 0090), read in place.
    views: BTreeMap<String, (String, Vec<usize>)>,
}

/// A source's rows by document: each document's sum of row hashes and its row count.
pub(in crate::code_rule_dsl) type DocDigests = rustc_hash::FxHashMap<i64, (u64, u64)>;

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

/// An order-independent digest of `source`'s rows (a set: rows are distinct), with its row count, of their content
/// (`Cells`, decision 0212): the same in every process, whichever ids it gave their atoms.
pub(in crate::code_rule_dsl) fn digest(source: &dyn Source, cells: &Cells) -> u64 {
    let mut sum: u64 = 0;
    each_row(source, cells, |_, h| sum = sum.wrapping_add(h));
    sum ^ (source.len() as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
}

/// Each of `source`'s rows as it holds it, with its digest of its content (`Cells::row`), read and keyed a chunk at
/// a time: one read of the atoms a chunk.
pub(in crate::code_rule_dsl) fn each_row(source: &dyn Source, cells: &Cells, mut f: impl FnMut(&[i64], u64)) {
    let width = source.columns().len();
    if width == 0 {
        return;
    }
    const CHUNK: usize = 4096;
    let mut keyed = Vec::with_capacity(width * CHUNK);
    // A held relation's rows, read where they lie, part by part (in its order, as `row_at` gives them).
    if crate::code_rule_dsl::logic::native::component::each_held_batch(source, width, |batch| {
        keyed.clear();
        keyed.extend_from_slice(batch);
        cells.key(&mut keyed);
        for (raw, key) in batch.chunks_exact(width).zip(keyed.chunks_exact(width)) {
            f(raw, crate::code_rule_dsl::logic::plan::row_digest(key));
        }
    })
    .is_some()
    {
        return;
    }
    let all: Vec<usize> = (0..width).collect();
    let mut raw = vec![0i64; width * CHUNK];
    let n = source.len();
    for start in (0..n).step_by(CHUNK) {
        let m = (n - start).min(CHUNK);
        for r in 0..m {
            source.row_at(start + r, &all, &mut raw[r * width..(r + 1) * width]);
        }
        keyed.clear();
        keyed.extend_from_slice(&raw[..m * width]);
        cells.key(&mut keyed);
        for r in 0..m {
            f(&raw[r * width..(r + 1) * width], crate::code_rule_dsl::logic::plan::row_digest(&keyed[r * width..(r + 1) * width]));
        }
    }
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
pub(in crate::code_rule_dsl) fn document_delta(rel: &str, old: &Arc<dyn Source>, was: Option<&DocDigests>, new: &Arc<dyn Source>, g: usize, cells: &Cells) -> Option<((Arc<dyn Source>, Arc<dyn Source>), Arc<DocDigests>)> {
    document_delta_except(rel, old, was, new, g, cells, None, None)
}

/// `document_delta`, given the documents whose rows are as they were (`same`: their content, key and memo plan are
/// the last run's, `write_run_docs`): their digests are `was`'s, and only the other owners' rows are hashed, not
/// every row of every document. Each owner's rows are counted first, a pass over the owner column: a document of
/// `same` whose count is not `was`'s (a relation fed otherwise than from its documents' entries) sends the whole
/// relation to `document_delta`'s pass.
///
/// `old_ranges`: each owner's rows in `old` (a kept input's layout), by which the old rows of a document that
/// differs are read, not found by reading every row's owner.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn document_delta_except(rel: &str, old: &Arc<dyn Source>, was: Option<&DocDigests>, new: &Arc<dyn Source>, g: usize, cells: &Cells, same: Option<&rustc_hash::FxHashSet<i64>>, old_ranges: Option<&Ranges>) -> Option<((Arc<dyn Source>, Arc<dyn Source>), Arc<DocDigests>)> {
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
    let all: Vec<usize> = (0..width).collect();
    let mut row = vec![0i64; width];
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
    let except = |was: &DocDigests, same: &FxHashSet<i64>| -> Option<(DocDigests, Vec<usize>)> {
        // Each row's document as the source gives it (`documents_of`: a view of a batch's documents names each
        // document's rows by its key, reading no row).
        let _s = crate::code_rule_dsl::logic::profile::span(|| "document delta: its owners counted".into());
        let mut counts: rustc_hash::FxHashMap<i64, u64> = rustc_hash::FxHashMap::default();
        let mut others: Vec<usize> = Vec::new();
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
        let mut by: DocDigests = same.iter().filter_map(|d| was.get(d).map(|w| (*d, *w))).collect();
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
            taken = by_doc(old.as_ref(), &mut row);
            &taken
        }
    };
    // The rows of the documents not of `same`, when only those were hashed: the new rows of a document that differs
    // are among them.
    let mut new_others: Option<Vec<usize>> = None;
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
    let differ: FxHashSet<i64> = was.iter().filter(|(d, h)| now.get(*d) != Some(*h)).map(|(d, _)| *d).chain(now.keys().filter(|d| !was.contains_key(*d)).copied()).collect();
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
            let mut out = Vec::new();
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
    Some((rows_delta(rel, width, old, &gone_ids, new, &made_ids), Arc::new(now)))
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
    let all: Vec<usize> = (0..width).collect();
    let mut row = vec![0i64; width];
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
    let (mut plus, mut minus): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
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
    let all: Vec<usize> = (0..width).collect();
    let (mut a, mut b) = (vec![0i64; width], vec![0i64; width]);
    let prefix = old.len() <= new.len()
        && (0..old.len()).all(|r| {
            old.row_at(r, &all, &mut a);
            new.row_at(r, &all, &mut b);
            a == b
        });
    if prefix {
        let made: Vec<usize> = (old.len()..new.len()).collect();
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
        let mut fresh: Vec<usize> = Vec::new();
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
}

/// The relations a `nearest` literal reads.
pub(in crate::code_rule_dsl) const TREE: [&str; 2] = ["node", "child"];

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

    pub(super) fn of_scc(&self, scc: &BTreeSet<String>) -> BTreeSet<String> {
        let v = self.f.v;
        let rules = || scc.iter().filter_map(|rel| self.by_head.get(rel.as_str())).flatten();
        let mut out: BTreeSet<String> = rules().flat_map(|r| r.reads()).map(|a| v.rel_name(a.rel()).to_owned()).collect();
        // A `nearest` literal climbs the tree: it reads `node` and `child`, which an edit changes.
        if rules().any(|r| r.body().any(|l| matches!(l.view(), Lit::Nearest { .. }))) {
            out.extend(TREE.iter().map(|r| (*r).to_owned()));
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
/// `sccs`: `prog`'s strata (`front::strata`), computed once by the caller for every pass over the program.
pub(in crate::code_rule_dsl) fn cut(kept: Option<&Retained>, key: [u8; 32], prog: &Placed, sccs: &[BTreeSet<String>], inputs: &HashMap<String, u64>) -> Result<Cut> {
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
            dirty.extend(scc.iter().cloned());
        } else if scc.iter().all(|r| views.contains_key(r)) {
            // A view of unchanged reads: not run, and not clean (no rows of it are kept to bind).
        } else if !scc.iter().all(|r| kept.is_some_and(|k| k.holds(r, prog))) {
            // Not kept (past the budget, or recomputed in its readers, decision 0099) but of unchanged reads: run
            // again, its rows as before, so its readers stay clean.
            ran += 1;
        } else {
            clean.extend(scc.iter().cloned());
        }
    }
    Ok(Cut { clean, strata: (ran, sccs.len()), changed })
}

impl Cut {
    /// `cross` without the clean strata's rules: the relations dirty rules read become inputs, and clean outputs are
    /// left to `outputs`.
    ///
    /// The cut is a part of `cross`'s version (decision 0244), read and made by its handles.
    pub(in crate::code_rule_dsl) fn program(&self, cross: &lower::Lowered) -> super::Result<lower::Lowered> {
        use crate::code_rule_dsl::logic::program_image::PartFields;
        let (dirty, all, none) = (|rel: &str| !self.clean.contains(rel), |_: &str| true, |_: &str| false);
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
            let part = v.part(&dirty, &inputs, &outputs, PartFields { natives: &dirty, lattice: &all, gates }).map_err(super::Error::Plan)?;
            crate::code_rule_dsl::logic::version_trace::note_versions("session.cut", v, &part);
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
    pub(in crate::code_rule_dsl) fn outputs(&self, kept: &Retained, outputs: &[String]) -> Vec<(String, std::sync::Arc<crate::code_rule_dsl::logic::plan::output::Rows>, Vec<usize>)> {
        outputs
            .iter()
            .filter(|o| self.clean.contains(*o))
            .filter_map(|o| kept.rels.get(o).and_then(|k| k.output.clone()).map(|(lines, nodes)| (o.clone(), lines, nodes)))
            .collect()
    }

    /// The relations this run derives, in stratum order: kept for the next run, as far as the budget goes.
    /// `sccs`: the program's strata, as `cut` was given them.
    pub(in crate::code_rule_dsl) fn derived(&self, sccs: &[BTreeSet<String>]) -> Result<Vec<String>> {
        Ok(sccs.iter().flatten().filter(|r| !self.clean.contains(*r)).cloned().collect())
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
    outputs: &BTreeMap<String, (std::sync::Arc<crate::code_rule_dsl::logic::plan::output::Rows>, Vec<usize>)>,
    sources: &[(String, Arc<dyn Source>)],
    measured: HashMap<(String, usize), Measured>,
    input_docs: HashMap<String, Arc<DocDigests>>,
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
        // rounds, decision 0331), so its next run's delta is made against it and its readers are maintained.
        if (held.view.is_some() || held.viewed) && !(indexes && held.held_source().is_some()) {
            if std::env::var_os("RERAIL_SHAPE_TRACE").is_some() {
                eprintln!("RERAIL_SHAPE_TRACE not kept {rel}: a view ({} rows; viewed {}, over a source {})", held.rows(), held.viewed, held.view.is_some());
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

/// `source` (input `rel`), or the last run's source of the same rows: same name, columns and digest, in any kept
/// part. Read in place of the copy, so its column indexes and statistics are kept with it, not built again (0094
/// §4). Its digest goes to `digests`, which the session's cut reads. An unheld column reads as 0 in both, so the
/// rows read alike whatever the program.
pub(in crate::code_rule_dsl) fn kept_source(
    parts: Option<&HashMap<[u8; 32], Retained>>,
    rel: &str,
    source: Arc<dyn Source>,
    cells: &Cells,
    digests: &mut HashMap<String, u64>,
) -> Arc<dyn Source> {
    let Some(parts) = parts else { return source };
    let d = digest(source.as_ref(), cells);
    digests.insert(rel.to_owned(), d);
    let columns = source.columns();
    if let Some(same) = parts.values().find_map(|p| p.input_srcs.get(rel).filter(|s| p.inputs.get(rel) == Some(&d) && s.columns() == columns).cloned()) {
        return same;
    }
    source
}

impl Retained {
    /// The part taken under program `key`, another program of the same cross part (`keys`): its rows are the cross
    /// program's, its inputs compared by digest (`cut`), and its documents answered only under the file-local
    /// program they were read under (`docs`).
    pub(in crate::code_rule_dsl) fn rekeyed(mut self, key: [u8; 32]) -> Self {
        self.key = key;
        self
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

    /// Reads from the frames the rows of those of `rels` they hold and this part has not read (decision 0117 step 4):
    /// a maintained run's level reads, in one batch, the old rows of the strata it reaches and the kept rows they read.
    /// `false`: the frames changed under the run (a frame missing, or of another shape).
    pub(in crate::code_rule_dsl) fn admit(&self, store: &ContentStore, prog: &Placed, rels: &[String]) -> Result<bool> {
        let Some(frames) = self.frames.as_ref() else { return Ok(true) };
        let wanted: Vec<&String> = {
            let late = self.late.borrow();
            rels.iter().filter(|r| frames.derived.contains_key(*r) && !self.rels.contains_key(*r) && !late.contains_key(*r)).collect()
        };
        if wanted.is_empty() {
            return Ok(true);
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("{} relations' kept rows, from the frames", wanted.len()));
        let keys: Vec<_> = wanted.iter().map(|r| (frames.names[*r].clone(), DOCS_PART)).collect();
        // Their levels, where stored (decision 0090), in the same batch: a recursive stratum reached is maintained by
        // levels only if its kept relations have them. Not a key of the admission's check: a relation may have none.
        let mut with_level_keys = keys.clone();
        with_level_keys.extend(wanted.iter().map(|r| (frames.names[*r].clone(), LEVELS_PART)));
        // Their statistics, stored for these rows (decision 0120 §3): taken with them, not counted from them.
        if kept_stats::on() {
            with_level_keys.extend(wanted.iter().map(|r| (frames.names[*r].clone(), kept_stats::PART)));
        }
        let admitted = store.admit_frame_parts(&frames.space, &with_level_keys).map_err(|e| Error::Plan(e.to_string()))?;
        super::maintain::shape::frames_read(admitted.len());
        if !super::kept_strata::admission_matches(&keys, &admitted, |(r, _)| frames.derived.get(&r.name)) {
            return Ok(false);
        }
        let levels = &admitted;
        let mut late = self.late.borrow_mut();
        let (mut rows, mut n) = self.admitted.get();
        for (rel, key) in wanted.into_iter().zip(&keys) {
            let Some(got) = admitted.get(key).and_then(|f| f.rows().ok()) else { return Ok(false) };
            let Some(width) = prog.fields()?.arity(rel) else { return Ok(false) };
            if got.types.len() != width {
                return Ok(false);
            }
            rows += got.n;
            n += 1;
            let Some(kept) = with_levels(frames, levels, rel, key, got) else { return Ok(false) };
            if kept_stats::on() {
                self.take_stats(frames, &admitted, rel, &kept);
            }
            late.insert(rel.clone(), Kept { rel: kept, width, output: None });
        }
        self.admitted.set((rows, n));
        Ok(true)
    }

    /// The statistics part stored with `rel`'s rows (`admitted` holds it, when stored), taken for `held`, the
    /// relation those rows are in this process. A part of another digest is of other rows: not taken.
    fn take_stats(&self, frames: &InFrames, admitted: &HashMap<(FrameRel, i64), Arc<crate::content_store::AdmittedFrame>>, rel: &str, held: &Arc<InRel>) {
        let Some(d) = frames.derived.get(rel).map(|s| s.digest) else { return };
        let Some(part) = admitted.get(&(frames.names[rel].clone(), kept_stats::PART)).filter(|p| p.stored.digest == d) else { return };
        let Some(all) = part.rows().ok().and_then(|r| kept_stats::from_words(&r.rows)) else { return };
        let at = Arc::as_ptr(held) as *const u8 as usize;
        let mut late = self.late_stats.borrow_mut();
        for ((name, _), m) in all {
            late.insert((name, at), m);
        }
    }

    /// The rows' count of each of `rels` the frames hold, read from their headers (their rows are not decoded).
    pub(in crate::code_rule_dsl) fn stored_rows(&self, store: &ContentStore, rels: &[String]) -> HashMap<String, usize> {
        let Some(frames) = self.frames.as_ref() else { return HashMap::new() };
        let keys: Vec<_> = rels.iter().filter(|r| frames.derived.contains_key(*r)).map(|r| (frames.names[r].clone(), DOCS_PART)).collect();
        let Ok(got) = store.admit_frame_parts(&frames.space, &keys) else { return HashMap::new() };
        super::maintain::shape::frames_read(got.len());
        got.into_iter().filter_map(|((r, _), f)| Some((r.name, f.len()?))).collect()
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
    pub(in crate::code_rule_dsl) fn input_docs(&self, rel: &str) -> Option<&Arc<DocDigests>> {
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
    /// The documents the last run read, under the memo plan `program`, as its frame holds them (`read_run_docs`).
    pub(in crate::code_rule_dsl) fn set_docs(&mut self, docs: super::input_delta::Docs, program: [u8; 32]) {
        self.docs = (docs, program);
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
    pub(in crate::code_rule_dsl) fn index_report(&self, probes: &HashMap<(String, Vec<usize>), u64>) -> String {
        let base = |name: &str| -> String {
            name.split(" [").next().unwrap_or(name).trim_end_matches("+delta").to_owned()
        };
        let mut shapes: HashMap<String, BTreeMap<Vec<usize>, u64>> = HashMap::new();
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
                let sets: Vec<Vec<usize>> = shapes.get(rel.as_str()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
                let counts: Vec<u64> = shapes.get(rel.as_str()).map(|m| m.values().copied().collect()).unwrap_or_default();
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
        let mut top: Vec<(&(String, Vec<usize>), &u64)> = probes.iter().collect();
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

/// What `load` reads an input's old rows by, beside the documents the last run read: the documents whose content and
/// key are as they were (`same`), and each changed input's new rows, as the run binds them.
pub(in crate::code_rule_dsl) struct NewInputs<'a> {
    pub(in crate::code_rule_dsl) same: &'a rustc_hash::FxHashSet<i64>,
    pub(in crate::code_rule_dsl) source: &'a dyn Fn(&str) -> Option<Arc<dyn Source>>,
}

/// An input's rows as the last run had them, where most of its documents are as they were (0353: no copy): the new
/// rows of the documents that are the last run's, read in place in the run's own source, then the other documents'
/// old rows, read from the frames. Decoded whole, every input a stratum reached was copied into columns and hashed
/// row by row against its documents' digests (O125 step 05: 105 inputs, 1.6 s).
pub(in crate::code_rule_dsl) struct OldInput {
    names: Vec<String>,
    new: Arc<dyn Source>,
    /// The runs of `new`'s rows of the unchanged documents: each run's first row in `new`, and here.
    runs: Vec<(u32, u32)>,
    /// The rows read from `new`, before the changed documents' old rows.
    kept: usize,
    changed: Arc<dyn Source>,
    /// Each column's rows by value, made where first probed (the engine's lookups, as `Held`'s).
    idx: Vec<std::sync::OnceLock<rustc_hash::FxHashMap<i64, Vec<u32>>>>,
}

impl OldInput {
    /// Where row `row` lies: in `new` (`true`) or among the changed documents' rows, and its row there.
    fn at(&self, row: usize) -> (bool, usize) {
        if row >= self.kept {
            return (false, row - self.kept);
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
        self.kept + self.changed.len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.new.estimated_matches(column, batch)
    }
    fn indexed(&self, _column: usize) -> bool {
        true
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let idx = self.idx.get(column)?.get_or_init(|| {
            let mut by: rustc_hash::FxHashMap<i64, Vec<u32>> = rustc_hash::FxHashMap::default();
            for r in 0..self.len() {
                by.entry(self.value(r, column)).or_default().push(r as u32);
            }
            by
        });
        let mut out: Vec<usize> = values.iter().flat_map(|v| idx.get(v).into_iter().flatten().map(|&r| r as usize)).collect();
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
    fn len(&self) -> usize {
        self.0.len()
    }
    fn estimated_len(&self) -> usize {
        self.0.estimated_len()
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.0.estimated_matches(column, batch)
    }
    fn lookup_cost(&self, column: usize) -> f64 {
        self.0.lookup_cost(column)
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        self.0.estimated_matches_all(columns)
    }
    fn composite(&self) -> bool {
        self.0.composite()
    }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        self.0.lookup_all(equal)
    }
    fn doc_column(&self) -> Option<usize> {
        self.0.doc_column()
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        self.0.lookup_in(column, values, doc)
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.0.kept_to_doc(column)
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        self.0.filter_eq(eqs)
    }
    fn made_on_lookup(&self) -> bool {
        self.0.made_on_lookup()
    }
    fn complete_rows(&self) -> bool {
        self.0.complete_rows()
    }
    fn rows_owned(&self, g: usize, owned: &dyn Fn(i64) -> bool) -> Option<Vec<usize>> {
        self.0.rows_owned(g, owned)
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        self.0.expect_lookups(column, keys);
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        let names = self.0.columns();
        let stats = self.0.statistics()?;
        stats.into_iter().map(|(c, a, b, x, y)| Some((self.1[names.iter().position(|n| *n == c)?].clone(), a, b, x, y))).collect()
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
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        self.0.range_lookup(column, lower, upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
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
    fn lookup_primary_batch(&self, values: &[Vec<i64>]) -> Option<Vec<usize>> {
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
    before: Vec<u32>,
    added: Arc<dyn Source>,
    /// Each changed owner's rows in `added`.
    appended: rustc_hash::FxHashMap<i64, (u32, u32)>,
    /// Each owner's rows as a range of this input's rows (`ranges_now`), found where first asked.
    now: std::sync::OnceLock<Arc<Ranges>>,
}

impl Layered {
    /// `kept` (each owner's rows at `ranges`) with each owner of `changed` holding its new rows instead, named
    /// `names`.
    pub(in crate::code_rule_dsl) fn new(names: Vec<String>, kept: Arc<dyn Source>, ranges: Arc<Ranges>, changed: Vec<(i64, Vec<Vec<i64>>)>) -> Layered {
        let width = names.len();
        let mut dead: Vec<(u32, u32)> = changed.iter().filter_map(|(o, _)| ranges.get(o).copied()).filter(|(_, n)| *n > 0).collect();
        dead.sort_unstable();
        let mut before = Vec::with_capacity(dead.len() + 1);
        let mut at = 0u32;
        before.push(0);
        for (_, n) in &dead {
            at += n;
            before.push(at);
        }
        let mut columns: Vec<Vec<i64>> = vec![Vec::new(); width];
        let mut appended = rustc_hash::FxHashMap::default();
        for (owner, rows) in changed {
            let start = columns.first().map_or(0, Vec::len) as u32;
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

    /// This input after a further patch (a session's next edit): laid over the same kept rows, not over this one, so
    /// a session of many edits reads one layer. `changed`'s rows come first among the appended ones (an
    /// `OwnerPatch::plus` names its rows by their places there), then the rows of the owners an earlier edit
    /// changed and this one did not, as they are here.
    pub(in crate::code_rule_dsl) fn then(&self, changed: Vec<(i64, Vec<Vec<i64>>)>) -> Layered {
        let width = self.names.len();
        let all: Vec<usize> = (0..width).collect();
        let now: rustc_hash::FxHashSet<i64> = changed.iter().map(|(o, _)| *o).collect();
        let mut earlier: Vec<(i64, (u32, u32))> = self.appended.iter().filter(|(o, _)| !now.contains(o)).map(|(o, r)| (*o, *r)).collect();
        earlier.sort_unstable();
        let mut owners = changed;
        owners.extend(earlier.into_iter().map(|(o, (start, n))| {
            let rows = (start..start + n)
                .map(|r| {
                    let mut row = vec![0i64; width];
                    self.added.row_at(r as usize, &all, &mut row);
                    row
                })
                .collect();
            (o, rows)
        }));
        Layered::new(self.names.clone(), Arc::clone(&self.kept), Arc::clone(&self.ranges), owners)
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
                if let Some(at) = self.from_kept(start as usize) {
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
    fn from_kept(&self, r: usize) -> Option<usize> {
        let k = self.dead.partition_point(|&(s, _)| (s as usize) <= r);
        if k > 0 && r < (self.dead[k - 1].0 + self.dead[k - 1].1) as usize {
            return None;
        }
        Some(r - self.before[k] as usize)
    }

    /// Owner `owner`'s rows, flat: its new rows if it changed, else its kept ones.
    pub(in crate::code_rule_dsl) fn owner_rows(&self, owner: i64) -> Vec<i64> {
        let width = self.names.len();
        let all: Vec<usize> = (0..width).collect();
        let (from, (start, n)): (&dyn Source, (u32, u32)) = match self.appended.get(&owner) {
            Some(r) => (self.added.as_ref(), *r),
            None => (self.kept.as_ref(), self.ranges.get(&owner).copied().unwrap_or((0, 0))),
        };
        let mut out = vec![0i64; width * n as usize];
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
    fn indexed(&self, column: usize) -> bool {
        self.kept.indexed(column)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        let live = self.live_kept();
        let mut out: Vec<usize> = self.kept.lookup(column, values)?.into_iter().filter_map(|r| self.from_kept(r)).collect();
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
    fn st(p: &front::Program) -> Vec<BTreeSet<String>> { front::strata(p).unwrap().0 }
    /// A program's version, as a run reads it (t36 item 7).
    fn pl(p: &front::Program) -> Placed { Placed::of_program(p).unwrap() }
    use super::*;

    /// A kept input with its delta reads as its rows now: the changed owners' kept rows gone, their new rows after
    /// the rest, and a lookup finds a row through the kept rows' index at its place now.
    #[test]
    fn a_layered_input_reads_its_kept_rows_less_the_changed_owners_with_their_new_rows() {
        let node = |d: i64, i: i64| d << 32 | i;
        let rows = [[node(1, 0), 10], [node(1, 1), 11], [node(2, 0), 20], [node(2, 1), 21], [node(3, 0), 30], [node(4, 0), 40]];
        let kept = crate::code_rule_dsl::logic::native::component::columns_source(vec!["n".into(), "v".into()], (0..2).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect());
        let ranges: Ranges = [(1, (0, 2)), (2, (2, 2)), (3, (4, 1)), (4, (5, 1))].into_iter().collect();
        // Owner 2 edited, 3 gone, 5 new.
        let l = Layered::new(vec!["n".into(), "v".into()], kept, Arc::new(ranges), vec![(2, vec![vec![node(2, 0), 22]]), (3, Vec::new()), (5, vec![vec![node(5, 0), 50]])]);
        let read: Vec<(i64, i64)> = (0..l.len()).map(|r| (l.value(r, 0), l.value(r, 1))).collect();
        assert_eq!(read, vec![(node(1, 0), 10), (node(1, 1), 11), (node(4, 0), 40), (node(2, 0), 22), (node(5, 0), 50)]);
        for (r, (n, v)) in read.iter().enumerate() {
            assert_eq!(l.lookup(1, &[*v]), Some(vec![r]));
            assert!(l.lookup(0, &[*n]).unwrap().contains(&r));
        }
        assert_eq!(l.lookup(1, &[20, 21, 30]), Some(Vec::new()));
        assert_eq!(l.owner_rows(2), vec![node(2, 0), 22]);
        assert_eq!(l.owner_rows(1), vec![node(1, 0), 10, node(1, 1), 11]);
        assert!(l.owner_rows(3).is_empty());
    }

    /// A session's second edit is laid over the same kept rows (`Layered::then`): it reads as the rows now, an owner
    /// the first edit changed keeps its rows unless the second changes it again, the new owners' rows come first
    /// among the appended ones, and each owner's range (`ranges_now`) holds exactly its rows.
    #[test]
    fn a_second_patch_is_laid_over_the_same_kept_rows() {
        let node = |d: i64, i: i64| d << 32 | i;
        let rows = [[node(1, 0), 10], [node(1, 1), 11], [node(2, 0), 20], [node(3, 0), 30], [node(4, 0), 40]];
        let kept = crate::code_rule_dsl::logic::native::component::columns_source(vec!["n".into(), "v".into()], (0..2).map(|c| Some(rows.iter().map(|r| r[c]).collect())).collect());
        let ranges: Ranges = [(1, (0, 2)), (2, (2, 1)), (3, (3, 1)), (4, (4, 1))].into_iter().collect();
        let first = Layered::new(vec!["n".into(), "v".into()], Arc::clone(&kept), Arc::new(ranges), vec![(2, vec![vec![node(2, 0), 22]]), (3, vec![vec![node(3, 0), 33]])]);
        // The second edit: owner 3 again, owner 1 gone, owner 5 new; owner 2 keeps its first edit's rows.
        let second = first.then(vec![(3, vec![vec![node(3, 0), 34], vec![node(3, 1), 35]]), (1, Vec::new()), (5, vec![vec![node(5, 0), 50]])]);
        assert!(Arc::ptr_eq(&second.kept, &kept), "one layer over the kept rows");
        let read: BTreeSet<(i64, i64)> = (0..second.len()).map(|r| (second.value(r, 0), second.value(r, 1))).collect();
        assert_eq!(read, BTreeSet::from([(node(4, 0), 40), (node(2, 0), 22), (node(3, 0), 34), (node(3, 1), 35), (node(5, 0), 50)]));
        assert_eq!(second.len(), 5);
        // The second edit's rows first among the appended ones.
        let live = second.live_kept();
        assert_eq!((second.value(live, 1), second.value(live + 1, 1), second.value(live + 2, 1)), (34, 35, 50));
        for (r, (_, v)) in read.iter().enumerate() {
            assert_eq!(second.lookup(1, &[*v]).map(|f| f.len()), Some(1), "row {r}");
        }
        let now = second.ranges_now();
        for (o, want) in [(2, vec![22]), (3, vec![34, 35]), (4, vec![40]), (5, vec![50])] {
            let (start, n) = now[&o];
            assert_eq!((start..start + n).map(|r| second.value(r as usize, 1)).collect::<Vec<_>>(), want, "owner {o}");
        }
        assert!(!now.contains_key(&1));
        // A session's first source's ranges are found by one pass; a Layered's are its own.
        let found = session_ranges(&kept, 0).unwrap();
        assert_eq!(found.get(&1), Some(&(0, 2)));
        let as_source: Arc<dyn Source> = Arc::new(second);
        assert_eq!(session_ranges(&as_source, 0).unwrap().get(&3).map(|r| r.1), Some(2));
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

    /// A relation the run binds from an earlier phase is read as an input by `from_frames`, declared or not, so it is
    /// one of the identities the cross part's space is read by (Q19): an undeclared one was left out of the preload.
    #[test]
    fn a_relation_bound_from_an_earlier_phase_is_read_by_identity_declared_or_not() {
        let text = ".decl e(x: number)\n.input e\n.decl out(x: number)\nout(x) :- e(x), types__wgen(x).\n.output out\n";
        let placed = pl(&crate::code_rule_dsl::logic::rr::parse(text).unwrap());
        let binds = [3; 32];
        let rels = frame_rels(&[&placed], &binds, &["types__wgen"]).unwrap();
        for rel in ["e", "types__wgen"] {
            assert!(rels.contains(&FrameRel { name: rel.into(), ident: input_ident(rel, &placed, &binds) }), "{rel} read by identity");
        }
    }

    #[test]
    fn unchanged_gated_programs_reuse_only_complete_retained_results() {
        use crate::code_rule_dsl::logic::magic;
        let p = front::prune(crate::code_rule_dsl::logic::rr::parse(include_str!("../golden/programs/workload.rr")).unwrap());
        let p = magic::magic_adorn(p, &magic::Options { gate:true, ..Default::default() }).unwrap();
        assert!(!p.gates.is_empty());
        let (strata,_) = front::strata(&p).unwrap();
        let key=[1;32];
        let inputs:HashMap<_,_>=p.inputs.iter().map(|r|(r.clone(),7)).collect();
        let rels=strata.iter().flatten().map(|name| {
            let width=p.decls[name.as_str()].len();
            (name.clone(),Kept { rel:InRel::flat(&[],vec![b'i';width]),width,output:None })
        }).collect();
        let mut kept=Retained {key,inputs:inputs.clone(),rels,input_srcs:Default::default(),input_layouts:Default::default(),framed_ranges:Default::default(),measured:Default::default(),input_docs:Default::default(),docs:Default::default(),frames:None,admitted:Default::default(),late:Default::default(),late_stats:Default::default(),past:Default::default(),views:Default::default()};
        let result=cut(Some(&kept),key,&pl(&p),&st(&p),&inputs).unwrap();
        assert_eq!(result.strata,(0,strata.len()));
        let v=crate::code_rule_dsl::logic::program_image::Builder::of_program(&p).unwrap().finish().unwrap().0;
        let lowered=lower::Lowered {version:v,domains:Default::default(),ops:[0;3],dialect:crate::code_rule_dsl::language::GrammarDialect::Python};
        let reused=result.program(&lowered).unwrap();
        let placed=reused.placed().unwrap();
        // A view of an input's columns (Q37: `kind(n, k) :- node(_, n, k, _, _).`) is never kept: its rule stays, to
        // hold it again. Nothing else runs.
        let left:BTreeSet<String>=placed.v.rules().map(|r| placed.v.rel_name(r.head().rel()).to_owned()).collect();
        assert_eq!(left,BTreeSet::from(["docof".to_owned(),"kind".to_owned()]));
        assert!(placed.v.rules().all(|r| crate::code_rule_dsl::logic::plan::input_view_shape(&placed.v,r,|n| p.inputs.iter().any(|i| i==n)).is_some()));
        assert!(placed.fields().unwrap().gates().is_empty());
        let mut changed=inputs.clone();
        *changed.values_mut().next().unwrap()=8;
        for (candidate,rows) in [(key,changed),([2;32],inputs.clone())] {
            assert_eq!(cut(Some(&kept),candidate,&pl(&p),&st(&p),&rows).unwrap().strata,(strata.len(),strata.len()));
        }
        // A kept relation missing (not a view's: those are not kept, so their absence changes nothing).
        let missing=kept.rels.keys().find(|r| !left.contains(r.as_str())).unwrap().clone();
        kept.rels.remove(&missing);
        assert_eq!(cut(Some(&kept),key,&pl(&p),&st(&p),&inputs).unwrap().strata,(strata.len(),strata.len()));
    }
}

/// Granules an input's frames are split into, by document (K1's partition): an edit rewrites the granules of the
/// documents it changed, not the input.
const GRANULES: i64 = 64;
/// The part of an input's frames that holds each document's digest (`DocDigests`), as rows (document, sum, rows);
/// its frame's digest is the input's (`digest`). Granule `g` is part `1 + g`.
const DOCS_PART: i64 = 0;

/// The frame of the documents a kept run read (`write_run_docs`), named by the part's program (`binds`).
const RUN_DOCS: &str = "rerail:run_docs";

/// The frame of the documents the last run of the part's program `binds` read: an identity the cross part's frames
/// are read by before the first wave (`frame_rels_kept`'s, with the run's own).
pub(in crate::code_rule_dsl) fn run_docs_rel(binds: [u8; 32]) -> FrameRel {
    FrameRel { name: RUN_DOCS.to_owned(), ident: binds }
}

/// The tag of a run's memo plan, as its documents' frame records it (its digest).
fn docs_tag(program: &[u8; 32]) -> u64 {
    u64::from_le_bytes(program[..8].try_into().expect("8 bytes"))
}

/// Queues the documents this run read (`docs`: each one's content as the memo knows it and its key, by path) as a
/// frame of the part's space, under the memo plan `program`: a later process reads its inputs' deltas from the
/// changed documents' memo entries (`input_delta::changed`), as a session does, not by hashing every document.
/// Not written when the frame holds them already (`was`): a run that changed nothing writes nothing.
pub(in crate::code_rule_dsl) fn write_run_docs(store: &ContentStore, space: [u8; 32], binds: [u8; 32], program: &[u8; 32], docs: &super::input_delta::Docs, was: Option<&super::input_delta::Docs>) -> Result<()> {
    use super::memo::Content;
    // As the frame has them, or differing only in how a content is named: a content the last run stored behind it
    // was named by its digest then and by its source now (or the other way), which the store alone could tell are
    // one (a read inside the waves). Not written again then, so a warm run writes nothing; the frame is only ever
    // compared for equality (a document named otherwise is hashed, not taken as unchanged), and the next run that
    // changes a document writes the names as they are.
    let named_alike = |a: &Content, b: &Content| a == b || matches!((a, b), (Content::Unstored(_), Content::Stored(_)) | (Content::Stored(_), Content::Unstored(_)));
    if docs.is_empty() || was.is_some_and(|was| was.len() == docs.len() && docs.iter().all(|(p, (c, k))| was.get(p).is_some_and(|(wc, wk)| wk == k && named_alike(wc, c)))) {
        return Ok(());
    }
    if std::env::var_os("RERAIL_STRATA_LIVE").is_some() {
        let differ: Vec<String> = docs.iter().filter(|(p, d)| was.and_then(|w| w.get(*p)) != Some(*d)).take(4).map(|(p, d)| format!("{p}: {:?} was {:?}", d, was.and_then(|w| w.get(p)))).collect();
        eprintln!("RERAIL_STRATA_LIVE run docs written: {} documents, {} before; first that differ: {differ:?}", docs.len(), was.map_or(0, |w| w.len()));
    }
    let mut paths: Vec<&String> = docs.keys().collect();
    paths.sort();
    let syms = store.intern_atoms(&paths.iter().map(|p| p.as_str()).collect::<Vec<_>>()).map_err(|e| Error::Plan(e.to_string()))?;
    let mut rows: Vec<i64> = Vec::with_capacity(paths.len() * 7);
    for (path, sym) in paths.iter().zip(syms) {
        let (content, key) = docs[*path];
        let (tag, words) = match content {
            Content::Stored(id) => (0, [id.raw(), 0, 0, 0]),
            Content::Unstored(d) => (1, digest_words(&d)),
            Content::Held(d) => (2, digest_words(&d)),
        };
        rows.extend([i64::from(sym), key, tag]);
        rows.extend(words);
    }
    let put = FramePut {
        rel: run_docs_rel(binds),
        part: DOCS_PART,
        digest: docs_tag(program),
        reads: Vec::new(),
        rows: FrameRows { types: b"siiiiii".to_vec(), rows, n: paths.len() },
    };
    store.queue_frames(space, vec![put], Vec::new(), None);
    Ok(())
}

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

/// The documents the last kept run of the part's program `binds` read (`write_run_docs`), if under the memo plan
/// `program`.
pub(in crate::code_rule_dsl) fn read_run_docs(store: &ContentStore, space: [u8; 32], binds: [u8; 32], program: &[u8; 32]) -> Result<Option<super::input_delta::Docs>> {
    use super::memo::Content;
    let rel = run_docs_rel(binds);
    let err = |e: crate::content_store::StoreError| Error::Plan(e.to_string());
    let stored = store.frames(&space, std::slice::from_ref(&rel), DOCS_PART).map_err(err)?;
    if stored.get(&rel).is_none_or(|s| s.digest != docs_tag(program)) {
        return Ok(None);
    }
    let admitted = store.admit_frames(&space, std::slice::from_ref(&rel), DOCS_PART).map_err(err)?;
    let Some(frame) = admitted.get(&rel) else { return Ok(None) };
    let rows = frame.rows().map_err(err)?;
    if rows.types != b"siiiiii" {
        return Ok(None);
    }
    let atoms = store.atoms_handle();
    let mut docs = super::input_delta::Docs::new();
    for row in rows.rows.chunks_exact(7) {
        let Some(path) = atoms.value(row[0] as crate::content_store::atoms::Sym).map_err(err)? else { return Ok(None) };
        let content = match row[2] {
            0 => Content::Stored(crate::content_store::SourceId::from_raw(row[3])),
            1 => Content::Unstored(words_digest(&row[3..7])),
            2 => Content::Held(words_digest(&row[3..7])),
            _ => return Ok(None),
        };
        docs.insert(path, (content, row[1]));
    }
    Ok(Some(docs))
}

/// An input's documents as its frame keeps them, decoded and checked where first asked (Q91): a run whose inputs
/// did not change reads only their digests. `None` where the frame does not decode or match its digest: the input
/// is then read as one with no frame.
pub(in crate::code_rule_dsl) struct FramedDocs {
    /// `None` for documents a run hashed itself (`ready`).
    frame: Option<Arc<crate::content_store::AdmittedFrame>>,
    digest: u64,
    /// A relation of no columns: its frame is its count (`write_frames`), one entry whose digest is not its rows'.
    counted: bool,
    docs: std::sync::OnceLock<Option<Arc<DocDigests>>>,
}

impl FramedDocs {
    /// Documents a run hashed itself, held as they are.
    fn ready(docs: Arc<DocDigests>) -> Arc<FramedDocs> {
        Arc::new(FramedDocs { frame: None, digest: 0, counted: false, docs: std::sync::OnceLock::from(Some(docs)) })
    }
    pub(in crate::code_rule_dsl) fn get(&self) -> Option<&Arc<DocDigests>> {
        self.docs
            .get_or_init(|| {
                let _span = crate::code_rule_dsl::logic::profile::span(|| "an input's documents, from its frame where first asked".into());
                let rows = self.frame.as_ref()?.rows().ok()?;
                if rows.types.len() != 3 || rows.rows.len() != rows.n * 3 {
                    return None;
                }
                let docs: DocDigests = rows.rows.chunks_exact(3).map(|r| (r[0], (r[1] as u64, r[2] as u64))).collect();
                (docs.len() == rows.n && (self.counted || digest_of_docs(&docs) == self.digest)).then(|| Arc::new(docs))
            })
            .as_ref()
    }
}

/// What an input's documents frame records beside its digest (an input's frame has no reads of its own): the
/// granules its rows lie in, a bit each (`granule_bit`).
const GRANULES_READ: &str = "#granules";

/// Granule `g`'s bit in `GRANULES_READ`'s mask (granules are `1..=GRANULES`).
fn granule_bit(g: i64) -> u64 {
    1u64 << ((g - 1) as u32 & 63)
}

/// The granules `GRANULES_READ`'s mask names, ascending.
fn granules_of(mask: u64) -> Vec<i64> {
    (1..=GRANULES).filter(|g| mask & granule_bit(*g) != 0).collect()
}
/// The part of a derived relation's frames that holds its rows' levels (decision 0090), one row each in its rows'
/// order, with their frame's digest: a new process maintains a recursive stratum by levels only if its kept
/// relations have them. Read where its digest and count match the rows'; without it, the rows have no levels. Not -1:
/// parts -1 to -k are a relation's delta frames (decision 0117 step 4), and a frame put whole removes them.
const LEVELS_PART: i64 = i64::MIN + 1;

/// `rel`'s kept rows `got` (its frame's part `key`) as a relation, with the levels `levels` holds for it when their
/// digest is its frame's and their count its rows'; else with none. `None`: the levels do not fit the rows.
fn with_levels(frames: &InFrames, levels: &HashMap<(FrameRel, i64), Arc<crate::content_store::AdmittedFrame>>, rel: &str, key: &(FrameRel, i64), got: FrameRows) -> Option<Arc<InRel>> {
    let level = levels.get(&(key.0.clone(), LEVELS_PART))
        .filter(|l| frames.derived.get(rel).is_some_and(|s| s.digest == l.stored.digest))
        .and_then(|l| l.rows().ok()).filter(|l| l.n == got.n && l.rows.len() == got.n)
        .map(|l| l.rows.iter().map(|&v| v as u32).collect::<Vec<u32>>());
    Some(match level {
        Some(level) => InRel::from_flat_levels(got.rows, got.n, engine_types(got.types), level)?,
        None => InRel::from_flat(got.rows, got.n, engine_types(got.types)),
    })
}

/// The deltas a relation's frame may have after it before it is written whole again (decision 0117 step 4;
/// `RERAIL_FRAME_DELTAS` sets it, 0 writes every changed relation whole).
fn max_deltas() -> u32 {
    std::env::var("RERAIL_FRAME_DELTAS").ok().and_then(|v| v.parse().ok()).unwrap_or(16)
}

/// What the store's frames hold of a cross-file part (t33 K4), read before the run's cut: the derived relations
/// stored and current, and each input's digest by document. Rows are read only for what the run then reads.
#[derive(Clone, Default)]
pub(in crate::code_rule_dsl) struct InFrames {
    space: [u8; 32],
    /// Each relation and input of the program, as its frame names it.
    names: HashMap<String, FrameRel>,
    /// Each derived relation stored and current: its digest and the digest each of its reads had.
    derived: HashMap<String, FrameStored>,
    /// Each input as stored: its digest, and its rows' digest by document (decoded where first asked, Q91).
    inputs: HashMap<String, (u64, Arc<FramedDocs>)>,
    /// The inputs whose owners are atoms (`atom_owners`, as their documents' table types them): their granules are
    /// by content (`granule`).
    atom_owned: std::collections::HashSet<String>,
    /// The relations admitted with their statistics part at their rows' digest (decision 0120 §3): written again
    /// only if their rows change.
    stats_kept: std::collections::HashSet<String>,
}

impl InFrames {
    /// The digest of `rel`'s stored frame, when it is stored and current.
    pub(in crate::code_rule_dsl) fn derived_digest(&self, rel: &str) -> Option<u64> {
        self.derived.get(rel).map(|s| s.digest)
    }
}

/// The digest of `read` a derived relation records (t33 K4): as `known` has it (an input as the run bound it, a
/// relation as derived or named), or, for the program's stated facts, their identity, which is the program's.
/// Anything else (an answer the run binds by another way, as a load's) has none: what reads it is not current.
fn read_digest(read: &str, known: &HashMap<String, u64>, derived: &BTreeSet<&String>, prog: &Placed, leaves: &crate::code_rule_dsl::logic::plan::Leaves<'_>) -> Option<u64> {
    if let Some(d) = known.get(read) {
        return Some(*d);
    }
    // A relation with literal rows that is also an input is named by its bound rows' digest, never by its literals.
    if derived.iter().any(|r| r.as_str() == read) || prog.fields().ok()?.is_input(read) {
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
fn kept_identities(store: &ContentStore, what: &str, key: [u8; 32], make: impl FnOnce() -> Result<Vec<(String, [u8; 32])>>) -> Result<Vec<(String, [u8; 32])>> {
    use super::preparation::parts::{hex, keep_facts, kept_facts_alone, rows_of, str};
    let kept = {
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("{what}: kept"));
        kept_facts_alone(store, &key).and_then(|f| rows_of(&f, "frame_ident").map(|r| Some((r.first()?.clone(), super::preparation::parts::unhex(r.get(1)?)?))).collect::<Option<Vec<_>>>())
    };
    if let Some(k) = kept {
        return Ok(k);
    }
    let made = make()?;
    let mut facts = crate::code_rule_dsl::logic::program_facts::Facts::new();
    facts.insert("frame_ident".to_owned(), made.iter().map(|(name, ident)| vec![str(name), str(&hex(ident))].into()).collect());
    keep_facts(store, &key, &facts);
    Ok(made)
}

/// A program's derived relations' identities (`plan::identities`, by its components), as `Retained::from_frames`
/// and `frame_rels` name them.
pub(in crate::code_rule_dsl) type StrataIdents = Arc<HashMap<String, [u8; 32]>>;

/// `p`'s strata identities (`identities_by_heads` over `sccs`), kept as a stored fact under the program and its
/// components. Read before the first wave only (decision 0104): a reader inside the waves is handed them.
fn strata_idents_kept(store: &ContentStore, p: &Placed, sccs: &[BTreeSet<String>]) -> Result<StrataIdents> {
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
    let made = kept_identities(store, "a program's strata identities", h.finalize().into(), || Ok(leaves.identities_by_heads(sccs)?.into_iter().collect()))?;
    Ok(Arc::new(made.into_iter().collect()))
}

/// `frame_rels`, its parts kept as stored facts (`kept_identities`): the whole set under the programs, `binds` and
/// `bound`, and each program's strata identities (`strata_idents_kept`), returned for `Retained::from_frames`.
/// Before the first wave (decision 0104).
pub(in crate::code_rule_dsl) fn frame_rels_kept(store: &ContentStore, progs: &[&Placed], binds: &[u8; 32], bound: &[&str]) -> Result<(std::collections::HashSet<FrameRel>, Vec<StrataIdents>)> {
    use sha2::{Digest as _, Sha256};
    let strata = progs.iter().map(|p| strata_idents_kept(store, p, &p.components()?)).collect::<Result<Vec<_>>>()?;
    let mut h = Sha256::new();
    h.update(b"rerail/frame rels v1\0");
    h.update(crate::code_rule_dsl::build_ids::engine().as_bytes());
    for p in progs {
        program_key(&mut h, p);
    }
    h.update(binds);
    let mut b: Vec<&str> = bound.to_vec();
    b.sort_unstable();
    b.dedup();
    for r in b {
        h.update(r.as_bytes());
        h.update([0]);
    }
    let rels = kept_identities(store, "the cross part's frame identities", h.finalize().into(), || {
        let mut out: std::collections::HashSet<FrameRel> = strata.iter().flat_map(|s| s.iter().map(|(name, ident)| FrameRel { name: name.clone(), ident: *ident })).collect();
        out.extend(input_rels(progs, binds, bound)?);
        Ok(out.into_iter().map(|f| (f.name, f.ident)).collect())
    })?;
    Ok((rels.into_iter().map(|(name, ident)| FrameRel { name, ident }).collect(), strata))
}

/// `frame_rels`' inputs: each declared relation of each of `progs`, and each of `bound`, as the part's input under
/// `binds`.
fn input_rels(progs: &[&Placed], binds: &[u8; 32], bound: &[&str]) -> Result<Vec<FrameRel>> {
    let _s = crate::code_rule_dsl::logic::profile::span(|| "frame identities: the inputs'".into());
    let mut out = Vec::new();
    for p in progs {
        let fields = p.fields()?;
        for rel in fields.decls_by_name().into_iter().map(|(rel, _)| rel).chain(bound.iter().copied()) {
            out.push(FrameRel { name: rel.to_owned(), ident: input_ident(rel, p, binds) });
        }
    }
    Ok(out)
}

/// The frames a run's cross part can read in its space (Q19, `ContentStore::preload_frames_live`): each derived
/// relation's identity in each of `progs` (`plan::identities`, by their components, as `Retained::from_frames`,
/// `kept_strata::load` and the planner's `Kept` name them), and each declared relation's as the part's input under
/// `binds` (`input_ident`, as `kept_inputs` and `from_frames` name them), and so each relation in `bound` (what the
/// run binds from an earlier phase, which `from_frames` reads as an input, declared or not). Not an earlier build's or
/// an earlier edit's identities, which a kept run's frames are under.
pub(in crate::code_rule_dsl) fn frame_rels(progs: &[&Placed], binds: &[u8; 32], bound: &[&str]) -> Result<std::collections::HashSet<FrameRel>> {
    let mut out = std::collections::HashSet::new();
    for p in progs {
        let sccs = {
            let _s = crate::code_rule_dsl::logic::profile::span(|| "frame identities: the components".into());
            p.components()?
        };
        let leaves = crate::code_rule_dsl::logic::plan::Leaves::of_placed((*p).clone());
        let _s = crate::code_rule_dsl::logic::profile::span(|| "frame identities: the strata's".into());
        out.extend(leaves.identities_by_heads(&sccs)?.into_iter().map(|(name, ident)| FrameRel { name, ident }));
    }
    out.extend(input_rels(progs, binds, bound)?);
    Ok(out)
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
pub(in crate::code_rule_dsl) type Locations = BTreeMap<String, Vec<usize>>;

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

/// Each owner's rows (by column `g`) in a session's bound input, as a range: a patched one's from its layers
/// (`Layered::ranges_now`), else found by one pass over the owner column, once a session (its next input is a
/// `Layered`). `None` when an owner's rows are not one run of rows.
pub(in crate::code_rule_dsl) fn session_ranges(source: &Arc<dyn Source>, g: usize) -> Option<Arc<Ranges>> {
    if let Some(l) = source.as_any().and_then(|a| a.downcast_ref::<Layered>()) {
        return Some(Arc::clone(l.ranges_now()));
    }
    let _s = crate::code_rule_dsl::logic::profile::span(|| "a session input's owners' ranges, found once".into());
    let mut out = Ranges::default();
    let (mut cur, mut start, mut ok) = (None::<i64>, 0usize, true);
    let mut next = 0usize;
    source.documents_of(g, &mut |r, d| {
        if !ok {
            return;
        }
        if r != next {
            ok = false;
            return;
        }
        next += 1;
        if cur != Some(d) {
            if let Some(c) = cur {
                out.insert(c, (start as u32, (r - start) as u32));
            }
            if out.contains_key(&d) {
                ok = false;
                return;
            }
            (cur, start) = (Some(d), r);
        }
    });
    if let Some(c) = cur.filter(|_| ok) {
        out.insert(c, (start as u32, (next - start) as u32));
    }
    (ok && next == source.len()).then(|| Arc::new(out))
}

impl KeptInput {
    /// A session's input as its last run bound it: its documents and ranges held already (`session_ranges`).
    pub(in crate::code_rule_dsl) fn held(source: Arc<dyn Source>, docs: Arc<DocDigests>, ranges: Arc<Ranges>) -> KeptInput {
        let digest = digest_of_docs(&docs);
        let laid = LaidOut { docs, ranges, owners: Vec::new() };
        let layout = InputLayout { rel: String::new(), frame: None, digest, atoms: None, granules: Vec::new(), laid: std::sync::OnceLock::from(Ok(laid)) };
        KeptInput { source, layout: Arc::new(layout), digest, stats: None }
    }
    /// An input whose documents and ranges are held already (a test's).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn laid(source: Arc<dyn Source>, docs: DocDigests, ranges: Ranges) -> KeptInput {
        let laid = LaidOut { docs: Arc::new(docs), ranges: Arc::new(ranges), owners: Vec::new() };
        let digest = digest_of_docs(&laid.docs);
        let layout = InputLayout { rel: String::new(), frame: None, digest, atoms: None, granules: Vec::new(), laid: std::sync::OnceLock::from(Ok(laid)) };
        KeptInput { source, layout: Arc::new(layout), digest, stats: None }
    }
    /// Its rows' digests by owner (its documents' frame, decoded and checked where first asked).
    pub(in crate::code_rule_dsl) fn docs(&self) -> std::result::Result<&Arc<DocDigests>, String> {
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
    frame: Option<Arc<crate::content_store::AdmittedFrame>>,
    digest: u64,
    atoms: Option<Arc<crate::content_store::atoms::Atoms>>,
    /// Each granule its rows lie in, and its rows' count, ascending.
    granules: Vec<(i64, usize)>,
    laid: std::sync::OnceLock<std::result::Result<LaidOut, String>>,
}

/// A kept input's documents, ranges and each granule's owners (`InputLayout::laid_out`).
pub(in crate::code_rule_dsl) struct LaidOut {
    docs: Arc<DocDigests>,
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
/// granule its rows lie in with its owners (the granule, then the owner, ascending), as `write_input_frames` laid
/// them out.
fn lay_out(rel: &str, frame: &crate::content_store::AdmittedFrame, digest: u64, atoms: &crate::content_store::atoms::Atoms) -> std::result::Result<(LaidOut, Vec<(i64, Vec<(i64, u32)>)>), String> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "a kept input's documents, laid out where first asked".into());
    let got = frame.rows().map_err(|_| format!("{rel}: its documents' frame does not decode"))?;
    if got.types.len() != 3 || got.rows.len() != got.n * 3 {
        return Err(format!("{rel}: its documents' frame is not three columns a row"));
    }
    let docs: DocDigests = got.rows.chunks_exact(3).map(|r| (r[0], (r[1] as u64, r[2] as u64))).collect();
    if docs.len() != got.n || digest_of_docs(&docs) != digest {
        return Err(format!("{rel}: its documents' frame does not match its digest"));
    }
    let mut keys: Vec<i64> = got.rows.chunks_exact(3).map(|r| r[0]).collect();
    if got.types.first() == Some(&b's') {
        atoms.content_keys(&mut keys);
    }
    let mut owners: Vec<(i64, i64, u64)> = got.rows.chunks_exact(3).zip(keys).map(|(r, k)| (1 + k.rem_euclid(GRANULES), r[0], r[2] as u64)).collect();
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

/// Whether the store's frames in `space` keep a frame of documents for every input `rels` of the cross part `prog`
/// (key `key`): `Err` (why) when one has none. Asked before the first wave, so that a run whose cross part kept no
/// inputs (its first, or one after a rule edit) decodes its memo hits whole as it finds them: the waves read no entry
/// of the store (decision 0104), and a hit read for its outputs alone has nothing else to take its rows from.
pub(in crate::code_rule_dsl) fn kept_inputs_stored(store: &ContentStore, space: [u8; 32], binds: [u8; 32], prog: &Placed, rels: &[String]) -> Result<std::result::Result<(), String>> {
    let all: Vec<FrameRel> = rels.iter().map(|rel| FrameRel { name: rel.clone(), ident: input_ident(rel, prog, &binds) }).collect();
    // Past the snapshot: an earlier run of the process (phase 0's, the sources phase's) installed it without this
    // root's cross space, which is read after this check (`preload_frames_live`). Through it, no input had a frame,
    // and every edit decoded its memo hits whole (Q151).
    let stored = store.frames_stored(&space, &all, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
    Ok(match all.iter().find(|f| !stored.contains_key(f)) {
        Some(f) => Err(format!("{}: no frame of its documents ({} of {} inputs have one)", f.name, stored.len(), all.len())),
        None => Ok(()),
    })
}

/// The inputs `rels` of the cross part `prog` (key `key`) as the store's frames in `space` keep them: read as
/// `Retained::load` reads a changed input, each owner's rows a range. `Err` (why) when one is not kept, or its frames
/// do not match their document table.
pub(in crate::code_rule_dsl) fn kept_inputs(store: &ContentStore, space: [u8; 32], binds: [u8; 32], prog: &Placed, locations: &Locations, rels: &[String], stats_of: &BTreeSet<String>) -> Result<std::result::Result<HashMap<String, KeptInput>, String>> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the kept inputs, from the frames".into());
    let names: Vec<(String, FrameRel)> = rels.iter().map(|rel| (rel.clone(), FrameRel { name: rel.clone(), ident: input_ident(rel, prog, &binds) })).collect();
    let all: Vec<FrameRel> = names.iter().map(|(_, f)| f.clone()).collect();
    let s_docs = crate::code_rule_dsl::logic::profile::span(|| "the documents' frames, admitted".into());
    let stored = store.frames(&space, &all, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
    if let Some(f) = all.iter().find(|f| !stored.contains_key(f)) {
        return Ok(Err(format!("{}: no frame of its documents ({} of {} inputs have one)", f.name, stored.len(), all.len())));
    }
    let admitted = store.admit_frames(&space, &all, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
    drop(s_docs);
    let atoms = store.atoms_handle();
    // Each input's granules: those its documents' frame records (`GRANULES_READ`), none decoded; or, for a frame
    // written before the keeper recorded them, its documents laid out now.
    let s_decode = crate::code_rule_dsl::logic::profile::span(|| "the documents' frames, decoded and checked".into());
    let mut planned: HashMap<String, (Arc<crate::content_store::AdmittedFrame>, Vec<i64>, Option<LaidOut>, usize)> = HashMap::new();
    for (rel, frame) in &names {
        let Some(width) = prog.fields()?.arity(rel) else { return Ok(Err(format!("{rel}: not declared by the cross part"))) };
        let Some(docs_frame) = admitted.get(frame) else { return Ok(Err(format!("{rel}: its documents' frame does not decode"))) };
        let at = &stored[frame];
        let recorded = at.reads.iter().find(|(r, _)| r == GRANULES_READ).map(|(_, mask)| *mask).filter(|_| width > 0);
        let (granules, laid) = match recorded {
            Some(mask) => (granules_of(mask), None),
            None => match lay_out(rel, docs_frame, at.digest, &atoms) {
                Ok((laid, granules)) => (granules.into_iter().map(|(g, _)| g).collect(), Some(laid)),
                Err(why) => return Ok(Err(why)),
            },
        };
        planned.insert(rel.clone(), (Arc::clone(docs_frame), granules, laid, width));
    }
    drop(s_decode);
    let s_granules = crate::code_rule_dsl::logic::profile::span(|| "the granules, admitted".into());
    let keys: Vec<(FrameRel, i64)> = names
        .iter()
        .flat_map(|(rel, frame)| planned[rel].1.iter().map(move |g| (frame.clone(), *g)))
        // An input's statistics, where the run is to read them kept (`stats_of`): a part of its frame, admitted
        // with its granules, so no read of its own.
        .chain(names.iter().filter(|(rel, _)| kept_stats::on() && stats_of.contains(rel)).map(|(_, frame)| (frame.clone(), kept_stats::PART)))
        .collect();
    let parts = store.admit_frame_parts(&space, &keys).map_err(|e| Error::Plan(e.to_string()))?;
    super::maintain::shape::frames_read(parts.len());
    drop(s_granules);
    let _s_sources = crate::code_rule_dsl::logic::profile::span(|| "the inputs' ranges and sources".into());
    let mut out = HashMap::new();
    for (rel, frame) in &names {
        let (docs_frame, ids, laid, width) = planned.remove(rel).expect("each input planned");
        let g = group_col(prog, locations, rel).min(width.max(1) - 1);
        let digest = stored[frame].digest;
        // Each granule's frame and its rows' count, read undecoded. The rows are decoded where the run first reads
        // the input (`Lazy`), and the documents where an owner is first asked (`InputLayout`): an edit reads few.
        let mut granules: Vec<Arc<crate::content_store::AdmittedFrame>> = Vec::with_capacity(ids.len());
        let mut counts: Vec<(i64, usize)> = Vec::with_capacity(ids.len());
        for (k, id) in ids.iter().enumerate() {
            let Some(part) = parts.get(&(frame.clone(), *id)) else { return Ok(Err(format!("{rel}: a granule's frame is missing"))) };
            let Some(n) = part.len().filter(|_| part.width() == Some(width)) else { return Ok(Err(format!("{rel}: a granule's frame does not hold its owners' rows"))) };
            if let Some(l) = &laid
                && l.owners[k].iter().map(|o| o.1 as usize).sum::<usize>() != n
            {
                return Ok(Err(format!("{rel}: a granule's frame does not hold its owners' rows")));
            }
            granules.push(Arc::clone(part));
            counts.push((*id, n));
        }
        let starts: Vec<usize> = counts.iter().scan(0usize, |at, (_, n)| { let s = *at; *at += n; Some(s) }).collect();
        let n: usize = counts.iter().map(|(_, n)| n).sum();
        let layout = Arc::new(InputLayout { rel: rel.clone(), frame: Some(docs_frame), digest, atoms: Some(Arc::clone(&atoms)), granules: counts, laid: laid.map(|l| std::sync::OnceLock::from(Ok(l))).unwrap_or_default() });
        let names: Vec<String> = prog.fields()?.columns(rel).unwrap_or_default().into_iter().map(|(n, _)| n.to_owned()).collect();
        let source: Arc<dyn Source> = Arc::new(Lazy { rel: rel.clone(), names, width, g, n, granules, layout: Arc::clone(&layout), starts: starts.clone(), parts: Default::default(), made: std::sync::OnceLock::new() });
        let source: Arc<dyn Source> = match std::env::var_os("RERAIL_KEPT_READS") {
            // Each granule's first row, in the order the rows were laid out.
            Some(_) => Arc::new(Traced::new(rel.clone(), source, starts)),
            None => source,
        };
        // Its statistics at these rows: the part's entry under the frame's digest (an older entry is never matched).
        let stats = parts
            .get(&(frame.clone(), kept_stats::PART))
            .filter(|p| p.stored.digest == digest)
            .and_then(|p| p.rows().ok())
            .and_then(|r| kept_stats::from_words(&r.rows))
            .and_then(|all| all.get(&(rel.clone(), digest)).map(|(columns, _, _)| columns.clone()));
        out.insert(rel.clone(), KeptInput { source, layout, digest, stats });
    }
    Ok(Ok(out))
}

/// A kept input as its frames hold it, decoded where the run first reads it (`kept_inputs`): its names and its rows'
/// count are known before. A granule whose rows are not its owners' is told through `failure`, its reader failing.
struct Lazy {
    rel: String,
    names: Vec<String>,
    width: usize,
    /// Its owner column.
    g: usize,
    n: usize,
    /// Each granule's frame, in its layout's order; its owners are its layout's (`InputLayout`).
    granules: Vec<Arc<crate::content_store::AdmittedFrame>>,
    layout: Arc<InputLayout>,
    /// Each granule's first row.
    starts: Vec<usize>,
    /// The granules decoded for rows read one at a time (`row_at`, `value`) before the input is read whole: an edit
    /// reads its evaluated owners' kept rows (`patch_owners`), a few owners' ranges of an input (Q70: 8,987 rows of
    /// 13 inputs decoded whole, 312 MB, on 300 files). Each granule's rows flat, by row; let go once `made` is.
    parts: parking_lot::Mutex<HashMap<usize, Arc<Vec<i64>>>>,
    /// Its rows, once read; with why not, read as none.
    made: std::sync::OnceLock<(Arc<dyn Source>, Option<String>)>,
}
impl Lazy {
    /// Granule `k`'s rows, flat, checked at each owner's range ends as `decode` checks them; `None` when they do not
    /// decode or are not by owner (read whole, `made` tells why).
    fn part(&self, k: usize) -> Option<Arc<Vec<i64>>> {
        if let Some(p) = self.parts.lock().get(&k) {
            return Some(Arc::clone(p));
        }
        let _span = crate::code_rule_dsl::logic::profile::span(|| "a kept input's granule, decoded where read".into());
        let part = &self.granules[k];
        let owners = &self.layout.laid_out().ok()?.owners[k];
        let width = self.width;
        let got = part.rows().ok()?;
        if got.types.len() != width || got.rows.len() != got.n * width {
            return None;
        }
        let mut start = 0usize;
        for &(owner, n) in owners {
            let n = n as usize;
            if width > 0 && n > 0 && [start, start + n - 1].iter().any(|r| owner_of(&got.rows[r * width..(r + 1) * width], self.g) != owner) {
                return None;
            }
            start += n;
        }
        let rows = Arc::new(got.rows);
        self.parts.lock().insert(k, Arc::clone(&rows));
        Some(rows)
    }
    /// Row `row`'s granule and its rows, while the input is not read whole.
    fn row_part(&self, row: usize) -> Option<(Arc<Vec<i64>>, usize)> {
        if self.made.get().is_some() || self.width == 0 {
            return None;
        }
        let k = self.starts.partition_point(|s| *s <= row).checked_sub(1)?;
        Some((self.part(k)?, row - self.starts[k]))
    }
    fn decode(&self) -> std::result::Result<Arc<dyn Source>, String> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "a kept input, decoded where read".into());
        let (rel, width) = (&self.rel, self.width);
        let mut columns: Vec<Vec<i64>> = (0..width).map(|_| Vec::with_capacity(self.n)).collect();
        let laid = self.layout.laid_out()?;
        for (part, owners) in self.granules.iter().zip(&laid.owners) {
            let got = part.rows().map_err(|e| format!("{rel}: a granule's frame does not decode: {e}"))?;
            if got.types.len() != width || got.rows.len() != got.n * width {
                return Err(format!("{rel}: a granule's frame does not hold its owners' rows"));
            }
            // Checked at each owner's range ends; an owner's rows are hashed again only when an edit changes them.
            let mut start = 0usize;
            for &(owner, n) in owners {
                let n = n as usize;
                if width > 0 && n > 0 && [start, start + n - 1].iter().any(|r| owner_of(&got.rows[r * width..(r + 1) * width], self.g) != owner) {
                    return Err(format!("{rel}: a granule's rows are not by owner"));
                }
                start += n;
            }
            for row in got.rows.chunks_exact(width.max(1)).take(got.n) {
                for (c, v) in row.iter().enumerate() {
                    columns[c].push(*v);
                }
            }
        }
        Ok(crate::code_rule_dsl::logic::native::component::columns_source(self.names.clone(), columns.into_iter().map(Some).collect()))
    }
    fn made(&self) -> &(Arc<dyn Source>, Option<String>) {
        let made = self.made.get_or_init(|| match self.decode() {
            Ok(s) => (s, None),
            Err(why) => (crate::code_rule_dsl::logic::native::component::columns_source(self.names.clone(), vec![Some(Vec::new()); self.width]), Some(why)),
        });
        if !self.parts.lock().is_empty() {
            self.parts.lock().clear();
        }
        made
    }
    fn inner(&self) -> &dyn Source {
        self.made().0.as_ref()
    }
}
impl Source for Lazy {
    fn columns(&self) -> Vec<String> {
        self.names.clone()
    }
    fn len(&self) -> usize {
        self.n
    }
    fn estimated_len(&self) -> usize {
        self.n
    }
    fn failure(&self) -> Option<String> {
        let (inner, why) = self.made();
        why.clone().or_else(|| inner.failure())
    }
    fn row_stream(&self, range: std::ops::Range<usize>, reverse: bool) -> Option<Box<dyn crate::code_rule_dsl::facts::vtab::RowStream>> {
        self.inner().row_stream(range, reverse)
    }
    fn direct_all(&self, equal: &[(usize, i64)]) -> Option<Vec<crate::code_rule_dsl::facts::vtab::ReadRow>> {
        self.inner().direct_all(equal)
    }
    fn direct_one(&self, column: usize, values: &[i64]) -> Option<Vec<crate::code_rule_dsl::facts::vtab::ReadRow>> {
        self.inner().direct_one(column, values)
    }
    fn caches_rows(&self) -> bool {
        self.inner().caches_rows()
    }
    fn row_cells(&self, row: usize) -> Option<(i64, Arc<dyn crate::code_rule_dsl::facts::vtab::RowCells>)> {
        self.inner().row_cells(row)
    }
    fn frozen(&self) -> Option<Arc<dyn Source>> {
        self.inner().frozen()
    }
    fn revision(&self) -> Option<u64> {
        self.inner().revision()
    }
    fn cell(&self, row: usize, column: usize) -> rusqlite::types::Value {
        self.inner().cell(row, column)
    }
    fn row_id(&self, row: usize) -> i64 {
        self.inner().row_id(row)
    }
    fn text_column(&self, column: usize) -> bool {
        self.inner().text_column(column)
    }
    fn row_at(&self, row: usize, columns: &[usize], out: &mut [i64]) {
        match self.row_part(row) {
            Some((rows, r)) => {
                for (o, &c) in out.iter_mut().zip(columns) {
                    *o = rows[r * self.width + c];
                }
            }
            None => self.inner().row_at(row, columns, out),
        }
    }
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.inner().estimated_matches(column, batch)
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        self.inner().estimated_matches_all(columns)
    }
    fn composite(&self) -> bool {
        self.inner().composite()
    }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        self.inner().lookup_all(equal)
    }
    fn doc_column(&self) -> Option<usize> {
        self.inner().doc_column()
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        self.inner().lookup_in(column, values, doc)
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.inner().kept_to_doc(column)
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        self.inner().filter_eq(eqs)
    }
    fn made_on_lookup(&self) -> bool {
        self.inner().made_on_lookup()
    }
    fn complete_rows(&self) -> bool {
        self.inner().complete_rows()
    }
    fn rows_owned(&self, g: usize, owned: &dyn Fn(i64) -> bool) -> Option<Vec<usize>> {
        self.inner().rows_owned(g, owned)
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        self.inner().expect_lookups(column, keys)
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.inner().statistics()
    }
    fn value_counts(&self, wanted: &[(usize, i64)]) -> Option<Vec<i64>> {
        self.inner().value_counts(wanted)
    }
    fn indexed(&self, column: usize) -> bool {
        self.inner().indexed(column)
    }
    fn range_upper_compatible(&self, lower: usize, upper: usize) -> bool {
        self.inner().range_upper_compatible(lower, upper)
    }
    fn range_indexed(&self, column: usize) -> bool {
        self.inner().range_indexed(column)
    }
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        self.inner().range_lookup(column, lower, upper)
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
        self.inner().lookup(column, values)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        match self.row_part(row) {
            Some((rows, r)) => rows[r * self.width + column],
            None => self.inner().value(row, column),
        }
    }
    fn sorted_rows(&self, column: usize, value: i64) -> Option<&[u32]> {
        self.inner().sorted_rows(column, value)
    }
    fn sorted_range(&self, column: usize, value: i64) -> Option<std::ops::Range<u32>> {
        self.inner().sorted_range(column, value)
    }
    fn sorts_rows(&self, column: usize) -> bool {
        self.inner().sorts_rows(column)
    }
    fn prepare_lookup(&self, columns: &[usize]) {
        self.inner().prepare_lookup(columns)
    }
    fn prepare_index(&self, columns: &[usize]) {
        self.inner().prepare_index(columns)
    }
    fn primary_key_len(&self) -> Option<usize> {
        self.inner().primary_key_len()
    }
    fn lookup_primary_batch(&self, values: &[Vec<i64>]) -> Option<Vec<usize>> {
        self.inner().lookup_primary_batch(values)
    }
    fn ordered(&self, equal: &[usize], order: &[(usize, bool)]) -> bool {
        self.inner().ordered(equal, order)
    }
    fn ordered_range(&self, equal: &[(usize, i64)]) -> Option<std::ops::Range<usize>> {
        self.inner().ordered_range(equal)
    }
}

/// `RERAIL_KEPT_READS`: a kept input that counts what the run reads of it, printed when it is dropped: the rows and
/// granules read one at a time, and the calls that read a whole column (a scan, an index, statistics).
struct Traced {
    rel: String,
    inner: Arc<dyn Source>,
    starts: Vec<usize>,
    rows: parking_lot::Mutex<rustc_hash::FxHashSet<usize>>,
    whole: parking_lot::Mutex<BTreeMap<&'static str, usize>>,
}
impl Traced {
    fn new(rel: String, inner: Arc<dyn Source>, starts: Vec<usize>) -> Self {
        Self { rel, inner, starts, rows: Default::default(), whole: Default::default() }
    }
    fn row(&self, r: usize) {
        let mut rows = self.rows.lock();
        if rows.is_empty() && std::env::var_os("RERAIL_KEPT_READS_STACK").is_some_and(|v| v == self.rel.as_str()) {
            eprintln!("RERAIL_KEPT_READS {}: first row read at\n{}", self.rel, std::backtrace::Backtrace::force_capture());
        }
        rows.insert(r);
    }
    fn rows_of(&self, got: &Option<Vec<usize>>) {
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
    fn estimated_matches(&self, column: usize, batch: bool) -> usize {
        self.inner.estimated_matches(column, batch)
    }
    fn estimated_matches_all(&self, columns: &[usize]) -> usize {
        self.inner.estimated_matches_all(columns)
    }
    fn composite(&self) -> bool {
        self.inner.composite()
    }
    fn lookup_all(&self, equal: &[(usize, i64)]) -> Option<Vec<usize>> {
        self.whole("lookup_all (index)");
        let got = self.inner.lookup_all(equal);
        self.rows_of(&got);
        got
    }
    fn doc_column(&self) -> Option<usize> {
        self.inner.doc_column()
    }
    fn lookup_in(&self, column: usize, values: &[i64], doc: i64) -> Option<Vec<usize>> {
        self.whole("lookup_in (index)");
        let got = self.inner.lookup_in(column, values, doc);
        self.rows_of(&got);
        got
    }
    fn kept_to_doc(&self, column: usize) -> bool {
        self.inner.kept_to_doc(column)
    }
    fn filter_eq(&self, eqs: &[(usize, i64)]) -> Vec<u32> {
        self.whole("filter_eq (scan)");
        self.inner.filter_eq(eqs)
    }
    fn made_on_lookup(&self) -> bool {
        self.inner.made_on_lookup()
    }
    fn complete_rows(&self) -> bool {
        self.inner.complete_rows()
    }
    fn rows_owned(&self, g: usize, owned: &dyn Fn(i64) -> bool) -> Option<Vec<usize>> {
        self.inner.rows_owned(g, owned)
    }
    fn expect_lookups(&self, column: usize, keys: f64) {
        self.inner.expect_lookups(column, keys)
    }
    fn statistics(&self) -> Option<Vec<(String, i64, i64, i64, i64)>> {
        self.whole("statistics");
        self.inner.statistics()
    }
    fn value_counts(&self, wanted: &[(usize, i64)]) -> Option<Vec<i64>> {
        self.whole("value_counts");
        self.inner.value_counts(wanted)
    }
    fn failure(&self) -> Option<String> {
        self.inner.failure()
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
    fn range_lookup(&self, column: usize, lower: Option<(i64, bool)>, upper: Option<(i64, bool)>) -> Option<Vec<usize>> {
        self.whole("range_lookup (index)");
        let got = self.inner.range_lookup(column, lower, upper);
        self.rows_of(&got);
        got
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<Vec<usize>> {
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
    fn lookup_primary_batch(&self, values: &[Vec<i64>]) -> Option<Vec<usize>> {
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
    pub(in crate::code_rule_dsl) fn from_frames(store: &ContentStore, space: [u8; 32], key: [u8; 32], binds: [u8; 32], prog: &Placed, sccs: &[BTreeSet<String>], bound: &HashMap<String, u64>) -> Result<Retained> {
        Self::from_frames_named(store, space, key, binds, prog, sccs, bound, None)
    }

    /// `from_frames`, the program's strata identities handed in where the run read them before the first wave
    /// (`frame_rels_kept`).
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn from_frames_named(store: &ContentStore, space: [u8; 32], key: [u8; 32], binds: [u8; 32], prog: &Placed, sccs: &[BTreeSet<String>], bound: &HashMap<String, u64>, strata: Option<&StrataIdents>) -> Result<Retained> {
        // The program's version, for its identities and its leaves' (`from_frame_snapshot`).
        let leaves = crate::code_rule_dsl::logic::plan::Leaves::of_placed(prog.clone());
        let idents: Vec<(String, [u8; 32])> = match strata {
            Some(held) => held.iter().map(|(name, ident)| (name.clone(), *ident)).collect(),
            None => {
                let _s = crate::code_rule_dsl::logic::profile::span(|| "the kept part's identities".into());
                leaves.identities_by_heads(sccs)?.into_iter().collect()
            }
        };
        let derived: BTreeSet<&String> = sccs.iter().flatten().collect();
        let mut names: HashMap<String, FrameRel> = idents.into_iter().map(|(name, ident)| (name.clone(), FrameRel { name, ident })).collect();
        // The inputs are what the run binds (`bound`, by their digests): the program's, and an earlier phase's export
        // as phase 1's `doc` or the types phase's `types__wgen`, which no rule may name (a pattern reads `node`).
        let read: BTreeSet<String> = prog.fields()?.inputs().map(str::to_owned).chain(bound.keys().cloned()).filter(|r| !derived.contains(r)).collect();
        for rel in &read {
            names.insert(rel.clone(), FrameRel { name: rel.clone(), ident: input_ident(rel, prog, &binds) });
        }
        let all: Vec<FrameRel> = names.values().cloned().collect();
        let stored = store.frames(&space, &all, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
        Self::from_frame_snapshot(store, space, key, prog, &leaves, &sccs, names, stored, bound)
    }

    fn empty_frames(key: [u8; 32], mut frames: InFrames) -> Self {
        frames.inputs.clear();
        frames.derived.clear();
        Self { key, docs: (Default::default(), [0xff; 32]), frames: Some(frames), ..Default::default() }
    }

    fn from_frame_snapshot(
        store: &ContentStore, space: [u8; 32], key: [u8; 32], prog: &Placed, leaves: &crate::code_rule_dsl::logic::plan::Leaves<'_>,
        sccs: &[BTreeSet<String>], names: HashMap<String, FrameRel>, stored: HashMap<FrameRel, FrameStored>, bound: &HashMap<String, u64>,
    ) -> Result<Self> {
        let derived: BTreeSet<&String> = sccs.iter().flatten().collect();
        let mut frames = InFrames { space, names, ..Default::default() };
        // The inputs are what the run binds (`bound`): the program's, and an earlier phase's export (`from_frames`).
        let f = prog.fields()?;
        let read: BTreeSet<&String> = f.inputs().filter_map(|r| frames.names.get_key_value(r).map(|(k, _)| k)).chain(bound.keys().filter(|r| frames.names.contains_key(*r))).filter(|r| !derived.contains(r)).collect();
        let asked: Vec<_> = read.iter().filter(|r| stored.contains_key(&frames.names[**r])).map(|r| frames.names[*r].clone()).collect();
        let admitted = store.admit_frames(&space, &asked, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
        if !super::kept_strata::admission_matches(&asked, &admitted, |r| stored.get(r)) {
            return Ok(Self::empty_frames(key, frames));
        }
        let mut inputs = HashMap::new();
        for frame in &asked {
            let at = &stored[frame];
            let docs_frame = &admitted[frame];
            // Its shape from the frame's header; its documents decoded where first asked (`FramedDocs`, Q91).
            let (Some(types), Some(n)) = (docs_frame.types(), docs_frame.len()) else { return Ok(Self::empty_frames(key, frames)); };
            if types.len() != 3 { return Ok(Self::empty_frames(key, frames)); }
            // A relation of no columns is its count (`write_frames`): one entry, its digest the input's.
            let counted = f.arity(&frame.name) == Some(0) && n == 1;
            if types.first() == Some(&b's') {
                frames.atom_owned.insert(frame.name.clone());
            }
            inputs.insert(frame.name.clone(), at.digest);
            frames.inputs.insert(frame.name.clone(), (at.digest, Arc::new(FramedDocs { frame: Some(Arc::clone(docs_frame)), digest: at.digest, counted, docs: std::sync::OnceLock::new() })));
        }
        // In stratum order: a relation is current when each of its reads is as it was when it was derived, in the
        // state the frames hold: an input as its frame has it (its old rows, which a changed stratum is maintained
        // from), or, with no frame, as the run binds it. A relation with no frame current is named by its reads as
        // `write_frames` names it, so a view of unchanged reads leaves its readers current.
        let mut known: HashMap<String, u64> = bound.clone();
        known.extend(frames.inputs.iter().map(|(rel, (d, _))| (rel.clone(), *d)));
        let all_reads = Reads::of(prog)?;
        // A frame a run found unchanged, under reads other than those it was stored with, is confirmed under them
        // (decision 0117 step 5): its reads are not written again, the space's confirmations name their hash.
        let confirmed = store.confirmed(&space).map_err(|e| Error::Plan(e.to_string()))?;
        for scc in sccs {
            // Each relation's frame as current, its reads as they are now (a confirmed one's stored reads are older).
            let current: Option<Vec<FrameStored>> = scc.iter().map(|rel| {
                let n = frames.names.get(rel)?;
                let at = stored.get(n)?;
                let now: Vec<(String, u64)> = at.reads.iter().map(|(read, _)| read_digest(read, &known, &derived, prog, &leaves).map(|d| (read.clone(), d))).collect::<Option<_>>()?;
                if now == at.reads {
                    return Some(at.clone());
                }
                (confirmed.get(&crate::content_store::frame_confirm_key(n, DOCS_PART)) == Some(&crate::content_store::frame_reads_hash(&now))).then(|| FrameStored { digest: at.digest, reads: now })
            }).collect();
            if let Some(current) = current {
                for (rel, at) in scc.iter().zip(current) {
                    known.insert(rel.to_string(), at.digest);
                    frames.derived.insert(rel.to_string(), at);
                }
            } else {
                let reads: Vec<String> = all_reads.of_scc(scc).into_iter().filter(|r| !scc.contains(r)).collect();
                if let Some(rd) = reads.iter().map(|r| read_digest(r, &known, &derived, prog, &leaves).map(|d| (r.clone(), d))).collect::<Option<Vec<_>>>() {
                    for rel in scc {
                        known.insert(rel.clone(), unkept_digest(rel, &rd));
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
    /// granules alone are read, and the rest are its new rows, read in place.
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
        let inputs: Vec<String> = cut.changed.iter().filter(|r| frames.inputs.contains_key(*r) && !self.input_srcs.contains_key(*r)).cloned().collect();
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
        let derived_keys: Vec<_> = wanted.iter().map(|r| (frames.names[r].clone(), DOCS_PART)).collect();
        let atoms = store.atoms_handle();
        let input_parts: Vec<_> = inputs.iter().map(|r| {
            let owners = frames.atom_owned.contains(r).then_some(&*atoms);
            let granules: BTreeSet<_> = frames.inputs[r].1.get().into_iter().flat_map(|docs| docs.keys()).filter(|d| changed_doc(r, d)).map(|d| granule(*d, owners)).collect();
            (r, granules.into_iter().map(|g| (frames.names[r].clone(), g)).collect::<Vec<_>>())
        }).collect();
        let mut keys = derived_keys.clone();
        keys.extend(input_parts.iter().flat_map(|(_, k)| k.iter().cloned()));
        // Each relation's statistics, a part of its frame (decision 0120 §3), admitted in the same batch.
        if kept_stats::on() {
            keys.extend(wanted.iter().map(|r| (frames.names[r].clone(), kept_stats::PART)));
        }
        // Their levels, where stored, in the same batch: not a key of the admission's check, as a relation may have
        // none.
        keys.extend(wanted.iter().map(|r| (frames.names[r].clone(), LEVELS_PART)));
        let admitting = crate::code_rule_dsl::logic::profile::span(|| "load: the frames' parts admitted".into());
        let admitted = store.admit_frame_parts(&frames.space, &keys).map_err(|e| Error::Plan(e.to_string()))?;
        drop(admitting);
        super::maintain::shape::frames_read(admitted.len());
        let levels = &admitted;
        // Prepare all rows before publishing any of them into the retained part. Failed admission is a cache
        // miss, and the caller must recompute its cut before maintenance can run.
        let decoded = (|| -> Option<_> {
            if !super::kept_strata::admission_matches(&derived_keys, &admitted, |(r, _)| frames.derived.get(&r.name)) { return None; }
            let mut rels = Vec::new();
            let mut sources = Vec::new();
            let mut rows = 0;
            let mut ranges_out: Vec<(String, Arc<dyn Source>, Arc<Ranges>)> = Vec::new();
            let _derived = crate::code_rule_dsl::logic::profile::span(|| "load: the derived relations' rows".into());
            for (rel, key) in wanted.iter().zip(&derived_keys) {
                let got = admitted.get(key)?.rows().ok()?;
                let width = f.arity(rel)?;
                if got.types.len() != width { return None; }
                rows += got.n;
                let rel_ = with_levels(frames, levels, rel, key, got)?;
                rels.push((rel.clone(), Kept { rel: rel_, width, output: None }));
            }
            drop(_derived);
            for (rel, keys) in &input_parts {
                let width = f.arity(rel)?;
                if let Some(p) = partial.get(*rel) {
                    let _s = crate::code_rule_dsl::logic::profile::span(|| "load: an input's changed documents' old rows".into());
                    let g = group_col(prog, locations, rel);
                    // The changed documents' rows, each document's together (its range), in the order first met.
                    let mut order: Vec<i64> = Vec::new();
                    let mut by: rustc_hash::FxHashMap<i64, Vec<i64>> = rustc_hash::FxHashMap::default();
                    let mut types: Option<Vec<u8>> = None;
                    let same = news?.same;
                    let (mut decoding, mut read) = (std::time::Duration::ZERO, 0usize);
                    for key in keys {
                        let t = std::time::Instant::now();
                        let got = admitted.get(key)?.rows().ok()?;
                        decoding += t.elapsed();
                        read += got.n;
                        if got.types.len() != width || got.rows.len() != got.n * width {
                            return None;
                        }
                        types.get_or_insert_with(|| got.types.clone());
                        for row in got.rows.chunks_exact(width).take(got.n) {
                            let d = owner_of(row, g);
                            if same.contains(&d) {
                                continue;
                            }
                            by.entry(d).or_insert_with(|| {
                                order.push(d);
                                Vec::new()
                            }).extend_from_slice(row);
                        }
                    }
                    let mut columns: Vec<Vec<i64>> = vec![Vec::new(); width];
                    let mut ranges = Ranges::default();
                    for d in &order {
                        let flat = &by[d];
                        let at = p.kept + columns[0].len();
                        ranges.insert(*d, (at as u32, (flat.len() / width) as u32));
                        for row in flat.chunks_exact(width) {
                            for (c, v) in row.iter().enumerate() {
                                columns[c].push(*v);
                            }
                        }
                    }
                    let names: Vec<String> = f.columns(rel)?.into_iter().map(|(n, _)| n.to_owned()).collect();
                    let changed = crate::code_rule_dsl::logic::native::component::columns_source(names.clone(), columns.into_iter().map(Some).collect());
                    // The changed documents' rows checked against their digests, as a whole input's are.
                    let cells = types.as_deref().map_or(Cells::plain(width), |t| Cells::new(&atoms, t));
                    let was = frames.inputs[*rel].1.get()?;
                    let expected: DocDigests = was.iter().filter(|(d, _)| !same.contains(d)).map(|(d, w)| (*d, *w)).collect();
                    if std::env::var_os("RERAIL_LOAD_TRACE").is_some() {
                        eprintln!("RERAIL_LOAD_TRACE {rel}: {} granules, {read} rows decoded in {:.3} s, {} rows of {} changed documents kept", keys.len(), decoding.as_secs_f64(), changed.len(), order.len());
                    }
                    if docs_of(changed.as_ref(), g, &cells) != expected {
                        return None;
                    }
                    rows += changed.len();
                    let source: Arc<dyn Source> = Arc::new(OldInput { names, new: Arc::clone(&p.new), runs: p.runs.clone(), kept: p.kept, changed, idx: (0..width).map(|_| std::sync::OnceLock::new()).collect() });
                    ranges_out.push(((*rel).clone(), Arc::clone(&source), Arc::new(ranges)));
                    sources.push(((*rel).clone(), source));
                    continue;
                }
                let copying = crate::code_rule_dsl::logic::profile::span(|| "load: an input's rows into columns".into());
                let mut columns: Vec<Vec<i64>> = vec![Vec::new(); width];
                let mut types: Option<Vec<u8>> = None;
                for key in keys {
                    let got = admitted.get(key)?.rows().ok()?;
                    if got.types.len() != width || got.rows.len() != got.n * width { return None; }
                    types.get_or_insert_with(|| got.types.clone());
                    for row in got.rows.chunks_exact(width.max(1)).take(got.n) {
                        for (c, v) in row.iter().enumerate() { columns[c].push(*v); }
                    }
                }
                // Named as the program declares them, as the run's own rows are: the session's input delta by
                // document (`document_delta`) takes an old and a new source of the same columns, and with other names
                // every input fell back to the diff by rules (`r__old, !r`), a scan per row (192 s on 20 files).
                let names = f.columns(rel)?.into_iter().map(|(n, _)| n.to_owned()).collect();
                let source = crate::code_rule_dsl::logic::native::component::columns_source(names, columns.into_iter().map(Some).collect());
                // Granule headers carry no digest. Their rows must match the admitted per-document digests,
                // including documents removed or replaced between the header and payload reads.
                drop(copying);
                super::maintain::shape::input_read_whole();
                let _checking = crate::code_rule_dsl::logic::profile::span(|| "load: an input's rows checked against its documents' digests".into());
                let cells = types.as_deref().map_or(Cells::plain(width), |t| Cells::new(&atoms, t));
                if Some(&docs_of(source.as_ref(), group_col(prog, locations, rel), &cells)) != frames.inputs[*rel].1.get().map(|d| &**d) { return None; }
                rows += source.len();
                sources.push(((*rel).clone(), source));
            }
            Some((rels, sources, rows, ranges_out))
        })();
        let Some((rels, sources, rows, ranges_out)) = decoded else {
            let frames = self.frames.take().expect("admitting frames");
            *self = Self::empty_frames(self.key, frames);
            crate::code_rule_dsl::logic::profile::decision("fact admission", || "frames changed during admission; recompute the cut and derive again".into());
            return Ok(None);
        };
        super::maintain::shape::rows_decoded(rows);
        let count = rels.len() + sources.len();
        // A relation whose statistics part was stored for the rows it has now takes them, under the address it has
        // in this process: it is not counted or measured again (decision 0120 §3).
        if kept_stats::on() {
            let mut measured = 0;
            // `RERAIL_KEPT_STATS_TRACE`: why a relation admitted took no statistics.
            let (mut no_part, mut other_digest, mut undecoded) = (0, 0, 0);
            for (rel, k) in &rels {
                let Some(d) = frames.derived.get(rel).map(|s| s.digest) else { continue };
                let Some(part) = admitted.get(&(frames.names[rel].clone(), kept_stats::PART)) else { no_part += 1; continue };
                if part.stored.digest != d {
                    other_digest += 1;
                    continue;
                }
                let Some(all) = part.rows().ok().and_then(|r| kept_stats::from_words(&r.rows)) else { undecoded += 1; continue };
                let at = Arc::as_ptr(&k.rel) as *const u8 as usize;
                for ((name, _), m) in all {
                    measured += 1;
                    self.measured.insert((name, at), m);
                }
            }
            if std::env::var_os("RERAIL_KEPT_STATS_TRACE").is_some() {
                eprintln!("rerail: kept statistics: {} admitted, {measured} measured; no part {no_part}, at another digest {other_digest}, not decoded {undecoded}", rels.len());
            }
            if let Some(f) = self.frames.as_mut() {
                f.stats_kept.extend(rels.iter().filter(|(_, k)| {
                    let at = Arc::as_ptr(&k.rel) as *const u8 as usize;
                    self.measured.keys().any(|(_, a)| *a == at)
                }).map(|(rel, _)| rel.clone()));
            }
            crate::code_rule_dsl::logic::profile::decision("kept statistics from the frames (decision 0120 §3)", || {
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
    pub(in crate::code_rule_dsl) fn take_frames(&mut self) -> Option<InFrames> {
        self.frames.take()
    }

    /// `frames` carried into this part, which `cut` made: a relation it derived again is held only as derived (its
    /// stored rows are of the reads before), the others as stored.
    pub(in crate::code_rule_dsl) fn with_frames(&mut self, mut frames: InFrames, cut: &Cut) {
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
    pub(in crate::code_rule_dsl) fn read_input(&self, store: &ContentStore, prog: &Placed, locations: &Locations, rel: &str, docs: Option<&dyn Fn(i64) -> bool>) -> Result<std::result::Result<Option<Vec<Vec<i64>>>, String>> {
        let width = prog.fields()?.arity(rel).unwrap_or(0);
        let g = group_col(prog, locations, rel);
        let take = |row: &[i64]| docs.is_none_or(|d| d(owner_of(row, g)));
        if let Some(src) = self.input_srcs.get(rel) {
            let all: Vec<usize> = (0..width).collect();
            let mut out = Vec::new();
            let mut row = vec![0i64; width];
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
            return Ok(Ok(Some(vec![Vec::new(); by_doc.values().map(|(_, n)| *n as usize).sum()])));
        }
        let atoms = store.atoms_handle();
        let owners = frames.atom_owned.contains(rel).then_some(&*atoms);
        let granules: BTreeSet<i64> = by_doc.keys().filter(|d| docs.is_none_or(|f| f(**d))).map(|d| granule(*d, owners)).collect();
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("{rel}: {} granules, from the frames", granules.len()));
        let mut out = Vec::new();
        for g in granules {
            let Some(got) = store.frame_rows(&frames.space, &frames.names[rel], g).map_err(|e| Error::Plan(e.to_string()))? else {
                return Ok(Err(format!("{rel}: granule {g} of its rows is not stored")));
            };
            out.extend(got.rows.chunks_exact(width.max(1)).take(got.n).filter(|r| take(r)).map(<[i64]>::to_vec));
        }
        Ok(Ok(Some(out)))
    }

    /// Derived relation `rel`'s rows, as held or read back from the frames.
    pub(in crate::code_rule_dsl) fn read_derived(&self, store: &ContentStore, rel: &str) -> Result<Option<Vec<Vec<i64>>>> {
        if let Some(k) = self.rels.get(rel) {
            let Some(flat) = k.rel.live_rows() else { return Ok(None) };
            return Ok(Some(flat.chunks_exact(k.width.max(1)).map(<[i64]>::to_vec).collect()));
        }
        let Some(frames) = self.frames.as_ref().filter(|f| f.derived.contains_key(rel)) else { return Ok(None) };
        let Some(got) = store.frame_rows(&frames.space, &frames.names[rel], DOCS_PART).map_err(|e| Error::Plan(e.to_string()))? else { return Ok(None) };
        let width = got.types.len();
        Ok(Some(got.rows.chunks_exact(width.max(1)).take(got.n).map(<[i64]>::to_vec).collect()))
    }

    /// What the frames held of this part before the run (`from_frames`).
    pub(in crate::code_rule_dsl) fn frames(&self) -> Option<&InFrames> {
        self.frames.as_ref()
    }

    /// Writes to the frames what this run changed of the part (t33 K4), behind the run (the store's writer): each
    /// relation it derived whose rows or reads differ from its frame, and each input's granules whose documents'
    /// rows changed, with the input's digests by document. The relations kept as they were stay as stored.
    /// `sccs`: `prog`'s strata, computed once by the caller.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::code_rule_dsl) fn write_frames(&self, store: &ContentStore, was: &InFrames, prog: &Placed, domains: &lower::Domains, locations: &Locations, sccs: &[BTreeSet<String>], cut: &Cut, inputs: &[(String, Arc<dyn Source>)], digests: &HashMap<String, u64>, unchanged: &BTreeSet<String>, deltas: &HashMap<String, crate::code_rule_dsl::logic::plan::Delta>, mut handed: HashMap<String, Handed>, inputs_done: Option<InputsWritten>, input_stats: &HashMap<String, Vec<(String, i64, i64, i64, i64)>>, placer: Option<crate::content_store::FramePlacer>) -> Result<usize> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the kept part's changes, to the frames".into());
        let leaves = crate::code_rule_dsl::logic::plan::Leaves::of_placed(prog.clone());
        // Literal-only leaves are part of the derived relation's identity (leaf_identity), not runtime
        // inputs requiring a frame digest. Without this, a read of gproto("Generic") or an env fact makes
        // publication skip its consumer and forces that consumer to run in every new process.
        let f = prog.fields()?;
        let produced: BTreeSet<&str> = f.answers().iter().map(|(_, p)| f.rel_name(*p)).collect();
        let dynamic: BTreeSet<&str> = f.inputs().chain(sccs.iter().flatten().map(String::as_str)).chain(f.natives().iter().map(|n| f.rel_name(n.rel)))
            .chain(inputs.iter().map(|(r,_)|r.as_str())).chain(produced.iter().copied()).collect();
        let literals: BTreeSet<&str> = f.fact_rels().into_iter().map(|r| f.text(r.name())).filter(|r| !dynamic.contains(r)).collect();
        // Each frame encoded as it is made: the rows copied out of a relation are held only until then.
        let mut puts: Vec<EncodedFrame> = Vec::new();
        let atoms = store.atoms_handle();
        // Inputs first: their digests are what the relations derived from them record (written before the run when it
        // lets its inputs go, `inputs_done`).
        let InputsWritten { mut now, mut written, queued: inputs_queued } = match inputs_done {
            Some(done) => done,
            None => {
                let held = &self.input_docs;
                write_input_frames(store, was, prog, domains, locations, inputs, digests, &|rel: &str| held.get(rel).and_then(|d| d.get()).cloned(), input_stats)
            }
        };
        // Every input the run bound, framed or not (an option's `env`, an earlier phase's export): its digest is
        // what a relation derived from it records.
        for (rel, d) in digests {
            now.entry(rel.clone()).or_insert(*d);
        }
        let derived: BTreeSet<&String> = sccs.iter().flatten().collect();
        // Each derived relation's frame is encoded straight into the write's segments as its rows are read (Q89):
        // neither a copy of the rows nor the frame's bytes are made on the heap. Copied and encoded on the heap, the
        // frames were held whole until queued, and the cross part's were a cold PostHog run's peak (1.8 → 2.3 GB in
        // its last second). The run's own (`placer`) when it handed relations off into it as it went; the inputs'
        // frames have placers of their own, one an input's job (`write_input_frames`).
        let placer = placer.unwrap_or_else(|| {
            let bytes: usize = derived.iter().filter_map(|rel| self.framed(rel)).map(|k| k.rel.rows() * k.width * 2).sum();
            store.frame_placer(derived.len() * 2 + 16, bytes)
        });
        let all_reads = Reads::of(prog)?;
        let trace = std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some();
        let mut tally = [(0usize, 0usize); 2];
        let mut retags: Vec<crate::content_store::FrameRetag> = Vec::new();
        let mut delta_out: Vec<crate::content_store::FrameDelta> = Vec::new();
        let mut delta_rows = 0usize;
        // The space's kept facts as blocks (decision 0401 F6), put beside each frame until the frames are retired; the
        // frames with no entry (a relation handed off, its rows dropped) counted.
        let mut kept: Vec<KeptPut> = Vec::new();
        let mut unkept = 0usize;
        // The relations written whole, each one's rows copied, digested and (when its rows are not the stored
        // ones) encoded in a job of its own (MLDB's per-item `parallelMap`): the loop below decides in its order,
        // with each one's digest at hand. A relation not done here (a delta written after its frame, whose reads
        // come out unknown) is done by the loop as before.
        let whole: Vec<(&String, &Kept)> = derived.iter().copied()
            .filter(|rel| was.names.contains_key(*rel) && !cut.clean.contains(*rel) && !(unchanged.contains(*rel) && was.derived.contains_key(*rel)))
            .filter(|rel| !self.by_delta(store, was, rel, deltas))
            .filter_map(|rel| self.framed(rel).map(|k| (rel, k)))
            .collect();
        // The copy of a relation's rows lives only in its job: kept for every relation until the loop below, the
        // copies were the run's peak (460 MB of 3.3 GB at 300 files, beside the relations and their bytes). A
        // relation whose rows are the stored ones and whose reads changed copies its rows again there.
        let made: HashMap<&String, Whole> = {
            use rayon::prelude::*;
            let atoms = &atoms;
            let placer = &placer;
            // Under the span open here, on whichever thread runs it: rayon's loops open none of their own.
            let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
            whole.into_par_iter().filter_map(|(rel, k)| {
                let _span = crate::code_rule_dsl::logic::profile::span_at(carry.under(), || "a relation's frame, digested and encoded".into());
                if k.rel.levels_shape().2 {
                    return None;
                }
                let types = frame_types(prog, domains, rel, k.rel.types());
                let d = held_digest(&Cells::new(atoms, &types), &k.rel)?;
                // Encoded straight into the write's segments as its rows are read where they lie (Q89).
                let bytes = was.derived.get(rel).is_none_or(|s| s.digest != d).then(|| held_placed(&k.rel, types.len(), placer));
                Some((rel, Whole { d, bytes }))
            }).collect()
        };
        let mut made = made;
        for scc in sccs {
            let reads: Vec<String> = all_reads.of_scc(scc).into_iter().filter(|r| !scc.contains(r) && !literals.contains(r.as_str())).collect();
            for rel in scc {
                let Some(name) = was.names.get(rel) else { continue };
                if cut.clean.contains(rel) {
                    if let Some(stored) = was.derived.get(rel) {
                        now.insert(rel.clone(), stored.digest);
                    }
                    continue;
                }
                let read_digests = reads.iter().map(|r| read_digest(r, &now, &derived, prog, &leaves).map(|d| (r.clone(), d))).collect::<Option<Vec<_>>>();
                // Its rows are as they were (decision 0117 step 4): its digest is the stored one, and its rows are
                // neither read nor written; its reads are told again if they changed.
                if unchanged.contains(rel)
                    && let Some(stored) = was.derived.get(rel)
                {
                    now.insert(rel.clone(), stored.digest);
                    match read_digests {
                        Some(rd) if rd != stored.reads => {
                            kept.push(kept_reads(name, DOCS_PART, rd.clone()));
                            retags.push(crate::content_store::FrameRetag { rel: name.clone(), part: DOCS_PART, reads: rd });
                        }
                        _ => {}
                    }
                    continue;
                }
                // Changed by a delta (decision 0117 step 4): the delta is written after its frame, and its digest is
                // patched by it, while its frame's deltas are few against its rows; else it is written whole. A
                // relation with levels (decision 0090) is written whole: its levels part is one row each in its rows'
                // order under its rows' digest, which a delta moves, and without it the next process has no levels.
                if let (Some(delta), Some(stored), Some(rd)) = (deltas.get(rel), was.derived.get(rel), read_digests.as_ref())
                    && let Some(k) = self.framed(rel)
                    && self.by_delta(store, was, rel, deltas)
                    && let Some((gone, added)) = delta.rows(&k.rel)
                {
                    let (ng, na) = (gone.len() / k.width, added.len() / k.width);
                    let n_now = k.rel.rows();
                    {
                        let n_was = (n_now + ng).saturating_sub(na);
                        let cells = Cells::new(&atoms, &frame_types(prog, domains, rel, k.rel.types()));
                        let d = crate::code_rule_dsl::logic::plan::rows_digest_patched(&cells, stored.digest, n_was, &gone, &added);
                        now.insert(rel.clone(), d);
                        written += ng + na;
                        delta_rows += ng + na;
                        match kept_rows(name, d, rd.clone(), frame_types(prog, domains, rel, k.rel.types()), &k.rel) {
                            Some(p) => kept.push(p),
                            None => unkept += 1,
                        }
                        delta_out.push(crate::content_store::FrameDelta { rel: name.clone(), digest: d, reads: rd.clone(), types: frame_types(prog, domains, rel, k.rel.types()), gone: gone.clone(), added: added.clone() });
                        continue;
                    }
                }
                // Handed off as the run went (`Handed`): its frame made then, in this order as any other's.
                if let Some(h) = handed.remove(rel) {
                    now.insert(rel.clone(), h.d);
                    let Some(read_digests) = read_digests else { continue };
                    if was.derived.get(rel).is_some_and(|s| s.digest == h.d && s.reads == read_digests) {
                        continue;
                    }
                    // Its rows are the stored ones: only its reads changed.
                    let Some(bytes) = h.bytes else {
                        kept.push(kept_reads(name, DOCS_PART, read_digests.clone()));
                        retags.push(crate::content_store::FrameRetag { rel: name.clone(), part: DOCS_PART, reads: read_digests });
                        continue;
                    };
                    // Its rows were dropped when it was handed off: no entry yet (0401 step 7 keeps it at the hand-off).
                    unkept += 1;
                    written += h.n;
                    if let Some(level) = h.levels {
                        puts.push(FramePut { rel: name.clone(), part: LEVELS_PART, digest: h.d, reads: Vec::new(), rows: FrameRows { types: vec![b'i'], rows: level.into_iter().map(i64::from).collect(), n: h.n } }.encoded());
                    }
                    puts.push(derived_frame(name, h.d, read_digests, h.types, h.n, bytes));
                    continue;
                }
                // A relation with no rows to keep (a view) is named by its reads' digests:
                // derived again from the same reads, it has the same rows, so its readers' frames stay current.
                let done = made.remove(rel);
                let Some((k, mut rows, d, bytes)) = (match done {
                    Some(Whole { d, bytes }) => self.framed(rel).map(|k| (k, None, d, bytes)),
                    None => self.framed(rel).and_then(|k| held_digest(&Cells::new(&atoms, &frame_types(prog, domains, rel, k.rel.types())), &k.rel).map(|d| (k, None, d, None))),
                }) else {
                    if let Some(rd) = &read_digests {
                        now.insert(rel.clone(), unkept_digest(rel, rd));
                    }
                    continue;
                };
                let n = k.rel.rows();
                now.insert(rel.clone(), d);
                let Some(read_digests) = read_digests else { continue };
                if was.derived.get(rel).is_some_and(|s| s.digest == d && s.reads == read_digests) {
                    continue;
                }
                // The frame's bytes, read from the rows where they lie into the write's segments; the rows copied only
                // to trace them.
                let bytes = bytes.or_else(|| Some(held_placed(&k.rel, k.rel.types().len(), &placer)));
                if trace {
                    rows = k.rel.live_rows();
                }
                if trace && let Some(rows) = &rows {
                    let i = usize::from(was.derived.get(rel).is_some_and(|s| s.digest == d));
                    tally[i].0 += 1;
                    tally[i].1 += n;
                    // Which relation, and its least rows: two runs' lines show the column whose values moved.
                    let mut least: Vec<&[i64]> = rows.chunks(k.width.max(1)).take(n).collect();
                    least.sort_unstable();
                    least.truncate(3);
                    eprintln!(
                        "rerail: derived frame written ({}): {rel}, {n} rows, types {}, least rows {least:?}",
                        if i == 0 { "rows changed" } else { "reads changed" },
                        String::from_utf8_lossy(&frame_types(prog, domains, rel, k.rel.types()))
                    );
                }
                written += n;
                if let Some(level) = k.rel.live_levels() {
                    puts.push(FramePut { rel: name.clone(), part: LEVELS_PART, digest: d, reads: Vec::new(), rows: FrameRows { types: vec![b'i'], rows: level.into_iter().map(i64::from).collect(), n } }.encoded());
                }
                let types = frame_types(prog, domains, rel, k.rel.types());
                match kept_rows(name, d, read_digests.clone(), types.clone(), &k.rel) {
                    Some(p) => kept.push(p),
                    None => unkept += 1,
                }
                puts.push(match (bytes, rows) {
                    (Some(bytes), _) => derived_frame(name, d, read_digests, types, n, bytes),
                    (None, Some(rows)) => FramePut { rel: name.clone(), part: DOCS_PART, digest: d, reads: read_digests, rows: FrameRows { types, rows, n } }.encoded(),
                    (None, None) => return Err(Error::Plan(format!("{rel}: its rows are not held, and its frame is to be written"))),
                });
            }
        }
        if trace {
            eprintln!("rerail: derived frames written: {} relations ({} rows) whose rows changed; {} ({} rows) whose rows are the same and whose reads changed; {} unchanged retagged; {} deltas ({delta_rows} rows) written after their frames", tally[0].0, tally[0].1, tally[1].0, tally[1].1, retags.len(), delta_out.len());
        }
        if kept_stats::on() {
            self.stats_puts(was, &now, &mut puts, &mut kept);
        }
        // Behind the run (decision 0104): the part's fixpoint is reached, and these are its results for the next run;
        // a wait for the writer's room here holds no wave. Each frame was encoded as it was made (`FramePut::encoded`).
        // Nothing changed: nothing written (decision 0227; the program's relations are kept by reachability). A
        // program that reuses an earlier one's frames (an edit, decision 0302) holds them, once: the earlier one
        // superseded, nothing else keeps them.
        if puts.is_empty() && retags.is_empty() && delta_out.is_empty() && !inputs_queued {
            store.hold_frames(was.space, was.names.values().cloned().collect(), super::run_token(), self.key);
            return Ok(written);
        }
        // Mapped (decision 0212): every atom a frame holds is in an `s` column (a text-owned input's owners too, in its
        // documents' table), its digests and granules are of content, so a forked process's frames are written as the
        // store numbers them, and read back as the reader does. Keyed by this process's numbering, they were dropped
        // after a fork, and every next run derived the cross part again.
        crate::code_rule_dsl::logic::profile::behind(|| store.queue_placed_retagged(was.space, puts, retags, delta_out, Vec::new(), Some((was.names.values().cloned().collect(), super::run_token(), self.key)), placer));
        crate::code_rule_dsl::logic::profile::decision("the kept facts as blocks, beside the frames (decision 0401 step 6)", || format!("this program: {} entries put; {unkept} frames with no entry (relations handed off, their rows dropped)", kept.len()));
        crate::code_rule_dsl::logic::profile::behind(|| store.queue_kept(was.space, kept));
        Ok(written)
    }
}

/// What `write_input_frames` wrote: each input's digest (what the relations derived from it record), the rows
/// written, and whether any input's frames were queued.
pub(in crate::code_rule_dsl) struct InputsWritten {
    now: HashMap<String, u64>,
    written: usize,
    queued: bool,
}

/// The inputs' part of `Retained::write_frames`: each input's granules whose documents' rows changed, and its digests
/// by document, queued as each input's job makes them. They depend on the inputs alone, so a run that lets its inputs
/// go after their last reader (Q74) writes them before it runs, and hands what it wrote to `write_frames`.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn write_input_frames(store: &ContentStore, was: &InFrames, prog: &Placed, domains: &lower::Domains, locations: &Locations, inputs: &[(String, Arc<dyn Source>)], digests: &HashMap<String, u64>, input_docs: &(dyn Fn(&str) -> Option<Arc<DocDigests>> + Sync), input_stats: &HashMap<String, Vec<(String, i64, i64, i64, i64)>>) -> InputsWritten {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the inputs' frames".into());
    let mut now: HashMap<String, u64> = HashMap::new();
    let mut written = 0;
    // Digests of content and granules by content (decision 0212): what every process reads back alike.
    let atoms = store.atoms_handle();
    // Inputs first: their digests are what the relations derived from them record. Each input is a job of its
    // own (its rows' digests by document, its changed granules' rows, each encoded); their frames are taken in
    // the inputs' order.
    let phase = super::phases::current();
    let atoms_ref = &atoms;
    // Each input's frames are queued as its job makes them (Q74), into the run's wave (`Carry`: the writer takes
    // the wave whole, in one transaction, with the derived relations' frames when they are written together; before
    // the run, in the wave it seals as it ends, `Boundary::CrossDerived`): placed in segments of
    // their own (decision 0295 §1), their bytes leave the run's heap then. Collected to be queued at the end, every
    // input's encoded frames were held at once beside the inputs' rows (270 MB at a cold run's peak, 300 files).
    let carry = crate::code_rule_dsl::logic::run_state::Carry::here();
    let carry = &carry;
    // One placer for every input's frames: each job places its granules' frames in it as it encodes them, and the
    // frames are queued together once the jobs end, in the inputs' order. A placer per input made a segment file per
    // input (each leased, mapped, named and written back): 1,317 files on 91 files of the data layer's repository,
    // 1,111 of them under 8 KB, 12% of the run's CPU.
    let estimate: usize = inputs.iter().map(|(_, s)| s.len() * s.columns().len() * 2).sum();
    let shared_placer = store.frame_placer(inputs.len() * 4 + 16, estimate.min(64 << 20));
    let placer = &shared_placer;
    let one_input = |rel: &String, source: &Arc<dyn Source>| -> Option<(u64, usize, Vec<EncodedFrame>)> {
        let _in = carry.enter();
        let _span = crate::code_rule_dsl::logic::profile::span(|| "an input's frames (job)".into());
        let queue = |puts: Vec<EncodedFrame>, kept: Vec<KeptPut>| {
            if !puts.is_empty() {
                store.queue_encoded_retagged(was.space, puts, Vec::new(), Vec::new(), Vec::new(), None);
            }
            store.queue_kept(was.space, kept);
        };
        let atoms: &crate::content_store::atoms::Atoms = atoms_ref;
        let mut puts: Vec<EncodedFrame> = Vec::new();
        // Each frame's entry in the space's kept facts (decision 0401 F6), until the frames are retired.
        let mut kept: Vec<KeptPut> = Vec::new();
        let mut written = 0;
        let name = was.names.get(rel)?;
        let width = source.columns().len();
        let types = types_of(prog, domains, rel);
        let cells = if types.len() == width { Cells::new(&atoms, &types) } else { Cells::plain(width) };
        let digest = digests.get(rel).copied().unwrap_or_else(|| digest(source.as_ref(), &cells));
        // Its statistics this run measured (an input no keeper counts, a semi-join's: `kept_inputs`' `stats_of`),
        // kept beside its frame at these rows (decision 0120 §3's part), unchanged rows or not: measured, they were
        // not kept at them.
        if let Some(columns) = input_stats.get(rel).filter(|_| kept_stats::on()) {
            let all: kept_stats::ByDigest = [((rel.clone(), digest), (columns.clone(), Vec::new(), BTreeSet::new()))].into_iter().collect();
            let words = kept_stats::to_words(&kept_stats::encode(&all));
            let put = FramePut { rel: name.clone(), part: kept_stats::PART, digest, reads: Vec::new(), rows: FrameRows { types: vec![b'i'], n: words.len(), rows: words } };
            kept.push(kept_flat(&put));
            puts.push(put.encoded());
        }
        if was.inputs.get(rel).is_some_and(|(d, _)| *d == digest) {
            queue(puts, kept);
            return Some((digest, written, Vec::new()));
        }
        // A relation of no columns is its count: one document's entry, its digest the input's (`digest`).
        if width == 0 {
            let n = source.len() as u64;
            let rows = vec![0, (digest ^ n.wrapping_mul(0xff51_afd7_ed55_8ccd)) as i64, n as i64];
            let put = FramePut { rel: name.clone(), part: DOCS_PART, digest, reads: Vec::new(), rows: FrameRows { types: vec![b'i'; 3], n: 1, rows } };
            kept.push(kept_flat(&put));
            puts.push(put.encoded());
            queue(puts, kept);
            return Some((digest, written, Vec::new()));
        }
        let g = group_col(prog, locations, rel).min(width - 1);
        let atom_owner = types.len() == width && atom_owners(&types, g);
        let owners = atom_owner.then_some(&*atoms);
        let granule = |d: i64| granule(d, owners);
        let docs = input_docs(rel).filter(|d| digest_of_docs(d) == digest).unwrap_or_else(|| Arc::new(docs_of(source.as_ref(), g, &cells)));
        let old = was.inputs.get(rel).and_then(|(_, d)| d.get()).map(Arc::clone);
        // The granules whose documents differ, or that hold a document come or gone.
        let differ = |doc: &i64| old.as_ref().is_none_or(|o| o.get(doc) != docs.get(doc));
        let mut changed: BTreeSet<i64> = docs.keys().filter(|d| differ(d)).map(|d| granule(*d)).collect();
        if let Some(o) = &old {
            changed.extend(o.keys().filter(|d| !docs.contains_key(*d)).map(|d| granule(*d)));
        }
        if std::env::var_os("RERAIL_INPUT_TRACE").is_some() && changed.len() <= 8 && source.len() > 20_000 {
            let differ_n: Vec<(i64, Option<(u64, u64)>, Option<(u64, u64)>)> = docs.iter().filter(|(d, _)| differ(d)).take(4).map(|(d, v)| (*d, Some(*v), old.as_ref().and_then(|o| o.get(d)).copied())).collect();
            eprintln!("rerail: input {rel} {:?} (group col {g} of {width}, {} rows, {} owners): granules {changed:?}; owners differ {differ_n:?}", prog.fields().ok().and_then(|f| f.columns(rel)), source.len(), docs.len());
        }
        if changed.len() > 8 && std::env::var_os("RERAIL_INPUT_TRACE").is_some() {
            let differ_n = docs.keys().filter(|d| differ(d)).count();
            let gone_n = old.as_ref().map_or(0, |o| o.keys().filter(|d| !docs.contains_key(*d)).count());
            let ex: Vec<String> = docs.iter().filter(|(d, _)| differ(d)).take(3).map(|(d, v)| format!("{d}: {v:?} was {:?}", old.as_ref().and_then(|o| o.get(d)))).collect();
            let hit = input_docs(rel).is_some_and(|d| digest_of_docs(&d) == digest);
            eprintln!("rerail: input {rel} (group col {g} of {width}, {} rows, docs {}, phase {:?}, layered {}): {} granules, {} owners of {} differ, {gone_n} gone; {}", source.len(), if hit { "the run's" } else { "recomputed" }, phase, source.as_any().and_then(|a| a.downcast_ref::<Layered>()).is_some(), changed.len(), differ_n, docs.len(), ex.join("; "));
        }
        // Each changed granule's rows, grouped by owner, owners ascending (`kept_inputs` reads each owner's
        // rows as one range), each in the source's order, read a window at a time into the granule's place in the
        // job's segments (`FramePlacer::place_rows`, Q89); copied and encoded on the heap only where no segment takes
        // it. A maintained input (`Layered`) gives each owner's rows from its ranges; another is read through once for
        // its rows' places (4 bytes a row), then by them.
        let layered = source.as_any().and_then(|a| a.downcast_ref::<Layered>());
        let all: Vec<usize> = (0..width).collect();
        let mut owners_of: BTreeMap<i64, Vec<i64>> = changed.iter().map(|g| (*g, Vec::new())).collect();
        let mut ids: BTreeMap<i64, BTreeMap<i64, Vec<u32>>> = BTreeMap::new();
        match layered {
            Some(_) => {
                for owner in docs.keys().filter(|d| changed.contains(&granule(**d))) {
                    owners_of.get_mut(&granule(*owner)).expect("a changed granule").push(*owner);
                }
            }
            None => {
                let mut row = vec![0i64; width];
                for r in 0..source.len() {
                    source.row_at(r, &all[g..=g], &mut row[g..=g]);
                    let owner = super::memo::document_of(row[g]);
                    let gr = granule(owner);
                    if changed.contains(&gr) {
                        ids.entry(gr).or_default().entry(owner).or_default().push(u32::try_from(r).expect("an input of fewer than 2^32 rows"));
                    }
                }
            }
        }
        // Each granule's frame is encoded straight into this job's segments as its rows are read (Q89): neither a copy
        // of its rows nor its bytes are made on the heap; the job's frames are queued with them when it ends.
        for part in changed.iter().copied() {
            let by_id = ids.remove(&part).unwrap_or_default();
            let mut owners = owners_of.remove(&part).unwrap_or_default();
            owners.sort_unstable();
            let n: usize = match layered {
                Some(l) => owners.iter().map(|o| l.owner_rows(*o).len() / width).sum(),
                None => by_id.values().map(Vec::len).sum(),
            };
            let each = |f: &mut dyn FnMut(&[i64])| match layered {
                Some(l) => owners.iter().for_each(|o| f(&l.owner_rows(*o))),
                None => {
                    let mut buf: Vec<i64> = Vec::with_capacity(4096.min(n) * width);
                    for r in by_id.values().flatten() {
                        let at = buf.len();
                        buf.resize(at + width, 0);
                        source.row_at(*r as usize, &all, &mut buf[at..]);
                        if buf.len() == 4096 * width {
                            f(&buf);
                            buf.clear();
                        }
                    }
                    if !buf.is_empty() {
                        f(&buf);
                    }
                }
            };
            written += n;
            // The granule's rows for its entry: gathered once more beside the frame's bytes until the frames go.
            let mut granule_rows: Vec<i64> = Vec::with_capacity(n * width);
            each(&mut |w: &[i64]| granule_rows.extend_from_slice(w));
            kept.push(kept_flat(&FramePut { rel: name.clone(), part, digest: 0, reads: Vec::new(), rows: FrameRows { types: types.clone(), rows: granule_rows, n } }));
            puts.push(match placer.place_rows(width, part, &each) {
                Some(p) => FramePut::placed_as(name.clone(), part, 0, Vec::new(), types.clone(), n, p),
                None => {
                    let mut rows: Vec<i64> = Vec::with_capacity(n * width);
                    each(&mut |w: &[i64]| rows.extend_from_slice(w));
                    FramePut { rel: name.clone(), part, digest: 0, reads: Vec::new(), rows: FrameRows { types: types.clone(), rows, n } }.encoded()
                }
            });
        }
        let mut table: Vec<(i64, (u64, u64))> = docs.iter().map(|(d, v)| (*d, *v)).collect();
        table.sort_unstable();
        let rows: Vec<i64> = table.iter().flat_map(|(d, (s, n))| [*d, *s as i64, *n as i64]).collect();
        // The granules its rows lie in, recorded with its documents' frame (`GRANULES_READ`): a run that reads none of
        // its owners admits them from this, decoding no frame (`kept_inputs`, Q91).
        let mask = table.iter().fold(0u64, |m, (d, _)| m | granule_bit(granule(*d)));
        let put = FramePut { rel: name.clone(), part: DOCS_PART, digest, reads: vec![(GRANULES_READ.to_owned(), mask)], rows: FrameRows { types: docs_types(atom_owner), n: table.len(), rows } };
        kept.push(kept_flat(&put));
        puts.push(put.encoded());
        store.queue_kept(was.space, kept);
        Some((digest, written, puts))
    };
    let mut framed_inputs: Vec<Option<(u64, usize, Vec<EncodedFrame>)>> = {
        use rayon::prelude::*;
        inputs.par_iter().map(|(rel, source)| one_input(rel, source)).collect()
    };
    let placed: Vec<EncodedFrame> = framed_inputs.iter_mut().flatten().flat_map(|(_, _, puts)| std::mem::take(puts)).collect();
    // None placed (every input as it was): nothing queued, as a run that changed nothing writes nothing.
    if !placed.is_empty() {
        let _in = carry.enter();
        store.queue_placed_retagged(was.space, placed, Vec::new(), Vec::new(), Vec::new(), None, shared_placer);
    }
    // Whether an input's frames were queued: then the derived relations' are queued with the program's hold even
    // when none of theirs changed.
    let mut inputs_queued = false;
    for ((rel, _), done) in inputs.iter().zip(framed_inputs) {
        let Some((digest, n, _)) = done else { continue };
        now.insert(rel.clone(), digest);
        inputs_queued |= was.inputs.get(rel).is_none_or(|(d, _)| *d != digest);
        written += n;
    }
    InputsWritten { now, written, queued: inputs_queued }
}

/// A relation written whole (`write_frames`): its digest, and its rows' bytes when its rows are not the stored ones.
struct Whole {
    d: u64,
    bytes: Option<WholeBytes>,
}

/// A frame's bytes made in its job: placed in the write's segments, or (no segment took them) on the heap.
pub(in crate::code_rule_dsl) enum WholeBytes {
    Placed(crate::content_store::PlacedFrame),
    Heap(Vec<u8>),
}

/// `held`'s live rows encoded into the write's segments (`placer`), read where they lie part by part; on the heap
/// when no segment takes them. No copy of the rows, nor of the bytes (Q89).
fn held_placed(held: &InRel, width: usize, placer: &crate::content_store::FramePlacer) -> WholeBytes {
    let each = |f: &mut dyn FnMut(&[i64])| {
        held.each_live_batch(0, held.len(), |rows| f(rows));
    };
    match placer.place_rows(width, DOCS_PART, &each) {
        Some(p) => WholeBytes::Placed(p),
        None => WholeBytes::Heap(held_bytes(held)),
    }
}

/// A frame of a derived relation from its bytes as made (`WholeBytes`).
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

/// `rel`'s entry: its rows lent whole. `None` for a relation with no block of its rows (a view, no columns).
fn kept_rows(name: &FrameRel, digest: u64, reads: Vec<(String, u64)>, types: Vec<u8>, rel: &Arc<InRel>) -> Option<KeptPut> {
    let block = rel.in_block()?;
    Some(KeptPut { rel: name.name.clone(), ident: name.ident, part: DOCS_PART, digest, reads, types, what: KeptWhat::Rows(Box::new(LentRel { rel: Arc::clone(rel), block })) })
}

/// A frame's rows as an entry of their own (an input's granule or documents, statistics' words).
fn kept_flat(put: &FramePut) -> KeptPut {
    let FrameRows { types, rows, n } = &put.rows;
    KeptPut { rel: put.rel.name.clone(), ident: put.rel.ident, part: put.part, digest: put.digest, reads: put.reads.clone(), types: types.clone(), what: KeptWhat::Flat { widths: vec![8; types.len()], rows: rows.clone(), n: *n } }
}

/// The entry's rows as they are, with `reads` (a frame's retag).
fn kept_reads(name: &FrameRel, part: i64, reads: Vec<(String, u64)>) -> KeptPut {
    KeptPut { rel: name.name.clone(), ident: name.ident, part, digest: 0, reads, types: Vec::new(), what: KeptWhat::Reads }
}

fn derived_frame(name: &FrameRel, d: u64, reads: Vec<(String, u64)>, types: Vec<u8>, n: usize, bytes: WholeBytes) -> EncodedFrame {
    match bytes {
        WholeBytes::Placed(p) => FramePut::placed_as(name.clone(), DOCS_PART, d, reads, types, n, p),
        WholeBytes::Heap(b) => FramePut::encoded_as(name.clone(), DOCS_PART, d, reads, types, n, b),
    }
}

/// A derived relation the cross-file run handed off once no later stratum read it (`plan::Hooks::hand_off`), with
/// its frame made then: its rows' digest and encoding, its levels. The run drops its rows, so the part's relations
/// are not all held to its end to be written (Q74: 2,700 of them, the cross part's peak on 300 files). Not held for
/// this process's next run either: it reads them from the frames, as a new process does.
pub(in crate::code_rule_dsl) struct Handed {
    n: usize,
    types: Vec<u8>,
    d: u64,
    /// Its bytes, placed in the part's write (`placer`); none when its rows are the stored ones (`was`), whose frame
    /// stays (its reads retagged if they changed).
    bytes: Option<WholeBytes>,
    levels: Option<Vec<u32>>,
}

impl Handed {
    /// `held`'s frame as `write_frames` makes one written whole, encoded into the part's write (`placer`, which
    /// `write_frames` queues); `None` for a relation of no rows of its own (a view). `was`: its stored frame's digest.
    pub(in crate::code_rule_dsl) fn of(store: &ContentStore, prog: &Placed, domains: &lower::Domains, rel: &str, held: &InRel, was: Option<u64>, placer: &crate::content_store::FramePlacer) -> Option<Handed> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "a relation handed off, its frame made".into());
        let n = held.rows();
        let types = frame_types(prog, domains, rel, held.types());
        let atoms = store.atoms_handle();
        let d = held_digest(&Cells::new(&atoms, &types), held)?;
        let bytes = (was != Some(d)).then(|| held_placed(held, types.len(), placer));
        Some(Handed { n, types, d, bytes, levels: held.live_levels() })
    }
}

impl Retained {
    /// Whether `rel`'s change is written as a delta after its frame (decision 0117 step 4), given its reads are
    /// known: its frame's deltas few against its rows, and no levels (written whole: decision 0090).
    fn by_delta(&self, store: &ContentStore, was: &InFrames, rel: &str, deltas: &HashMap<String, crate::code_rule_dsl::logic::plan::Delta>) -> bool {
        let (Some(delta), Some(_), Some(name), Some(k)) = (deltas.get(rel), was.derived.get(rel), was.names.get(rel), self.framed(rel)) else { return false };
        if k.width == 0 || k.rel.has_levels() {
            return false;
        }
        let (ng, na) = delta.counts(k.width);
        let (chain, chain_rows) = store.frame_chain(&was.space, name);
        chain < max_deltas() && (chain_rows + ng + na) * 4 <= k.rel.rows().max(4096)
    }

    /// Each kept relation's statistics of its rows now, as a part of its frame under the rows' digest (`now`, as
    /// the frames name them; decision 0120 §3).
    fn stats_puts(&self, was: &InFrames, now: &HashMap<String, u64>, puts: &mut Vec<EncodedFrame>, kept: &mut Vec<KeptPut>) {
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
            let words = kept_stats::to_words(&kept_stats::encode(&all));
            if std::env::var_os("RERAIL_KEPT_STATS_TRACE").is_some() {
                eprintln!("rerail: kept statistics written: {rel} ({} entries)", all.len());
            }
            let put = FramePut { rel: name.clone(), part: kept_stats::PART, digest, reads: Vec::new(), rows: FrameRows { types: vec![b'i'], n: words.len(), rows: words } };
            kept.push(kept_flat(&put));
            puts.push(put.encoded());
        }
    }
}

mod kept_stats;

#[cfg(test)]
pub mod frame_tests;

/// The frame digest of `held`'s live rows (`rows_digest`), read where they lie a batch at a time: no copy of the
/// relation (it was copied whole for each frame written, 460 MB of a 300-file run's peak). `None` for a view.
pub(in crate::code_rule_dsl) fn held_digest(cells: &Cells, held: &InRel) -> Option<u64> {
    held.each_live_batch(0, 0, |_| {})?;
    Some(cells.digest_batches(held.len(), held.rows(), &|lo, hi, f| {
        held.each_live_batch(lo, hi, |rows| f(rows));
    }))
}

/// `held`'s live rows encoded as a frame's (`FramePut::encode_rows`) on the heap, read where they lie part by part:
/// for a frame no segment took (`held_placed`).
pub(in crate::code_rule_dsl) fn held_bytes(held: &InRel) -> Vec<u8> {
    FramePut::encode_windows(held.types().len(), &|f| {
        held.each_live_batch(0, held.len(), |rows| f(rows));
    })
}
