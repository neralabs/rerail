//! File-local strata in the transducer (03 §13; 05 Phase 10 row 1, "split file-local strata"; list step
//! 29), the part list step 22 needs: `nearest` (O12, option a, decided 2026-11-03).
//!
//! `nearest($n, $a, set)` holds when `$a` is `$n`'s nearest strict ancestor whose kind is in the set. Tree
//! structure is not a relation (§12), so the strata do not climb the tree: the relation is made per chunk,
//! file-locally, as a flat-lattice system the transducer's file-local FIX solves (list step 17,
//! `transducer::packed::metal_lattice`, on the device work queue), and the strata read its rows as an input
//! (`nearest_<set>(n, a)`, `facts::vtab::syntax::with_nearest`).
//!
//! The system has a cell per (set, node). A node whose parent's kind is in the set is seeded with its parent;
//! any other node with a parent copies its parent's cell; a root has no value. Each cell has one input, so the
//! flat lattice never reaches TOP, and the least fixpoint is each node's nearest ancestor in the set. Every
//! set of the program is one system and one launch. The CPU solves the same system (`solve_cpu`), the
//! reference the device's must equal.

use crate::content_store::{StoreError, StoreResult};

/// No node: a root's parent, or a node with no ancestor in the set.
pub(crate) const NONE: u32 = u32::MAX;
/// The flat lattice's absent and top values (`logic::native::BOT`, `TOP`).
const BOT: i64 = i64::MIN;
const TOP: i64 = 1 << 62;

/// Where the file-local FIX runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Binding {
    Cpu,
    /// The device work queue; the CPU where there is no device.
    Metal,
}

impl Binding {
    pub(crate) fn of(backend: crate::code_rule_dsl::transducer::packed::ExecutionBackend) -> Self {
        use crate::code_rule_dsl::transducer::packed::ExecutionBackend as B;
        match backend {
            B::MetalInterpreter | B::MetalJit => Binding::Metal,
            _ => Binding::Cpu,
        }
    }
}

/// A chunk's trees: per node (a cell of the chunk), its parent's cell or `NONE`, and its kind atom.
pub(crate) struct Trees<'a> {
    pub(crate) parent: &'a [u32],
    pub(crate) kind: &'a [i64],
}

/// The flat-lattice system: seeds, and copies `(cell, source)`.
struct System {
    seeds: Vec<i64>,
    copies: Vec<(u32, u32)>,
}

fn system(trees: &Trees<'_>, sets: &[Vec<i64>]) -> StoreResult<System> {
    let n = trees.parent.len();
    let cells = n.checked_mul(sets.len()).filter(|&c| c < u32::MAX as usize).ok_or_else(|| StoreError::Invalid(format!("nearest: {n} nodes in {} sets", sets.len())))?;
    let mut seeds = vec![BOT; cells];
    let mut copies = Vec::new();
    for (k, set) in sets.iter().enumerate() {
        let base = k * n;
        for (node, &parent) in trees.parent.iter().enumerate() {
            if parent == NONE {
                continue;
            }
            if set.contains(&trees.kind[parent as usize]) {
                seeds[base + node] = i64::from(parent);
            } else {
                copies.push(((base + node) as u32, (base + parent as usize) as u32));
            }
        }
    }
    Ok(System { seeds, copies })
}

/// The system's least fixpoint on the CPU: a cell's value is its seed joined with its sources', evaluated
/// again whenever a source rises.
fn solve_cpu(system: &System) -> Vec<i64> {
    let n = system.seeds.len();
    // Sources and dependents as CSR (a cell has one source here; the solver does not rely on it).
    let csr = |edges: &mut Vec<(u32, u32)>| -> (Vec<u32>, Vec<u32>) {
        edges.sort_unstable();
        let mut start = vec![0_u32; n + 1];
        for &(a, _) in edges.iter() {
            start[a as usize + 1] += 1;
        }
        for at in 0..n {
            start[at + 1] += start[at];
        }
        (start, edges.iter().map(|e| e.1).collect())
    };
    let (source_start, sources) = csr(&mut system.copies.clone());
    let (dependent_start, dependents) = csr(&mut system.copies.iter().map(|&(cell, source)| (source, cell)).collect());
    let join = |a: i64, b: i64| if a == BOT { b } else if b == BOT || a == b { a } else { TOP };
    let mut values = system.seeds.clone();
    let mut work: Vec<u32> = (0..n as u32).filter(|&c| values[c as usize] != BOT).collect();
    while let Some(cell) = work.pop() {
        for &dependent in &dependents[dependent_start[cell as usize] as usize..dependent_start[cell as usize + 1] as usize] {
            let d = dependent as usize;
            let value = sources[source_start[d] as usize..source_start[d + 1] as usize].iter().fold(system.seeds[d], |v, &s| join(v, values[s as usize]));
            if value != values[d] {
                values[d] = value;
                work.push(dependent);
            }
        }
    }
    values
}

