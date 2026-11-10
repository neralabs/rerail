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
pub(in crate::code_rule_dsl) fn document_delta(rel: &str, old: &dyn Source, was: Option<&DocDigests>, new: &dyn Source) -> Option<((Arc<dyn Source>, Arc<dyn Source>), Arc<DocDigests>)> {
    use rustc_hash::FxHashSet;
    let width = new.columns().len();
    if width == 0 || old.columns() != new.columns() {
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
            let d = super::memo::document_of(row[0]);
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
            s.row_at(r, &all[..1], &mut row[..1]);
            if differ.contains(&super::memo::document_of(row[0])) {
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

/// The relations each stratum's rules read (atoms, aggregates' atoms and a native operator's inputs).
fn reads(prog: &front::Program, scc: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = prog
        .rules
        .iter()
        .filter(|r| scc.contains(&r.head.rel))
        .flat_map(|r| &r.body)
        .filter_map(|l| l.atom().map(|a| a.rel.clone()))
        .collect();
    // A `nearest` literal climbs the tree: it reads `node` and `child`, which an edit changes.
    if prog.rules.iter().filter(|r| scc.contains(&r.head.rel)).any(|r| r.body.iter().any(|l| matches!(l, front::Literal::Nearest { .. }))) {
        out.extend(TREE.iter().map(|r| (*r).to_owned()));
    }
    for rel in scc {
        if let Some(native) = prog.natives.get(rel) {
            out.extend(native.reads().into_iter().map(str::to_owned));
        }
    }
    out
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
    let complete = kept.is_some_and(|k| sccs.iter().flatten().all(|r| k.rels.contains_key(r)));
    let whole = kept.is_none() || (!prog.gates.is_empty() && (!changed.is_empty() || !complete));
    let mut ran = 0;
    for scc in &sccs {
        let stale = whole || reads(prog, scc).iter().any(|r| dirty.contains(r));
        if stale {
            ran += 1;
            dirty.extend(scc.iter().cloned());
        } else if !scc.iter().all(|r| kept.is_some_and(|k| k.rels.contains_key(r))) {
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
        program.rules.retain(|r| !self.clean.contains(&r.head.rel));
        program.natives.retain(|rel, _| !self.clean.contains(rel));
        if self.strata.0 == 0 {
            program.gates.clear();
        }
        let read: BTreeSet<String> = program.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom().map(|a| a.rel.clone())).collect();
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
    Retained { key, inputs, rels, input_srcs, measured, input_stats, input_docs, docs, patched }
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
    parts
        .values()
        .find_map(|p| p.input_srcs.get(rel).filter(|s| p.inputs.get(rel) == Some(&d) && s.columns() == columns).cloned())
        .unwrap_or(source)
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
        let mut by_size: Vec<(&String, &Kept)> = self.rels.iter().collect();
        by_size.sort_by_key(|(_, k)| std::cmp::Reverse(k.rel.held_bytes().2));
        for (rel, kept) in by_size {
            let (k, set, chains, csr) = kept.rel.index_detail();
            let n = kept.rel.rows();
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
                out.push(format!("SPIKE grown, all kept: {} offers, {} held; today {:.1} ms {:.1} B/row; mixed {:.1} ms (set {:.1}, placed {:.1}, orders {:.1}) {:.1} B/row", g.2, g.3, g.0, g.4 as f64 / g.3.max(1) as f64, g.1, g.6, g.7, g.8, g.5 as f64 / g.3.max(1) as f64));
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


#[cfg(test)]
mod tests {
    use super::*;

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
        let ((plus, minus), docs) = document_delta("r", old.as_ref(), None, new.as_ref()).unwrap();
        assert_eq!(rows(&plus), BTreeSet::from([vec![node(2, 1), 22], vec![node(4, 0), 40]]));
        assert_eq!(rows(&minus), BTreeSet::from([vec![node(2, 1), 21], vec![node(3, 0), 30]]));
        // The next run: `new` is old, its hashes kept; nothing changed.
        let ((plus, minus), _) = document_delta("r", new.as_ref(), Some(&docs), source(&[[node(1, 0), 10], [node(1, 1), 11], [node(2, 0), 20], [node(2, 1), 22], [node(4, 0), 40]]).as_ref()).unwrap();
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
        let mut kept=Retained {key,inputs:inputs.clone(),rels,input_srcs:Default::default(),measured:Default::default(),input_stats:Default::default(),input_docs:Default::default(),docs:Default::default(),patched:Default::default()};
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
