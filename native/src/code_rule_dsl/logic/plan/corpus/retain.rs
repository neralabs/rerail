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
//! past it, a relation is not kept and its stratum runs again next time. Nor is one the run recomputed in its readers
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
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use crate::code_rule_dsl::logic::native::component::InRel;
use crate::content_store::{ContentStore, FramePut, FrameRel, FrameRows, FrameStored};

/// A relation as the last run derived it: the engine's relation, and its decoded rows when it is an output.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Kept {
    pub(in crate::code_rule_dsl) rel: Arc<InRel>,
    pub(in crate::code_rule_dsl) width: usize,
    pub(in crate::code_rule_dsl) output: Option<(Vec<String>, Vec<usize>)>,
}

impl Kept {
    fn bytes(&self) -> usize {
        self.rel.bytes() + self.output.as_ref().map_or(0, |(lines, _)| lines.iter().map(String::len).sum())
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
    /// The statistics a run measured of a kept relation, under the name it was bound as (itself, or `r__old`) and
    /// the relation it is (decision 0094 §4: measured once, while it is kept as it is).
    measured: HashMap<(String, usize), Measured>,
    /// Each input's statistics as the run planned it (`Stats::of_relation`): next run's `rel__old`'s, not measured.
    input_stats: HashMap<String, InputStats>,
    /// The documents the run read, and the file-local program their memo entries are under (`input_delta`).
    docs: (super::input_delta::Docs, [u8; 32]),
    /// Each input source's rows hashed by document (`document_delta`), when a run took them: the next run's old
    /// side, not hashed again.
    input_docs: HashMap<String, Arc<DocDigests>>,
    /// A maintained relation's statistics kept by its delta (decision 0096 step 4), under the name it is bound as
    /// next run (`r__old`): of the engine's relation `at`, which a run derived as the one before less the rows it
    /// marked dead and with those it added (`InRel::delta_since`).
    patched: HashMap<String, Patched>,
    /// What the store's frames hold of this part, when it was read from them (t33 K4): a process with no session
    /// starts from them, and writes back what its run changed.
    frames: Option<InFrames>,
}

/// A relation's statistics kept by delta, of the engine's relation at address `at`, with its counts of the
/// constants `tested` (`Measured`'s second and third).
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Patched {
    pub(in crate::code_rule_dsl) at: usize,
    pub(in crate::code_rule_dsl) stats: crate::code_rule_dsl::logic::plan::statkeep::RelStats,
    pub(in crate::code_rule_dsl) counts: Vec<(String, i64, i64)>,
    pub(in crate::code_rule_dsl) tested: BTreeSet<(usize, i64)>,
}

impl Patched {
    /// The statistics of `new`, derived from `old` (whose these are) by `delta_since`: these patched by its rows
    /// gone and added. `None` if `new` did not start as `old`.
    pub(in crate::code_rule_dsl) fn after(&self, old: &InRel, new: &Arc<InRel>, width: usize) -> Option<Patched> {
        let (added, removed) = new.delta_since(old)?;
        let mut p = self.clone();
        p.at = Arc::as_ptr(new) as *const u8 as usize;
        p.stats.delta(&added, &removed, width);
        // Each constant's count, by the rows of its column holding it.
        let index: HashMap<&str, usize> = p.stats.columns().iter().enumerate().map(|(c, n)| (n.as_str(), c)).collect();
        for (name, v, n) in &mut p.counts {
            let Some(&c) = index.get(name.as_str()).filter(|c| **c < width) else { continue };
            let count = |flat: &[i64]| flat.chunks_exact(width).filter(|r| r[c] == *v).count() as i64;
            *n += count(&added) - count(&removed);
        }
        Some(p)
    }
}

/// A source's rows by document: each document's sum of row hashes and its row count.
pub(in crate::code_rule_dsl) type DocDigests = rustc_hash::FxHashMap<i64, (u64, u64)>;

/// A relation's statistics as bound (`syntax::statistics`), and its counts of the constants rules test it by.
/// The counts are of the constants tested when they were measured (the third): taken again only for the same.
pub(in crate::code_rule_dsl) type Measured = (Vec<(String, i64, i64, i64, i64)>, Vec<(String, i64, i64)>, BTreeSet<(usize, i64)>);

/// A relation's statistics by column, and its exact postings by (column, value).
pub(in crate::code_rule_dsl) type InputStats = (Vec<[i64; 4]>, Vec<(usize, i64, i64)>);

/// The key of a cross-file part: the prepared program's key and the part's program as lowered.
pub(in crate::code_rule_dsl) fn key(prepared: &[u8; 32], cross: &front::Program) -> [u8; 32] {
    use sha2::Digest as _;
    let mut hash = sha2::Sha256::new();
    hash.update(b"rerail.cross-retained/v1\0");
    hash.update(prepared);
    hash.update(format!("{:?}\n{:?}\n{:?}\n{:?}", cross.rules, cross.decls, cross.inputs, cross.outputs).as_bytes());
    hash.finalize().into()
}

/// An order-independent digest of `source`'s rows (a set: rows are distinct), with its row count.
pub(in crate::code_rule_dsl) fn digest(source: &dyn Source) -> u64 {
    let width = source.columns().len();
    let n = source.len();
    let mut sum: u64 = 0;
    for i in 0..n {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for c in 0..width {
            h = (h ^ source.value(i, c) as u64).wrapping_mul(0x0000_0100_0000_01b3);
            h ^= h >> 29;
        }
        sum = sum.wrapping_add(h.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    }
    sum ^ (n as u64).wrapping_mul(0xff51_afd7_ed55_8ccd)
}

/// An input's rows new and its rows gone between its last run's source `old` and this run's `new`, as
/// `maintain::rewrite`'s `given` binds them (`r__plus`, `r__minus`), found document by document: each document's rows (by the
/// one its first column names, `memo::document_of`) are hashed as `digest` hashes rows, and only the documents whose
/// hashes differ are compared row by row. A pass over the columns, not a join of every new row against every old
/// one (`r__plus :- r, !r__old`), which read each base relation whole every run however few files changed. Exact
/// whatever the documents are: one whose rows all moved is a document that differs. `None` when the two hold other
/// columns. Each is named as the planner reads its statistics (`sql_col` of `rel__plus`, `rel__minus`).
pub(in crate::code_rule_dsl) fn document_delta(rel: &str, old: &dyn Source, was: Option<&DocDigests>, new: &dyn Source, g: usize) -> Option<((Arc<dyn Source>, Arc<dyn Source>), Arc<DocDigests>)> {
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
    let hash = |row: &[i64]| {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &v in row {
            h = (h ^ v as u64).wrapping_mul(0x0000_0100_0000_01b3);
            h ^= h >> 29;
        }
        h.wrapping_mul(0x9e37_79b9_7f4a_7c15)
    };
    // A document's rows are mostly one run of rows: its sum is kept while the run lasts, not looked up per row.
    let by_doc = |s: &dyn Source, row: &mut [i64]| {
        let mut by = DocDigests::default();
        let mut at: Option<(i64, u64, u64)> = None;
        for r in 0..s.len() {
            s.row_at(r, &all, row);
            let d = owner_of(row, g);
            let h = hash(row);
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
        }
        if let Some((doc, sum, n)) = at {
            let e = by.entry(doc).or_default();
            e.0 = e.0.wrapping_add(sum);
            e.1 += n;
        }
        by
    };
    let taken;
    let was = match was {
        Some(w) => w,
        None => {
            taken = by_doc(old, &mut row);
            &taken
        }
    };
    let now = by_doc(new, &mut row);
    let differ: FxHashSet<i64> = was.iter().filter(|(d, h)| now.get(*d) != Some(*h)).map(|(d, _)| *d).chain(now.keys().filter(|d| !was.contains_key(*d)).copied()).collect();
    let rows_of = |s: &dyn Source, row: &mut [i64]| -> FxHashSet<Vec<i64>> {
        let mut out = FxHashSet::default();
        if differ.is_empty() {
            return out;
        }
        for r in 0..s.len() {
            s.row_at(r, &all[g..=g], &mut row[g..=g]);
            if differ.contains(&super::memo::document_of(row[g])) {
                s.row_at(r, &all, row);
                out.insert(row.to_vec());
            }
        }
        out
    };
    let (gone, made) = (rows_of(old, &mut row), rows_of(new, &mut row));
    let relation = |suffix: &str, rows: &mut dyn Iterator<Item = &Vec<i64>>| {
        let names: Vec<String> = (0..width).map(|c| crate::code_rule_dsl::logic::plan::sql_col(&format!("{rel}__{suffix}"), c)).collect();
        let mut rel = crate::code_rule_dsl::facts::vtab::Relation::new(&names.iter().map(String::as_str).collect::<Vec<_>>());
        let mut rows: Vec<&Vec<i64>> = rows.collect();
        rows.sort_unstable();
        for r in rows {
            rel.push(r);
        }
        Arc::new(rel) as Arc<dyn Source>
    };
    Some(((relation("plus", &mut made.difference(&gone)), relation("minus", &mut gone.difference(&made))), Arc::new(now)))
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
struct Reads<'a> {
    prog: &'a front::Program,
    by_head: HashMap<&'a str, Vec<&'a front::Rule>>,
}

impl<'a> Reads<'a> {
    fn of(prog: &'a front::Program) -> Self {
        let mut by_head: HashMap<&str, Vec<&front::Rule>> = HashMap::new();
        for r in &prog.rules {
            by_head.entry(r.head.rel.as_str()).or_default().push(r);
        }
        Reads { prog, by_head }
    }

    fn of_scc(&self, scc: &BTreeSet<String>) -> BTreeSet<String> {
        let rules = || scc.iter().filter_map(|rel| self.by_head.get(rel.as_str())).flatten();
        let mut out: BTreeSet<String> = rules().flat_map(|r| &r.body).filter_map(|l| l.atom().map(|a| a.rel.to_string())).collect();
        // A `nearest` literal climbs the tree: it reads `node` and `child`, which an edit changes.
        if rules().any(|r| r.body.iter().any(|l| matches!(l, front::Literal::Nearest { .. }))) {
            out.extend(TREE.iter().map(|r| (*r).to_owned()));
        }
        for rel in scc {
            if let Some(native) = self.prog.natives.get(rel) {
                out.extend(native.reads().into_iter().map(str::to_owned));
            }
        }
        out
    }
}

/// The strata of `prog` that `kept` (of program `key`) lets a run skip, given its inputs' digests.
pub(in crate::code_rule_dsl) fn cut(kept: Option<&Retained>, key: [u8; 32], prog: &front::Program, inputs: &HashMap<String, u64>) -> Result<Cut> {
    let (sccs, _) = front::strata(prog).map_err(|e| Error::Plan(e.to_string()))?;
    let kept = kept.filter(|k| k.key == key);
    let mut changed: Vec<String> = inputs.iter().filter(|(rel, d)| kept.and_then(|k| k.inputs.get(*rel)) != Some(*d)).map(|(rel, _)| rel.clone()).collect();
    changed.sort();
    let mut dirty: BTreeSet<String> = changed.iter().cloned().collect();
    let mut clean = BTreeSet::new();
    // Gates decide on the whole program. Reuse it only when every input and derived relation survived unchanged.
    // A view of one input (`input_views`, only with `RERAIL_SESSION_VIEWS=on`) is never kept: its rule stays.
    let views = super::maintain::input_views(prog);
    let complete = kept.is_some_and(|k| sccs.iter().flatten().all(|r| k.holds(r, prog) || views.contains_key(r)));
    let whole = kept.is_none() || (!prog.gates.is_empty() && (!changed.is_empty() || !complete));
    let mut ran = 0;
    let reads = Reads::of(prog);
    for scc in &sccs {
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
    pub(in crate::code_rule_dsl) fn program(&self, cross: &lower::Lowered) -> lower::Lowered {
        let mut program = cross.program.clone();
        program.rules.retain(|r| !self.clean.contains(r.head.rel.as_str()));
        program.natives.retain(|rel, _| !self.clean.contains(rel));
        if self.strata.0 == 0 {
            program.gates.clear();
        }
        let read: BTreeSet<String> = program.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom().map(|a| a.rel.to_string())).collect();
        for rel in self.clean.iter().filter(|r| read.contains(*r)) {
            if !program.inputs.contains(rel) {
                program.inputs.push(rel.clone());
            }
        }
        program.outputs.retain(|o| !self.clean.contains(o));
        lower::Lowered { program, domains: cross.domains.clone(), ops: cross.ops, dialect: cross.dialect }
    }

    /// The clean relations `program` reads, as kept: each as a source (bound by the caller, as the other inputs) and
    /// as the engine's relation (adopted by the caller, read in place).
    pub(in crate::code_rule_dsl) fn sources(&self, kept: &Retained, program: &front::Program) -> Vec<(String, Arc<dyn Source>, Option<Arc<InRel>>)> {
        self.clean
            .iter()
            .filter(|rel| program.inputs.contains(*rel))
            .filter_map(|rel| {
                let k = kept.rels.get(rel)?;
                Some((rel.clone(), k.source(rel), Some(Arc::clone(&k.rel))))
            })
            .collect()
    }

    /// The clean outputs' decoded rows and node columns, as the last run gave them.
    pub(in crate::code_rule_dsl) fn outputs(&self, kept: &Retained, outputs: &[String]) -> Vec<(String, Vec<String>, Vec<usize>)> {
        outputs
            .iter()
            .filter(|o| self.clean.contains(*o))
            .filter_map(|o| kept.rels.get(o).and_then(|k| k.output.clone()).map(|(lines, nodes)| (o.clone(), lines, nodes)))
            .collect()
    }

    /// The relations this run derives, in stratum order: kept for the next run, as far as the budget goes.
    pub(in crate::code_rule_dsl) fn derived(&self, prog: &front::Program) -> Result<Vec<String>> {
        let (sccs, _) = front::strata(prog).map_err(|e| Error::Plan(e.to_string()))?;
        Ok(sccs.into_iter().flatten().filter(|r| !self.clean.contains(r)).collect())
    }
}

/// The byte budget of what a session keeps of its cross-file part.
pub(in crate::code_rule_dsl) fn budget() -> usize {
    std::env::var("RERAIL_CROSS_RETAIN_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(256) << 20
}

/// What the next run keeps: the clean relations as they were, and those this run derived (each the engine's
/// relation, with its width), in stratum order while they fit the budget, with the decoded `outputs`; and every
/// input's source (held anyway while the session's inputs are: nothing is copied).
pub(in crate::code_rule_dsl) fn after(
    old: Option<Retained>,
    key: [u8; 32],
    inputs: HashMap<String, u64>,
    cut: &Cut,
    derived: Vec<(String, usize, Arc<InRel>)>,
    outputs: &BTreeMap<String, (Vec<String>, Vec<usize>)>,
    sources: &[(String, Arc<dyn Source>)],
    measured: HashMap<(String, usize), Measured>,
    input_stats: HashMap<String, InputStats>,
    mut input_docs: HashMap<String, Arc<DocDigests>>,
    docs: (super::input_delta::Docs, [u8; 32]),
    mut patched: HashMap<String, Patched>,
) -> Retained {
    // An input bound as the last run's source keeps that source's hashes.
    if let Some(old) = &old {
        for (rel, s) in sources {
            if !input_docs.contains_key(rel)
                && let (Some(d), Some(was)) = (old.input_docs.get(rel), old.input_srcs.get(rel))
                && Arc::ptr_eq(was, s)
            {
                input_docs.insert(rel.clone(), Arc::clone(d));
            }
        }
    }
    let mut rels: HashMap<String, Kept> = HashMap::new();
    if let Some(old) = old.filter(|o| o.key == key) {
        // A relation this run did not derive again keeps its statistics kept by delta, as it keeps its rows.
        for (name, p) in old.patched {
            patched.entry(name).or_insert(p);
        }
        for (rel, k) in old.rels {
            if cut.clean.contains(&rel) {
                rels.insert(rel, k);
            }
        }
    }
    let mut bytes: usize = rels.values().map(Kept::bytes).sum();
    let budget = budget();
    for (rel, width, held) in derived {
        let k = Kept { rel: held, width, output: outputs.get(&rel).cloned() };
        bytes += k.bytes();
        if bytes > budget {
            break;
        }
        rels.insert(rel, k);
    }
    let input_srcs = sources.iter().filter(|(_, s)| !s.columns().is_empty()).map(|(rel, s)| (rel.clone(), Arc::clone(s))).collect();
    // Statistics of the relations still kept, as they are.
    let alive: std::collections::HashSet<usize> = rels.values().map(|k| Arc::as_ptr(&k.rel) as *const u8 as usize).collect();
    let measured = measured.into_iter().filter(|((_, at), _)| alive.contains(at)).collect();
    let patched = patched.into_iter().filter(|(_, p)| alive.contains(&p.at)).collect();
    Retained { key, inputs, rels, input_srcs, measured, input_stats, input_docs, docs, patched, frames: None }
}

/// `source` (input `rel`), or the last run's source of the same rows: same name, columns and digest, in any kept
/// part. Read in place of the copy, so its column indexes and statistics are kept with it, not built again (0094
/// §4). Its digest goes to `digests`, which the session's cut reads. An unheld column reads as 0 in both, so the
/// rows read alike whatever the program.
pub(in crate::code_rule_dsl) fn kept_source(
    parts: Option<&HashMap<[u8; 32], Retained>>,
    rel: &str,
    source: Arc<dyn Source>,
    digests: &mut HashMap<String, u64>,
) -> Arc<dyn Source> {
    let Some(parts) = parts else { return source };
    let d = digest(source.as_ref());
    digests.insert(rel.to_owned(), d);
    let columns = source.columns();
    if let Some(same) = parts.values().find_map(|p| p.input_srcs.get(rel).filter(|s| p.inputs.get(rel) == Some(&d) && s.columns() == columns).cloned()) {
        return same;
    }
    // Changed rows: last run's sorted columns, patched by the documents that changed, not sorted again (decision
    // 0101 §5). `RERAIL_SORT_BASE_AGAIN` sorts them again, as before.
    if std::env::var_os("RERAIL_SORT_BASE_AGAIN").is_none()
        && let Some(old) = parts.values().find_map(|p| p.input_srcs.get(rel).filter(|s| s.columns() == columns))
    {
        let _s = crate::code_rule_dsl::logic::profile::span(|| format!("carry {rel}'s sorted columns"));
        let (laid, _) = crate::code_rule_dsl::logic::native::component::carry_sorted(old.as_ref(), source.as_ref());
        if let Some(laid) = laid {
            return laid;
        }
    }
    source
}

impl Retained {
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
        let rels = self.rels.iter().map(|(rel, k)| (rel.as_str(), k.width, &k.rel)).collect();
        (inputs, rels)
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
    pub(in crate::code_rule_dsl) fn measured(&self, name: &str, held: &Arc<InRel>) -> Option<&Measured> {
        self.measured.get(&(name.to_owned(), Arc::as_ptr(held) as *const u8 as usize))
    }

    /// The statistics kept by delta of kept relation `held` bound as `name` (`Patched`), if a run kept them.
    pub(in crate::code_rule_dsl) fn patched(&self, name: &str, held: &Arc<InRel>) -> Option<&Patched> {
        self.patched.get(name).filter(|p| p.at == Arc::as_ptr(held) as *const u8 as usize)
    }

    /// Every statistic kept, to be carried to the next run's (`after`).
    pub(in crate::code_rule_dsl) fn measured_all(&self) -> HashMap<(String, usize), Measured> {
        self.measured.clone()
    }

    /// The last run's source of an input, as it bound it.
    /// Kept relation `rel` and its width, if this run kept it.
    pub(in crate::code_rule_dsl) fn kept(&self, rel: &str) -> Option<(&Arc<InRel>, usize)> {
        self.rels.get(rel).map(|k| (&k.rel, k.width))
    }

    pub(in crate::code_rule_dsl) fn input_source(&self, rel: &str) -> Option<&Arc<dyn Source>> {
        self.input_srcs.get(rel)
    }

    /// That source's rows hashed by document, if a run took them (`document_delta`).
    pub(in crate::code_rule_dsl) fn input_docs(&self, rel: &str) -> Option<&Arc<DocDigests>> {
        self.input_docs.get(rel)
    }

    /// The digest the last run took of an input (`digest`, or a phase export's version).
    pub(in crate::code_rule_dsl) fn input_digest(&self, rel: &str) -> Option<u64> {
        self.inputs.get(rel).copied()
    }

    /// Whether the last run's rows of a relation or an input are kept.
    pub(in crate::code_rule_dsl) fn has_old(&self, rel: &str) -> bool {
        self.rels.contains_key(rel) || self.input_srcs.contains_key(rel)
    }
    /// A relation's or an input's rows as the last run had them, as a source named `name` (`sql_col`), with the
    /// engine's relation for a derived one (to adopt, read in place).
    /// An input's statistics as the last run planned it.
    /// The documents the last run read, if under the file-local program `program`.
    pub(in crate::code_rule_dsl) fn docs(&self, program: &[u8; 32]) -> Option<&super::input_delta::Docs> {
        (self.docs.1 == *program).then_some(&self.docs.0)
    }
    pub(in crate::code_rule_dsl) fn input_stats(&self, rel: &str) -> Option<&InputStats> {
        self.input_stats.get(rel)
    }
    pub(in crate::code_rule_dsl) fn old_source(&self, rel: &str, name: &str) -> Option<(Arc<dyn Source>, Option<Arc<InRel>>)> {
        match self.rels.get(rel) {
            Some(k) => Some((k.source(name), Some(Arc::clone(&k.rel)))),
            None => self.input_srcs.get(rel).map(|s| {
                let names = (0..s.columns().len()).map(|c| crate::code_rule_dsl::logic::plan::sql_col(name, c)).collect();
                (Arc::new(Renamed(Arc::clone(s), names)) as Arc<dyn Source>, None)
            }),
        }
    }
    /// A relation's or an input's rows as the last run had them, copied (for the spikes' checks).
    pub(in crate::code_rule_dsl) fn old_rows(&self, rel: &str) -> Option<Vec<Vec<i64>>> {
        let (s, _) = self.old_source(rel, rel)?;
        let w = s.columns().len();
        Some((0..s.len()).map(|i| (0..w).map(|c| s.value(i, c)).collect()).collect())
    }
    pub(in crate::code_rule_dsl) fn relations(&self) -> usize {
        self.rels.len()
    }
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.rels.values().map(Kept::bytes).sum()
    }
    /// The index-cost spike (`RERAIL_INDEX_REPORT`): each kept relation's dedup set and indexes now, the key-column
    /// sets its probes bound (`probes`, by relation name: a stratum's own table, `+delta` for a delta probe, and its
    /// `__` forms), and what one index per probed key set would hold: a row id a row (4 B) and a directory (0.5 B),
    /// the dedup riding on the first such index (sorted by key hash then row), or a set of row ids (6 B) when none is
    /// probed; per-column statistics from sketches (256 B a column).
    pub(in crate::code_rule_dsl) fn index_report(&self, probes: &HashMap<(String, Vec<usize>), u64>) -> String {
        let base = |name: &str| -> String {
            let n = name.split(" [").next().unwrap_or(name).trim_end_matches("+delta");
            let n = ["__old", "__new", "__plus", "__minus", "__ins", "__del", "__kept"].iter().fold(n, |n, s| n.strip_suffix(s).unwrap_or(n));
            n.to_owned()
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
        let mut totals = [("supports", 0usize, 0usize, 0usize, 0usize), ("other", 0, 0, 0, 0)];
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
            let t = &mut totals[usize::from(!super::supports::is_support(rel))];
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
    /// What the kept relations hold, by kind of relation (a counting support, `h__sup…`, or any other): rows,
    /// values, levels, and indexes and sets, in bytes (`InRel::held_bytes`). `RERAIL_KEPT_BYTES` prints it.
    pub(in crate::code_rule_dsl) fn held(&self) -> [(&'static str, usize, usize, usize, usize); 2] {
        let mut out = [("supports", 0, 0, 0, 0), ("other", 0, 0, 0, 0)];
        for (rel, k) in &self.rels {
            let at = usize::from(!super::supports::is_support(rel));
            let (values, levels, index) = k.rel.held_bytes();
            let o = &mut out[at];
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
        Layered { names, kept, ranges, dead, before, added, appended }
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
        let ((plus, minus), docs) = document_delta("r", old.as_ref(), None, new.as_ref(), 0).unwrap();
        assert_eq!(rows(&plus), BTreeSet::from([vec![node(2, 1), 22], vec![node(4, 0), 40]]));
        assert_eq!(rows(&minus), BTreeSet::from([vec![node(2, 1), 21], vec![node(3, 0), 30]]));
        // The next run: `new` is old, its hashes kept; nothing changed.
        let ((plus, minus), _) = document_delta("r", new.as_ref(), Some(&docs), source(&[[node(1, 0), 10], [node(1, 1), 11], [node(2, 0), 20], [node(2, 1), 22], [node(4, 0), 40]]).as_ref(), 0).unwrap();
        assert_eq!((plus.len(), minus.len()), (0, 0));
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
            let width=p.decls[name].len();
            (name.clone(),Kept { rel:InRel::flat(&[],vec![b'i';width]),width,output:None })
        }).collect();
        let mut kept=Retained {key,inputs:inputs.clone(),rels,input_srcs:Default::default(),measured:Default::default(),input_stats:Default::default(),input_docs:Default::default(),docs:Default::default(),patched:Default::default(),frames:None};
        let result=cut(Some(&kept),key,&p,&inputs).unwrap();
        assert_eq!(result.strata,(0,strata.len()));
        let lowered=lower::Lowered {program:p.clone(),domains:Default::default(),ops:[0;3],dialect:crate::code_rule_dsl::language::GrammarDialect::Python};
        let reused=result.program(&lowered);
        assert!(reused.program.rules.is_empty() && reused.program.gates.is_empty());
        let mut changed=inputs.clone();
        *changed.values_mut().next().unwrap()=8;
        for (candidate,rows) in [(key,changed),([2;32],inputs.clone())] {
            assert_eq!(cut(Some(&kept),candidate,&p,&rows).unwrap().strata,(strata.len(),strata.len()));
        }
        let missing=kept.rels.keys().next().unwrap().clone();
        kept.rels.remove(&missing);
        assert_eq!(cut(Some(&kept),key,&p,&inputs).unwrap().strata,(strata.len(),strata.len()));
    }
}

/// Granules an input's frames are split into, by document (K1's partition): an edit rewrites the granules of the
/// documents it changed, not the input.
const GRANULES: i64 = 64;
/// The part of an input's frames that holds each document's digest (`DocDigests`), as rows (document, sum, rows);
/// its frame's digest is the input's (`digest`). Granule `g` is part `1 + g`.
const DOCS_PART: i64 = 0;

/// What the store's frames hold of a cross-file part (t33 K4), read before the run's cut: the derived relations
/// stored and current, and each input's digest by document. Rows are read only for what the run then reads.
#[derive(Clone, Default)]
pub(in crate::code_rule_dsl) struct InFrames {
    space: [u8; 32],
    /// Each relation and input of the program, as its frame names it.
    names: HashMap<String, FrameRel>,
    /// Each derived relation stored and current: its digest and the digest each of its reads had.
    derived: HashMap<String, FrameStored>,
    /// Each input as stored: its digest, and its rows' digest by document.
    inputs: HashMap<String, (u64, Arc<DocDigests>)>,
}

/// The digest of `read` a derived relation records (t33 K4): as `known` has it (an input as the run bound it, a
/// relation as derived or named), or, for the program's stated facts, their identity, which is the program's.
/// Anything else (an answer the run binds by another way, as a load's) has none: what reads it is not current.
fn read_digest(read: &str, known: &HashMap<String, u64>, derived: &BTreeSet<&String>, prog: &front::Program) -> Option<u64> {
    if let Some(d) = known.get(read) {
        return Some(*d);
    }
    // A relation with literal rows that is also an input is named by its bound rows' digest, never by its literals.
    if derived.iter().any(|r| r.as_str() == read) || prog.inputs.iter().any(|i| i == read) {
        return None;
    }
    // Neither derived nor an input, and not bound: its rows are the program's own (its facts, or none, as an
    // earlier phase's export of no rows), the same whatever the run reads, so its identity names them.
    let id = crate::code_rule_dsl::logic::plan::leaf_identity(prog, read);
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

/// An input's identity: its name, its columns and the program that binds it (`key`). Each program's input is a frame
/// of its own (phase 1's `doc` and the pack's are not one frame each overwrites with its rows), while a derived
/// relation is named by its rules and what they read (`plan::identities`), so two programs that derive it from
/// inputs of the same digests share it (decision 0103).
fn input_ident(rel: &str, prog: &front::Program, key: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    // v3: an input's rows grouped by owner within each granule (decision 0117 step 2), so each owner's are one range.
    h.update(b"rerail.relation/v3\0input\0");
    h.update(rel.as_bytes());
    h.update(format!("\0{:?}\0", prog.decls.get(rel)).as_bytes());
    h.update(key);
    h.finalize().into()
}

/// A relation's column types, as frames store them.
fn types_of(prog: &front::Program, rel: &str) -> Vec<u8> {
    prog.decls.get(rel).map(|cols| cols.iter().map(|(_, t)| if t == "sym" { b's' } else { b'i' }).collect()).unwrap_or_default()
}

/// The column whose document owns an input's rows (decision 0117 step 2): its first node column, the column the memo
/// attributes an export's rows by (`split.locations`); else the column `memo::owner` names (a string's, for
/// `value_type`); else the first. Every row of one owner is held by the same files, and the frames keep an input's
/// rows grouped by it, so an edit reads only the owners it changed.
pub(in crate::code_rule_dsl) fn group_col(prog: &front::Program, rel: &str) -> usize {
    let decl = prog.decls.get(rel);
    let width = decl.map_or(0, Vec::len);
    decl.and_then(|c| c.iter().position(|(_, t)| t == "node"))
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

/// A source's rows' digest by owner (`group_col` `g`), as `document_delta` takes it.
pub(in crate::code_rule_dsl) fn docs_of(source: &dyn Source, g: usize) -> DocDigests {
    let width = source.columns().len();
    let mut by = DocDigests::default();
    if width == 0 {
        return by;
    }
    let all: Vec<usize> = (0..width).collect();
    let mut row = vec![0i64; width];
    for r in 0..source.len() {
        source.row_at(r, &all, &mut row);
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &v in &row {
            h = (h ^ v as u64).wrapping_mul(0x0000_0100_0000_01b3);
            h ^= h >> 29;
        }
        let e = by.entry(owner_of(&row, g)).or_default();
        e.0 = e.0.wrapping_add(h.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        e.1 += 1;
    }
    by
}

/// An input of the cross part as the last run kept it (decision 0117 step 1): its rows, named as the program
/// declares them, its rows' digests by owner, its digest, and each owner's rows as a range of them (step 2).
pub(in crate::code_rule_dsl) struct KeptInput {
    pub(in crate::code_rule_dsl) source: Arc<dyn Source>,
    pub(in crate::code_rule_dsl) docs: Arc<DocDigests>,
    pub(in crate::code_rule_dsl) digest: u64,
    pub(in crate::code_rule_dsl) ranges: Arc<Ranges>,
}

/// Each owner's rows in a kept input: (first row, rows).
pub(in crate::code_rule_dsl) type Ranges = rustc_hash::FxHashMap<i64, (u32, u32)>;

/// The inputs `rels` of the cross part `prog` (key `key`) as the store's frames in `space` keep them: read as
/// `Retained::load` reads a changed input, each owner's rows a range. `None` when one is not kept, or its frames do
/// not match their document table.
pub(in crate::code_rule_dsl) fn kept_inputs(store: &ContentStore, space: [u8; 32], key: [u8; 32], prog: &front::Program, rels: &[String]) -> Result<Option<HashMap<String, KeptInput>>> {
    let _span = crate::code_rule_dsl::logic::profile::span(|| "the kept inputs, from the frames".into());
    let names: Vec<(String, FrameRel)> = rels.iter().map(|rel| (rel.clone(), FrameRel { name: rel.clone(), ident: input_ident(rel, prog, &key) })).collect();
    let all: Vec<FrameRel> = names.iter().map(|(_, f)| f.clone()).collect();
    let stored = store.frames(&space, &all, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
    if all.iter().any(|f| !stored.contains_key(f)) {
        return Ok(None);
    }
    let admitted = store.admit_frames(&space, &all, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
    let mut docs_of_rel: HashMap<String, (Arc<DocDigests>, u64)> = HashMap::new();
    for (rel, frame) in &names {
        let Some(got) = admitted.get(frame).and_then(|a| a.rows().ok()) else { return Ok(None) };
        if got.types.len() != 3 || got.rows.len() != got.n * 3 {
            return Ok(None);
        }
        let docs: DocDigests = got.rows.chunks_exact(3).map(|r| (r[0], (r[1] as u64, r[2] as u64))).collect();
        let at = &stored[frame];
        if docs.len() != got.n || digest_of_docs(&docs) != at.digest {
            return Ok(None);
        }
        docs_of_rel.insert(rel.clone(), (Arc::new(docs), at.digest));
    }
    let keys: Vec<(FrameRel, i64)> = names
        .iter()
        .flat_map(|(rel, frame)| docs_of_rel[rel].0.keys().map(|d| granule(*d)).collect::<BTreeSet<_>>().into_iter().map(move |g| (frame.clone(), g)))
        .collect();
    let parts = store.admit_frame_parts(&space, &keys).map_err(|e| Error::Plan(e.to_string()))?;
    let mut out = HashMap::new();
    for (rel, frame) in &names {
        let Some(decl) = prog.decls.get(rel) else { return Ok(None) };
        let width = decl.len();
        let g = group_col(prog, rel).min(width.max(1) - 1);
        let mut columns: Vec<Vec<i64>> = vec![Vec::new(); width];
        let (docs, digest) = &docs_of_rel[rel];
        // A granule's rows are its owners', each owner's together, owners ascending (`write_frames`): each owner's
        // range follows from the owners' row counts. Checked at each range's ends; an owner's rows are hashed
        // again only when an edit changes them.
        let mut owners: Vec<(i64, i64, u64)> = docs.iter().map(|(d, (_, n))| (granule(*d), *d, *n)).collect();
        owners.sort_unstable();
        let mut ranges = Ranges::default();
        let mut at = 0usize;
        for run in owners.chunk_by(|a, b| a.0 == b.0) {
            let Some(got) = parts.get(&(frame.clone(), run[0].0)).and_then(|a| a.rows().ok()) else { return Ok(None) };
            if got.types.len() != width || got.rows.len() != got.n * width || run.iter().map(|o| o.2 as usize).sum::<usize>() != got.n {
                return Ok(None);
            }
            let mut start = 0usize;
            for &(_, owner, n) in run {
                let n = n as usize;
                if width > 0 && n > 0 && [start, start + n - 1].iter().any(|r| owner_of(&got.rows[r * width..(r + 1) * width], g) != owner) {
                    return Ok(None);
                }
                ranges.insert(owner, ((at + start) as u32, n as u32));
                start += n;
            }
            for row in got.rows.chunks_exact(width.max(1)).take(got.n) {
                for (c, v) in row.iter().enumerate() {
                    columns[c].push(*v);
                }
            }
            at += got.n;
        }
        let source = crate::code_rule_dsl::logic::native::component::columns_source(decl.iter().map(|(n, _)| n.clone()).collect(), columns.into_iter().map(Some).collect());
        out.insert(rel.clone(), KeptInput { source, docs: Arc::clone(docs), digest: *digest, ranges: Arc::new(ranges) });
    }
    Ok(Some(out))
}

/// `source` with its columns named `names` (as the run binds it), its rows as they are.
pub(in crate::code_rule_dsl) fn renamed(source: Arc<dyn Source>, names: Vec<String>) -> Arc<dyn Source> {
    Arc::new(Renamed(source, names))
}

/// A row's hash, as `digest` and `document_delta` take it.
pub(in crate::code_rule_dsl) fn row_hash(row: &[i64]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &v in row {
        h = (h ^ v as u64).wrapping_mul(0x0000_0100_0000_01b3);
        h ^= h >> 29;
    }
    h.wrapping_mul(0x9e37_79b9_7f4a_7c15)
}

fn granule(doc: i64) -> i64 {
    1 + doc.rem_euclid(GRANULES)
}

impl Retained {
    /// The part of program `prog` (key `key`) as the store's frames in `space` hold it (t33 K4): what a session
    /// keeps in memory, kept across processes. A derived relation is taken when its frame is current: each relation
    /// it read has the digest it had then, and a derived read is taken too. So the relations taken are the rows one
    /// run left, whichever programs ran in between. Their rows are read when the run needs them (`load`).
    pub(in crate::code_rule_dsl) fn from_frames(store: &ContentStore, space: [u8; 32], key: [u8; 32], prog: &front::Program, bound: &HashMap<String, u64>) -> Result<Retained> {
        let (sccs, _) = front::strata(prog).map_err(|e| Error::Plan(e.to_string()))?;
        let by_stratum: Vec<Vec<&front::Rule>> = front::rules_by_stratum(prog, &sccs);
        let idents = crate::code_rule_dsl::logic::plan::identities(prog, &sccs, &by_stratum);
        let derived: BTreeSet<&String> = sccs.iter().flatten().collect();
        let mut names: HashMap<String, FrameRel> = idents.into_iter().map(|(name, ident)| (name.clone(), FrameRel { name, ident })).collect();
        // The inputs are what the run binds (`bound`, by their digests): the program's, and an earlier phase's export
        // as phase 1's `doc` or the types phase's `types__wgen`, which no rule may name (a pattern reads `node`).
        let read: BTreeSet<String> = prog.inputs.iter().chain(bound.keys()).filter(|r| !derived.contains(r)).cloned().collect();
        for rel in &read {
            names.insert(rel.clone(), FrameRel { name: rel.clone(), ident: input_ident(rel, prog, &key) });
        }
        let all: Vec<FrameRel> = names.values().cloned().collect();
        let stored = store.frames(&space, &all, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
        Self::from_frame_snapshot(store, space, key, prog, &sccs, names, stored, bound)
    }

    fn empty_frames(key: [u8; 32], mut frames: InFrames) -> Self {
        frames.inputs.clear();
        frames.derived.clear();
        Self { key, docs: (Default::default(), [0xff; 32]), frames: Some(frames), ..Default::default() }
    }

    fn from_frame_snapshot(
        store: &ContentStore, space: [u8; 32], key: [u8; 32], prog: &front::Program,
        sccs: &[BTreeSet<String>], names: HashMap<String, FrameRel>, stored: HashMap<FrameRel, FrameStored>, bound: &HashMap<String, u64>,
    ) -> Result<Self> {
        let derived: BTreeSet<&String> = sccs.iter().flatten().collect();
        let mut frames = InFrames { space, names, ..Default::default() };
        // The inputs are what the run binds (`bound`): the program's, and an earlier phase's export (`from_frames`).
        let read: BTreeSet<&String> = prog.inputs.iter().chain(bound.keys()).filter(|r| !derived.contains(r) && frames.names.contains_key(*r)).collect();
        let asked: Vec<_> = read.iter().filter(|r| stored.contains_key(&frames.names[**r])).map(|r| frames.names[*r].clone()).collect();
        let admitted = store.admit_frames(&space, &asked, DOCS_PART).map_err(|e| Error::Plan(e.to_string()))?;
        if !super::kept_strata::admission_matches(&asked, &admitted, |r| stored.get(r)) {
            return Ok(Self::empty_frames(key, frames));
        }
        let mut input_docs = HashMap::new();
        let mut inputs = HashMap::new();
        for frame in &asked {
            let at = &stored[frame];
            let Ok(rows) = admitted[frame].rows() else { return Ok(Self::empty_frames(key, frames)); };
            if rows.types.len() != 3 || rows.rows.len() != rows.n * 3 { return Ok(Self::empty_frames(key, frames)); }
            let docs: DocDigests = rows.rows.chunks_exact(3).map(|r| (r[0], (r[1] as u64, r[2] as u64))).collect();
            // A relation of no columns is its count (`write_frames`): one entry, its digest the input's.
            let counted = prog.decls.get(&frame.name).is_some_and(Vec::is_empty) && rows.n == 1;
            if docs.len() != rows.n || (!counted && digest_of_docs(&docs) != at.digest) { return Ok(Self::empty_frames(key, frames)); }
            let docs = Arc::new(docs);
            inputs.insert(frame.name.clone(), at.digest);
            input_docs.insert(frame.name.clone(), Arc::clone(&docs));
            frames.inputs.insert(frame.name.clone(), (at.digest, docs));
        }
        // In stratum order: a relation is current when each of its reads is as it was when it was derived, in the
        // state the frames hold: an input as its frame has it (its old rows, which a changed stratum is maintained
        // from), or, with no frame, as the run binds it. A relation with no frame current is named by its reads as
        // `write_frames` names it, so a view of unchanged reads leaves its readers current.
        let mut known: HashMap<String, u64> = bound.clone();
        known.extend(frames.inputs.iter().map(|(rel, (d, _))| (rel.clone(), *d)));
        let all_reads = Reads::of(prog);
        for scc in sccs {
            let current = scc.iter().all(|rel| {
                let Some(at) = frames.names.get(rel).and_then(|n| stored.get(n)) else { return false };
                at.reads.iter().all(|(read, d)| read_digest(read, &known, &derived, prog) == Some(*d))
            });
            if current {
                for rel in scc {
                    let at = &stored[&frames.names[rel]];
                    known.insert(rel.to_string(), at.digest);
                    frames.derived.insert(rel.to_string(), at.clone());
                }
            } else {
                let reads: Vec<String> = all_reads.of_scc(scc).into_iter().filter(|r| !scc.contains(r)).collect();
                if let Some(rd) = reads.iter().map(|r| read_digest(r, &known, &derived, prog).map(|d| (r.clone(), d))).collect::<Option<Vec<_>>>() {
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
        Ok(Retained { key, inputs, input_docs, docs: (Default::default(), [0xff; 32]), frames: Some(frames), ..Default::default() })
    }

    /// Whether the last run's rows of `rel` are kept, as a cut takes them: held, or in the frames. Ordinary
    /// outputs are decoded from their retained rows; missing decoded text does not invalidate their derivation.
    /// `finding` retains its existing SQL decoding path, which also reads the current node and document facts.
    pub(in crate::code_rule_dsl) fn holds(&self, rel: &str, prog: &front::Program) -> bool {
        self.rels.contains_key(rel) || (!(rel == "finding" && prog.outputs.iter().any(|o| o == rel)) && self.frames.as_ref().is_some_and(|f| f.derived.contains_key(rel)))
    }

    /// Reads from the frames the rows this run reads of what they hold (t33 K4): the clean relations a stratum that
    /// runs reads, the relations of the strata that run (their old rows, to be maintained from), and the old rows of
    /// the inputs that changed. What no stratum that runs reads stays in the store.
    pub(in crate::code_rule_dsl) fn load(&mut self, store: &ContentStore, cut: &Cut, prog: &front::Program) -> Result<Option<(usize, usize)>> {
        let Some(frames) = self.frames.as_ref() else { return Ok(Some((0, 0))) };
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the kept part's rows, from the frames".into());
        let running: Vec<&front::Rule> = prog.rules.iter().filter(|r| !cut.clean.contains(r.head.rel.as_str())).collect();
        let mut wanted: BTreeSet<String> = running.iter().flat_map(|r| &r.body).filter_map(|l| l.atom()).map(|a| a.rel.to_string()).filter(|r| cut.clean.contains(r.as_str())).collect();
        wanted.extend(running.iter().map(|r| r.head.rel.to_string()));
        // Outputs are consumers too. Admit their raw facts in the same batch as other demanded relations,
        // then decode them at the output boundary without scheduling their producing strata again.
        wanted.extend(prog.outputs.iter().filter(|r| cut.clean.contains(*r)).cloned());
        let wanted: Vec<String> = wanted.into_iter().filter(|r| frames.derived.contains_key(r) && !self.rels.contains_key(r)).collect();
        let inputs: Vec<String> = cut.changed.iter().filter(|r| frames.inputs.contains_key(*r) && !self.input_srcs.contains_key(*r)).cloned().collect();
        let derived_keys: Vec<_> = wanted.iter().map(|r| (frames.names[r].clone(), DOCS_PART)).collect();
        let input_parts: Vec<_> = inputs.iter().map(|r| {
            let granules: BTreeSet<_> = frames.inputs[r].1.keys().map(|d| granule(*d)).collect();
            (r, granules.into_iter().map(|g| (frames.names[r].clone(), g)).collect::<Vec<_>>())
        }).collect();
        let mut keys = derived_keys.clone();
        keys.extend(input_parts.iter().flat_map(|(_, k)| k.iter().cloned()));
        let admitted = store.admit_frame_parts(&frames.space, &keys).map_err(|e| Error::Plan(e.to_string()))?;
        // Prepare all rows before publishing any of them into the retained part. Failed admission is a cache
        // miss, and the caller must recompute its cut before maintenance can run.
        let decoded = (|| -> Option<_> {
            if !super::kept_strata::admission_matches(&derived_keys, &admitted, |(r, _)| frames.derived.get(&r.name)) { return None; }
            let mut rels = Vec::new();
            let mut sources = Vec::new();
            let mut rows = 0;
            for (rel, key) in wanted.iter().zip(&derived_keys) {
                let got = admitted.get(key)?.rows().ok()?;
                let width = prog.decls.get(rel)?.len();
                if got.types.len() != width { return None; }
                rows += got.n;
                rels.push((rel.clone(), Kept { rel: InRel::from_flat(got.rows, got.n, got.types), width, output: None }));
            }
            for (rel, keys) in &input_parts {
                let width = prog.decls.get(*rel)?.len();
                let mut columns: Vec<Vec<i64>> = vec![Vec::new(); width];
                for key in keys {
                    let got = admitted.get(key)?.rows().ok()?;
                    if got.types.len() != width || got.rows.len() != got.n * width { return None; }
                    for row in got.rows.chunks_exact(width.max(1)).take(got.n) {
                        for (c, v) in row.iter().enumerate() { columns[c].push(*v); }
                    }
                }
                // Named as the program declares them, as the run's own rows are: the session's input delta by
                // document (`document_delta`) takes an old and a new source of the same columns, and with other names
                // every input fell back to the diff by rules (`r__old, !r`), a scan per row (192 s on 20 files).
                let names = prog.decls.get(*rel)?.iter().map(|(n, _)| n.clone()).collect();
                let source = crate::code_rule_dsl::logic::native::component::columns_source(names, columns.into_iter().map(Some).collect());
                // Granule headers carry no digest. Their rows must match the admitted per-document digests,
                // including documents removed or replaced between the header and payload reads.
                if docs_of(source.as_ref(), group_col(prog, rel)) != *frames.inputs[*rel].1 { return None; }
                rows += source.len();
                sources.push(((*rel).clone(), source));
            }
            Some((rels, sources, rows))
        })();
        let Some((rels, sources, rows)) = decoded else {
            let frames = self.frames.take().expect("admitting frames");
            *self = Self::empty_frames(self.key, frames);
            crate::code_rule_dsl::logic::profile::decision("fact admission", || "frames changed during admission; recompute the cut and derive again".into());
            return Ok(None);
        };
        let count = rels.len() + sources.len();
        self.rels.extend(rels);
        self.input_srcs.extend(sources);
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
    pub(in crate::code_rule_dsl) fn read_input(&self, store: &ContentStore, prog: &front::Program, rel: &str, docs: Option<&dyn Fn(i64) -> bool>) -> Result<Option<Vec<Vec<i64>>>> {
        let width = prog.decls.get(rel).map_or(0, Vec::len);
        let g = group_col(prog, rel);
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
            return Ok(Some(out));
        }
        let Some(frames) = self.frames.as_ref() else { return Ok(None) };
        let Some((_, by_doc)) = frames.inputs.get(rel) else { return Ok(None) };
        if width == 0 {
            return Ok(Some(vec![Vec::new(); by_doc.values().map(|(_, n)| *n as usize).sum()]));
        }
        let granules: BTreeSet<i64> = by_doc.keys().filter(|d| docs.is_none_or(|f| f(**d))).map(|d| granule(*d)).collect();
        let _span = crate::code_rule_dsl::logic::profile::span(|| format!("{rel}: {} granules, from the frames", granules.len()));
        let mut out = Vec::new();
        for g in granules {
            let Some(got) = store.frame_rows(&frames.space, &frames.names[rel], g).map_err(|e| Error::Plan(e.to_string()))? else {
                return Err(Error::Plan(format!("{rel}: granule {g} of its stored rows is missing")));
            };
            out.extend(got.rows.chunks_exact(width.max(1)).take(got.n).filter(|r| take(r)).map(<[i64]>::to_vec));
        }
        Ok(Some(out))
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
    pub(in crate::code_rule_dsl) fn write_frames(&self, store: &ContentStore, was: &InFrames, prog: &front::Program, cut: &Cut, inputs: &[(String, Arc<dyn Source>)], digests: &HashMap<String, u64>) -> Result<usize> {
        let _span = crate::code_rule_dsl::logic::profile::span(|| "the kept part's changes, to the frames".into());
        let (sccs, _) = front::strata(prog).map_err(|e| Error::Plan(e.to_string()))?;
        // Literal-only leaves are part of the derived relation's identity (leaf_identity), not runtime
        // inputs requiring a frame digest. Without this, a read of gproto("Generic") or an env fact makes
        // publication skip its consumer and forces that consumer to run in every new process.
        let produced = crate::code_rule_dsl::logic::answers::produced(prog);
        let dynamic: BTreeSet<&str> = prog.inputs.iter().chain(sccs.iter().flatten()).chain(prog.natives.keys())
            .map(String::as_str).chain(inputs.iter().map(|(r,_)|r.as_str())).chain(produced.keys().copied()).collect();
        let literals: BTreeSet<&str> = prog.facts.keys().map(String::as_str).filter(|r| !dynamic.contains(r)).collect();
        let mut puts: Vec<FramePut> = Vec::new();
        let mut now: HashMap<String, u64> = HashMap::new();
        let mut written = 0;
        // Inputs first: their digests are what the relations derived from them record.
        for (rel, source) in inputs {
            let Some(name) = was.names.get(rel) else { continue };
            let width = source.columns().len();
            let digest = digests.get(rel).copied().unwrap_or_else(|| digest(source.as_ref()));
            now.insert(rel.clone(), digest);
            if was.inputs.get(rel).is_some_and(|(d, _)| *d == digest) {
                continue;
            }
            // A relation of no columns is its count: one document's entry, its digest the input's (`digest`).
            if width == 0 {
                let n = source.len() as u64;
                let rows = vec![0, (digest ^ n.wrapping_mul(0xff51_afd7_ed55_8ccd)) as i64, n as i64];
                puts.push(FramePut { rel: name.clone(), part: DOCS_PART, digest, reads: Vec::new(), rows: FrameRows { types: vec![b'i'; 3], n: 1, rows } });
                continue;
            }
            let g = group_col(prog, rel).min(width - 1);
            let docs = self.input_docs.get(rel).cloned().filter(|d| digest_of_docs(d) == digest).unwrap_or_else(|| Arc::new(docs_of(source.as_ref(), g)));
            let old = was.inputs.get(rel).map(|(_, d)| Arc::clone(d));
            // The granules whose documents differ, or that hold a document come or gone.
            let differ = |doc: &i64| old.as_ref().is_none_or(|o| o.get(doc) != docs.get(doc));
            let mut changed: BTreeSet<i64> = docs.keys().filter(|d| differ(d)).map(|d| granule(*d)).collect();
            if let Some(o) = &old {
                changed.extend(o.keys().filter(|d| !docs.contains_key(*d)).map(|d| granule(*d)));
            }
            let types = types_of(prog, rel);
            // Each changed granule's rows, grouped by owner, owners ascending (`kept_inputs` reads each owner's
            // rows as one range). A maintained input (`Layered`) gives each owner's rows from its ranges; another
            // is read through once.
            let mut by: BTreeMap<i64, BTreeMap<i64, Vec<i64>>> = changed.iter().map(|g| (*g, BTreeMap::new())).collect();
            match source.as_any().and_then(|a| a.downcast_ref::<Layered>()) {
                Some(l) => {
                    for owner in docs.keys().filter(|d| changed.contains(&granule(**d))) {
                        by.get_mut(&granule(*owner)).expect("a changed granule").insert(*owner, l.owner_rows(*owner));
                    }
                }
                None => {
                    let all: Vec<usize> = (0..width).collect();
                    let mut row = vec![0i64; width];
                    for r in 0..source.len() {
                        source.row_at(r, &all[g..=g], &mut row[g..=g]);
                        let owner = super::memo::document_of(row[g]);
                        if let Some(out) = by.get_mut(&granule(owner)) {
                            source.row_at(r, &all, &mut row);
                            out.entry(owner).or_default().extend_from_slice(&row);
                        }
                    }
                }
            }
            for (g, owners) in by {
                let rows: Vec<i64> = owners.into_values().flatten().collect();
                let n = rows.len() / width;
                written += n;
                puts.push(FramePut { rel: name.clone(), part: g, digest: 0, reads: Vec::new(), rows: FrameRows { types: types.clone(), rows, n } });
            }
            let mut table: Vec<(i64, (u64, u64))> = docs.iter().map(|(d, v)| (*d, *v)).collect();
            table.sort_unstable();
            let rows: Vec<i64> = table.iter().flat_map(|(d, (s, n))| [*d, *s as i64, *n as i64]).collect();
            puts.push(FramePut { rel: name.clone(), part: DOCS_PART, digest, reads: Vec::new(), rows: FrameRows { types: vec![b'i'; 3], n: table.len(), rows } });
        }
        // Every input the run bound, framed or not (an option's `env`, an earlier phase's export): its digest is
        // what a relation derived from it records.
        for (rel, d) in digests {
            now.entry(rel.clone()).or_insert(*d);
        }
        let derived: BTreeSet<&String> = sccs.iter().flatten().collect();
        let all_reads = Reads::of(prog);
        let trace = std::env::var_os("RERAIL_STATS_KEEP_TRACE").is_some();
        let mut tally = [(0usize, 0usize); 2];
        for scc in &sccs {
            let reads: Vec<String> = all_reads.of_scc(scc).into_iter().filter(|r| !scc.contains(r) && !literals.contains(r.as_str())).collect();
            for rel in scc {
                let Some(name) = was.names.get(rel) else { continue };
                if cut.clean.contains(rel) {
                    if let Some(stored) = was.derived.get(rel) {
                        now.insert(rel.clone(), stored.digest);
                    }
                    continue;
                }
                let read_digests = reads.iter().map(|r| read_digest(r, &now, &derived, prog).map(|d| (r.clone(), d))).collect::<Option<Vec<_>>>();
                // A relation with no rows to keep (a view, one past the budget) is named by its reads' digests:
                // derived again from the same reads, it has the same rows, so its readers' frames stay current.
                let Some(rows) = self.rels.get(rel).and_then(|k| k.rel.live_rows().map(|rows| (k, rows))) else {
                    if let Some(rd) = &read_digests {
                        now.insert(rel.clone(), unkept_digest(rel, rd));
                    }
                    continue;
                };
                let (k, rows) = rows;
                let n = k.rel.rows();
                let d = crate::code_rule_dsl::logic::plan::rows_digest(k.width, &rows, n);
                now.insert(rel.clone(), d);
                let Some(read_digests) = read_digests else { continue };
                if was.derived.get(rel).is_some_and(|s| s.digest == d && s.reads == read_digests) {
                    continue;
                }
                if trace {
                    let i = usize::from(was.derived.get(rel).is_some_and(|s| s.digest == d));
                    tally[i].0 += 1;
                    tally[i].1 += n;
                }
                written += n;
                puts.push(FramePut { rel: name.clone(), part: DOCS_PART, digest: d, reads: read_digests, rows: FrameRows { types: k.rel.types().to_vec(), rows, n } });
            }
        }
        if trace {
            eprintln!("rerail: derived frames written: {} relations ({} rows) whose rows changed; {} ({} rows) whose rows are the same and whose reads changed", tally[0].0, tally[0].1, tally[1].0, tally[1].1);
        }
        store.queue_frames(was.space, puts, Vec::new(), Some((was.names.values().cloned().collect(), super::run_token())));
        Ok(written)
    }
}

#[cfg(test)]
pub mod frame_tests;