#[cfg(target_os = "macos")]
fn solve_device(system: &System) -> StoreResult<Option<Vec<i64>>> {
    use crate::code_rule_dsl::transducer::packed::metal_lattice;
    let device = metal_lattice::System { seeds: &system.seeds, copies: &system.copies, bins: &[] };
    match metal_lattice::solve(&device) {
        Ok(solved) => {
            crate::code_rule_dsl::prof::count("nearest.device_launches", solved.launches);
            Ok(Some(solved.values))
        }
        Err(crate::code_rule_dsl::transducer::packed::Error::UnsupportedBackend) => Ok(None),
        Err(error) => Err(StoreError::Invalid(format!("nearest on the device: {error:?}"))),
    }
}

#[cfg(not(target_os = "macos"))]
fn solve_device(_: &System) -> StoreResult<Option<Vec<i64>>> {
    Ok(None)
}

/// Cells of one system at most: a chunk's (list step 17's queue is sized per launch). Larger batches are split at
/// their documents' roots, a system and a launch each.
const CELLS: usize = 1 << 18;

/// Per set, per node of `trees`: its nearest strict ancestor whose kind is in the set, or `NONE`.
pub(crate) fn nearest(trees: &Trees<'_>, sets: &[Vec<i64>], binding: Binding) -> StoreResult<Vec<Vec<u32>>> {
    let n = trees.parent.len();
    if n == 0 || sets.is_empty() {
        return Ok(vec![Vec::new(); sets.len()]);
    }
    let mut out = vec![Vec::with_capacity(n); sets.len()];
    let per = (CELLS / sets.len()).max(1);
    let mut start = 0;
    while start < n {
        // The next piece ends at a root (a document's first node) once it holds `per` nodes; a tree is never split.
        let mut end = (start + per).min(n);
        while end < n && trees.parent[end] != NONE {
            end += 1;
        }
        let parent: Vec<u32> = trees.parent[start..end].iter().map(|&p| if p == NONE { NONE } else { p - start as u32 }).collect();
        let piece = Trees { parent: &parent, kind: &trees.kind[start..end] };
        for (set, found) in out.iter_mut().zip(solve(&piece, sets, binding)?) {
            set.extend(found.into_iter().map(|a| if a == NONE { NONE } else { a + start as u32 }));
        }
        start = end;
    }
    Ok(out)
}

