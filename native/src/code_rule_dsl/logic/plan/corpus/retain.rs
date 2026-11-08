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
//! them again; it reads the old rows of what changed, so inputs' rows are kept too.

use super::{Error, Result, lower};
use crate::code_rule_dsl::facts::vtab::Source;
use crate::code_rule_dsl::logic::front;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// A relation as the last run derived it: its columns, and its decoded rows when it is an output.
#[derive(Clone)]
struct Kept {
    columns: Vec<Vec<i64>>,
    output: Option<(Vec<String>, Vec<usize>)>,
}

impl Kept {
    fn bytes(&self) -> usize {
        self.columns.iter().map(|c| c.len() * 8).sum::<usize>() + self.output.as_ref().map_or(0, |(lines, _)| lines.iter().map(String::len).sum())
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
    /// Each input's rows, as columns, for its deltas next run (stage 2).
    input_rows: HashMap<String, Vec<Vec<i64>>>,
}

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

/// The relations each stratum's rules read (atoms, aggregates' atoms and a native operator's inputs).
fn reads(prog: &front::Program, scc: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = prog
        .rules
        .iter()
        .filter(|r| scc.contains(&r.head.rel))
        .flat_map(|r| &r.body)
        .filter_map(|l| l.atom().map(|a| a.rel.clone()))
        .collect();
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
    // A program with gates keeps their relations to its end and decides on them together: it runs whole.
    let whole = kept.is_none() || !prog.gates.is_empty();
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
        let read: BTreeSet<String> = program.rules.iter().flat_map(|r| &r.body).filter_map(|l| l.atom().map(|a| a.rel.clone())).collect();
        for rel in self.clean.iter().filter(|r| read.contains(*r)) {
            if !program.inputs.contains(rel) {
                program.inputs.push(rel.clone());
            }
        }
        program.outputs.retain(|o| !self.clean.contains(o));
        lower::Lowered { program, domains: cross.domains.clone(), ops: cross.ops, dialect: cross.dialect }
    }

    /// The clean relations `program` reads, as sources over what was kept: bound by the caller, as the other inputs.
    pub(in crate::code_rule_dsl) fn sources(&self, kept: &Retained, program: &front::Program) -> Vec<(String, Arc<dyn Source>)> {
        self.clean
            .iter()
            .filter(|rel| program.inputs.contains(*rel))
            .filter_map(|rel| {
                let k = kept.rels.get(rel)?;
                let names = (0..k.columns.len()).map(|c| crate::code_rule_dsl::logic::plan::sql_col(rel, c)).collect();
                Some((rel.clone(), crate::code_rule_dsl::logic::native::component::columns_source(names, k.columns.iter().cloned().map(Some).collect())))
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
fn budget() -> usize {
    std::env::var("RERAIL_CROSS_RETAIN_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(256) << 20
}

/// What the next run keeps: the clean relations as they were, and those this run derived (`rows` of each, with its
/// decoded `outputs`), in stratum order while they fit the budget.
pub(in crate::code_rule_dsl) fn after(
    old: Option<Retained>,
    key: [u8; 32],
    inputs: HashMap<String, u64>,
    cut: &Cut,
    derived: Vec<(String, usize, Vec<Vec<i64>>)>,
    outputs: &BTreeMap<String, (Vec<String>, Vec<usize>)>,
    sources: &[(String, Arc<dyn Source>)],
) -> Retained {
    let mut rels: HashMap<String, Kept> = HashMap::new();
    let mut old_inputs = HashMap::new();
    if let Some(old) = old.filter(|o| o.key == key) {
        for (rel, k) in old.rels {
            if cut.clean.contains(&rel) {
                rels.insert(rel, k);
            }
        }
        old_inputs = old.input_rows;
    }
    let mut bytes: usize = rels.values().map(Kept::bytes).sum();
    let budget = budget();
    for (rel, width, rows) in derived {
        let mut columns = vec![Vec::with_capacity(rows.len()); width];
        for row in rows {
            for (c, v) in row.into_iter().enumerate() {
                columns[c].push(v);
            }
        }
        let k = Kept { columns, output: outputs.get(&rel).cloned() };
        bytes += k.bytes();
        if bytes > budget {
            break;
        }
        rels.insert(rel, k);
    }
    // Inputs' rows after the relations: an input past the budget derives its readers again when it changes. An
    // unchanged input's rows are the ones kept.
    let mut input_rows = HashMap::new();
    for (rel, source) in sources {
        let width = source.columns().len();
        if width == 0 || input_rows.contains_key(rel) {
            continue;
        }
        let columns = match old_inputs.remove(rel) {
            Some(columns) if !cut.changed.contains(rel) => columns,
            _ => (0..width).map(|c| (0..source.len()).map(|i| source.value(i, c)).collect()).collect(),
        };
        bytes += columns.iter().map(|c: &Vec<i64>| c.len() * 8).sum::<usize>();
        if bytes > budget {
            break;
        }
        input_rows.insert(rel.clone(), columns);
    }
    Retained { key, inputs, rels, input_rows }
}

impl Retained {
    /// A relation's or an input's rows as the last run had them.
    pub(in crate::code_rule_dsl) fn old(&self, rel: &str) -> Option<&Vec<Vec<i64>>> {
        self.rels.get(rel).map(|k| &k.columns).or_else(|| self.input_rows.get(rel))
    }
    pub(in crate::code_rule_dsl) fn relations(&self) -> usize {
        self.rels.len()
    }
    pub(in crate::code_rule_dsl) fn bytes(&self) -> usize {
        self.rels.values().map(Kept::bytes).sum()
    }
}
