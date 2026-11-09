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
//! past it, a relation is not kept and its stratum runs again next time.
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
}

/// A relation's statistics as bound (`syntax::statistics`), and its counts of the constants rules test it by.
/// The counts are of the constants tested when they were measured (the third): taken again only for the same.
pub(in crate::code_rule_dsl) type Measured = (Vec<(String, i64, i64, i64, i64)>, Vec<(String, i64, i64)>, BTreeSet<(usize, i64)>);

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
        let runs = whole || reads(prog, scc).iter().any(|r| dirty.contains(r)) || !scc.iter().all(|r| kept.is_some_and(|k| k.rels.contains_key(r)));
        if runs {
            ran += 1;
            dirty.extend(scc.iter().cloned());
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
) -> Retained {
    let mut rels: HashMap<String, Kept> = HashMap::new();
    if let Some(old) = old.filter(|o| o.key == key) {
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
    Retained { key, inputs, rels, input_srcs, measured }
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

    /// Every statistic kept, to be carried to the next run's (`after`).
    pub(in crate::code_rule_dsl) fn measured_all(&self) -> HashMap<(String, usize), Measured> {
        self.measured.clone()
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
        let mut kept=Retained {key,inputs:inputs.clone(),rels,input_srcs:Default::default()};
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
