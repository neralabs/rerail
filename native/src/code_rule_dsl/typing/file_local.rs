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
    let mut dependents = vec![Vec::new(); n];
    let mut sources = vec![Vec::new(); n];
    for &(cell, source) in &system.copies {
        dependents[source as usize].push(cell);
        sources[cell as usize].push(source);
    }
    let join = |a: i64, b: i64| if a == BOT { b } else if b == BOT || a == b { a } else { TOP };
    let mut values = system.seeds.clone();
    let mut work: Vec<u32> = (0..n as u32).filter(|&c| values[c as usize] != BOT).collect();
    while let Some(cell) = work.pop() {
        for &dependent in &dependents[cell as usize] {
            let d = dependent as usize;
            let value = sources[d].iter().fold(system.seeds[d], |v, &s| join(v, values[s as usize]));
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

/// Per set, per node of `trees`: its nearest strict ancestor whose kind is in the set, or `NONE`.
pub(crate) fn nearest(trees: &Trees<'_>, sets: &[Vec<i64>], binding: Binding) -> StoreResult<Vec<Vec<u32>>> {
    let n = trees.parent.len();
    if n == 0 || sets.is_empty() {
        return Ok(vec![Vec::new(); sets.len()]);
    }
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