/// `nearest` of one system.
fn solve(trees: &Trees<'_>, sets: &[Vec<i64>], binding: Binding) -> StoreResult<Vec<Vec<u32>>> {
    let n = trees.parent.len();
    let system = system(trees, sets)?;
    let values = match binding {
        Binding::Metal => solve_device(&system)?.map_or_else(|| solve_cpu(&system), |values| values),
        Binding::Cpu => solve_cpu(&system),
    };
    crate::code_rule_dsl::prof::count("nearest.cells", values.len());
    values
        .chunks(n)
        .map(|set| {
            set.iter()
                .map(|&v| match v {
                    BOT => Ok(NONE),
                    TOP => Err(StoreError::Invalid("nearest: a node with two nearest ancestors".to_owned())),
                    v => Ok(v as u32),
                })
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests;

/// The relations a program derives within one document (list step 29's split, R0): a relation is file-local when
/// each of its rules joins only one document's rows. A rule is when every relation it reads (positive, negated or
/// aggregated) is a base relation, a constant one, or file-local, each with a location (a node, or a document) of the
/// rule's one component: locations are joined within an atom of a base or file-local relation (`node(d, n, …)`,
/// `child(p, c, _)`, `nearest_<set>(n, a)`), and nothing else joins them. A symbol shared by two atoms (`text($a,
/// $x), text($b, $x)`) does not: those may be two files. The relations are the greatest set closed under this, so a
/// recursion within a document is file-local. Evaluated per chunk of documents, a file-local relation's rows are the
/// union of the chunks'.
pub(crate) fn split(prog: &crate::code_rule_dsl::logic::front::Program) -> std::collections::BTreeSet<String> {
    use crate::code_rule_dsl::logic::front::{Literal, Term};
    use std::collections::{BTreeMap, BTreeSet};
    let derived: BTreeSet<&str> = prog.rules.iter().map(|r| r.head.rel.as_str()).collect();
    // A base relation's location columns: node columns, and the document column of `node`, `doc`, `pkg` and
    // `syntax_error`.
    let base_locations = |rel: &str| -> Vec<usize> {
        let mut columns: Vec<usize> = prog.decls.get(rel).map_or_else(Vec::new, |cols| cols.iter().enumerate().filter(|(_, (_, t))| t == "node").map(|(i, _)| i).collect());
        if matches!(rel, "node" | "doc" | "pkg" | "syntax_error") {
            columns.push(0);
        }
        columns
    };
    // A derived relation's location columns: those every rule fills from a location.
    let mut locations: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let location_vars = |rule: &crate::code_rule_dsl::logic::front::Rule, locations: &BTreeMap<String, Vec<usize>>| -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for literal in &rule.body {
            if let Some(a) = literal.atom().filter(|a| !a.neg) {
                let columns = if derived.contains(a.rel.as_str()) { locations.get(&a.rel).cloned().unwrap_or_default() } else { base_locations(&a.rel) };
                for c in columns {
                    if let Some(Term::Var(v)) = a.args.get(c) {
                        out.insert(v.clone());
                    }
                }
            }
        }
        out
    };
    // Location columns to a fixpoint (a column is one if every rule's head fills it from a location variable).
    let mut changed = true;
    for rel in &derived {
        let width = prog.decls.get(*rel).map_or(0, Vec::len);
        locations.insert((*rel).to_owned(), (0..width).collect());
    }
    while changed {
        changed = false;
        for rel in &derived {
            let rules: Vec<_> = prog.rules.iter().filter(|r| r.head.rel == *rel).collect();
            let kept: Vec<usize> = locations[*rel]
                .iter()
                .copied()
                .filter(|&c| rules.iter().all(|r| matches!(r.head.args.get(c), Some(Term::Var(v)) if location_vars(r, &locations).contains(v))))
                .collect();
            if kept != locations[*rel] {
                locations.insert((*rel).to_owned(), kept);
                changed = true;
            }
        }
    }
    // String filters (decision 0037): a text test's relation (`strtest_<k>`), and a derived relation whose every
    // rule reads only text relations and such filters, each head string from a text column. A chunk's rows hold
    // every string of the chunk that passes, so a rule that joins one on a string its chunk gave stays file-local.
    let text_columns = |rel: &str| crate::code_rule_dsl::logic::strings::TEXT_COLUMNS.iter().find(|(r, _)| *r == rel).map(|(_, c)| *c);
    let test_relation = |rel: &str| rel.starts_with(crate::code_rule_dsl::logic::strings::RELATION);
    // To a fixpoint: a filter may read other filters.
    let mut filters: BTreeSet<String> = derived.iter().map(|r| (*r).to_owned()).collect();
    loop {
        let kept: BTreeSet<String> = filters
            .iter()
            .filter(|rel| {
                prog.rules.iter().filter(|r| &r.head.rel == *rel).all(|r| {
                    let mut strings = BTreeSet::new();
                    let reads_only_text = r.body.iter().all(|l| match l {
                        Literal::Atom(a) if test_relation(&a.rel) || filters.contains(&a.rel) => true,
                        Literal::Atom(a) if !a.neg && let Some(cols) = text_columns(&a.rel) => {
                            strings.extend(cols.iter().filter_map(|&c| if let Some(Term::Var(v)) = a.args.get(c) { Some(v.clone()) } else { None }));
                            true
                        }
                        Literal::Cmp { .. } => true,
                        _ => false,
                    });
                    reads_only_text && r.head.args.iter().all(|t| matches!(t, Term::Var(v) if strings.contains(v)))
                })
            })
            .cloned()
            .collect();
        if kept == filters {
            break;
        }
        filters = kept;
    }
    let mut local: BTreeSet<String> = derived.iter().map(|r| (*r).to_owned()).collect();
    loop {
        let rule_local = |rule: &crate::code_rule_dsl::logic::front::Rule| -> bool {
            // Each read relation's location variables; the rule's locations must form one component.
            let mut groups: Vec<BTreeSet<String>> = Vec::new();
            for literal in &rule.body {
                let atom = match literal {
                    Literal::Atom(a) | Literal::Agg { atom: a, .. } => a,
                    Literal::Ast { .. } => return false,
                    Literal::Nearest { n, a, .. } => {
                        groups.push(BTreeSet::from([n.clone(), a.clone()]));
                        continue;
                    }
                    Literal::Cmp { .. } => continue,
                };
                let constant = !derived.contains(atom.rel.as_str()) && prog.facts.contains_key(&atom.rel) && !prog.inputs.contains(&atom.rel);
                if constant || test_relation(&atom.rel) || filters.contains(&atom.rel) || filters.contains(&rule.head.rel) {
                    continue;
                }
                if derived.contains(atom.rel.as_str()) && !local.contains(&atom.rel) {
                    return false;
                }
                let columns = if derived.contains(atom.rel.as_str()) { locations[&atom.rel].clone() } else { base_locations(&atom.rel) };
                let vars: BTreeSet<String> = columns.iter().filter_map(|&c| if let Some(Term::Var(v)) = atom.args.get(c) { Some(v.clone()) } else { None }).collect();
                if vars.is_empty() {
                    // A read with no location variable (`field(_, "left", _)`'s rows, a count over them): any file's.
                    return false;
                }
                groups.push(vars);
            }
            // Merge groups sharing a variable; one group left is one document.
            let mut merged: Vec<BTreeSet<String>> = Vec::new();
            for group in groups {
                let (joined, rest): (Vec<_>, Vec<_>) = merged.into_iter().partition(|m| !m.is_disjoint(&group));
                let mut all = group;
                for j in joined {
                    all.extend(j);
                }
                merged = rest;
                merged.push(all);
            }
            merged.len() <= 1
        };
        let before = local.len();
        let failing: Vec<String> = prog.rules.iter().filter(|r| local.contains(&r.head.rel) && !rule_local(r)).map(|r| r.head.rel.clone()).collect();
        for rel in failing {
            local.remove(&rel);
        }
        if local.len() == before {
            return local;
        }
    }
}

/// A program split at its file-local relations (`split`): `local` derives them, run per chunk of documents over the
/// chunk's base relations; `cross` derives the rest, run once over what the chunks wrote: `exports` (the file-local
/// relations a cross-file rule reads) and `base` (the base relations a cross-file rule reads, and `node` and `doc`,
/// which decode findings). In `cross`, both are inputs.
pub(crate) struct Programs {
    pub(crate) local: crate::code_rule_dsl::logic::front::Program,
    pub(crate) cross: crate::code_rule_dsl::logic::front::Program,
    pub(crate) exports: Vec<String>,
    pub(crate) base: Vec<String>,
}

pub(crate) fn programs(prog: &crate::code_rule_dsl::logic::front::Program) -> Programs {
    use crate::code_rule_dsl::logic::front::Literal;
    use std::collections::BTreeSet;
    let local = split(prog);
    let is_local = |rel: &str| local.contains(rel);
    let read = |rules: &mut dyn Iterator<Item = &crate::code_rule_dsl::logic::front::Rule>| -> BTreeSet<String> {
        rules
            .flat_map(|r| &r.body)
            .filter_map(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a.rel.clone()),
                _ => None,
            })
            .collect()
    };
    let cross_reads = read(&mut prog.rules.iter().filter(|r| !is_local(&r.head.rel)));
    // A file-local relation that only projects a base input (each rule one positive input atom: `kind` of `node`)
    // is not exported: the cross-file part derives it from the stored input (a view), not from a copy of it.
    let projection = |rel: &str| {
        let mut rules = prog.rules.iter().filter(|r| r.head.rel == rel).peekable();
        rules.peek().is_some()
            && !prog.lattice.contains_key(rel)
            && rules.all(|r| matches!(r.body.as_slice(), [Literal::Atom(a)] if !a.neg && prog.inputs.contains(&a.rel)))
    };
    let projected: BTreeSet<String> = cross_reads.iter().filter(|r| is_local(r) && projection(r)).cloned().collect();
    let in_cross = |rel: &str| !is_local(rel) || projected.contains(rel);
    let cross_reads = read(&mut prog.rules.iter().filter(|r| in_cross(&r.head.rel)));
    let exports: Vec<String> = cross_reads.iter().filter(|r| is_local(r) && !projected.contains(*r)).cloned().collect();
    let base_read: Vec<String> = prog.inputs.iter().filter(|i| cross_reads.contains(*i)).cloned().collect();
    let mut base = base_read.clone();
    for decode in ["node", "doc"] {
        if prog.inputs.iter().any(|i| i == decode) && !base.iter().any(|b| b == decode) {
            base.push(decode.to_owned());
        }
    }
    let mut l = prog.clone();
    l.rules.retain(|r| is_local(&r.head.rel));
    l.lattice.retain(|rel, _| is_local(rel));
    l.gates.retain(|(rel, _)| is_local(rel));
    l.outputs = prog.outputs.iter().filter(|o| is_local(o)).cloned().chain(exports.iter().cloned()).collect::<BTreeSet<_>>().into_iter().collect();
    let mut c = prog.clone();
    c.rules.retain(|r| in_cross(&r.head.rel));
    c.lattice.retain(|rel, _| !is_local(rel));
    c.gates.retain(|(rel, _)| !is_local(rel));
    c.outputs.retain(|o| !is_local(o));
    c.inputs = base_read.into_iter().chain(exports.iter().cloned()).collect();
    Programs { local: l, cross: c, exports, base }
}
