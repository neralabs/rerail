//! Tier 2 in clusters (docs/parser-generators/28-tier-2-in-clusters.md): which words each of
//! tier 2's functions holds.
//!
//! Tier 2 is lowered as one function, or as one function a cluster of words
//! (`native_cpu_stream::compile_cut`). Inside a cluster, a word jumps to the next directly.
//! A jump to a word of another cluster is a tail call of that cluster's function, with the
//! word's place as its entry. So a cluster's registers are allocated over its own words alone,
//! and each function compiles alone.
//!
//! The cut comes from a profile (`traffic`): how often each word ran right after another,
//! from the interpreter over sample files. Words that pass control among themselves the most
//! are merged first, while a cluster stays under a cap in words (`by_traffic`). Words the
//! sample never ran go with their neighbours, then into the clusters with room.

use crate::native_ir::Structure;
use std::collections::HashMap;

/// Each word's cluster (`of`, by place), and the number of clusters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cut {
    pub of: Vec<u32>,
    pub count: u32,
    /// The state's fields (`native_cpu_stream`'s `Mem`, by index) each function holds in
    /// registers: loaded where it is entered, stored where it calls another cluster. Empty: the
    /// whole state in memory.
    pub held: Vec<u32>,
    /// Each word loads the `ctx` fields it reads where it runs, instead of every field living
    /// from the function's entry (`native_cpu_stream::ctx_reads`).
    pub reload: bool,
}

impl Cut {
    /// Every word in one function: tier 2 as it was before clusters.
    pub fn one(words: usize) -> Self {
        Self { of: vec![0; words], count: 1, held: Vec::new(), reload: false }
    }
}

/// How often word `b` ran right after word `a`, by the interpreter over `files`
/// (`stream::run_seen`), each file's tier 1 from the program's first block program.
pub fn traffic(program: &crate::program::Words, files: &[&[u8]]) -> HashMap<(u32, u32), u64> {
    let mut edges: HashMap<(u32, u32), u64> = HashMap::new();
    let Some(block) = program.block_programs.first() else { return edges };
    for &bytes in files {
        let run = block.run(bytes);
        let mut last: Option<u32> = None;
        let _ = crate::stream::run_seen(program, bytes, &run, &mut |pc| {
            let pc = pc as u32;
            if let Some(l) = last {
                *edges.entry((l, pc)).or_default() += 1;
            }
            last = Some(pc);
        });
    }
    edges
}

/// The place each word is lowered at: a word of a shared arm that is not its body's first place
/// (`native_cpu_stream::shared_runs`) runs as the body's word, at the first place.
fn lowered_at(native: &Structure) -> (Vec<usize>, Vec<bool>) {
    let pushes = crate::native_cpu_stream::push_targets(native);
    let shared = crate::native_cpu_stream::shared_runs(native, &pushes);
    let n = native.structural.len();
    let mut at: Vec<usize> = (0..n).collect();
    for (&s, &(b, _)) in &shared.at {
        let body = &shared.bodies[b];
        if body.start == s {
            continue;
        }
        for k in 1..=body.end - body.start {
            if s + k < n {
                at[s + k] = body.start + k;
            }
        }
    }
    (at, shared.skipped)
}

/// Clusters of at most `cap` lowered words, cut by `traffic` (`traffic`'s places).
///
/// - **Hot words first.** The pairs of words with the most traffic between them merge first,
///   while the merged cluster stays under `cap`.
/// - **Then the program's own edges**, in order of place: a word the sample never ran joins the
///   cluster of a word next to it.
/// - **Then packing.** The clusters left are packed into as few as hold them, the largest
///   first: clusters with no traffic between them lose nothing by sharing a function.
pub fn by_traffic(native: &Structure, traffic: &HashMap<(u32, u32), u64>, cap: usize) -> Cut {
    let n = native.structural.len();
    if n == 0 {
        return Cut::one(0);
    }
    let (at, skipped) = lowered_at(native);
    let size0: Vec<usize> = (0..n).map(|pc| usize::from(!skipped[pc])).collect();
    let mut parent: Vec<usize> = (0..n).collect();
    let mut size = size0.clone();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let union = |parent: &mut Vec<usize>, size: &mut Vec<usize>, a: usize, b: usize| {
        let (ra, rb) = (find(parent, a), find(parent, b));
        if ra != rb && size[ra] + size[rb] <= cap {
            parent[rb] = ra;
            size[ra] += size[rb];
        }
    };
    // The traffic between lowered places, both ways summed.
    let mut pairs: HashMap<(usize, usize), u64> = HashMap::new();
    for (&(a, b), &w) in traffic {
        let (a, b) = (a as usize, b as usize);
        if a >= n || b >= n {
            continue;
        }
        let (a, b) = (at[a], at[b]);
        if a != b {
            *pairs.entry((a.min(b), a.max(b))).or_default() += w;
        }
    }
    let mut hot: Vec<((usize, usize), u64)> = pairs.into_iter().collect();
    hot.sort_unstable_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
    for ((a, b), _) in hot {
        union(&mut parent, &mut size, a, b);
    }
    // The program's edges, a word to each it goes on to.
    let pushes = crate::native_cpu_stream::push_targets(native);
    for pc in 0..n {
        if skipped[pc] {
            continue;
        }
        let mut next = crate::native_cpu_stream::successors(native, &pushes, pc);
        if matches!(native.structural[pc].operation, crate::native_ir::StructuralOperation::Token { .. }) && pc + 1 < n {
            next.push(pc + 1);
        }
        for t in next {
            if t < n && !skipped[t] {
                union(&mut parent, &mut size, pc, at[t]);
            }
        }
    }
    // Packing: the roots, largest first, each into the first bin with room.
    let mut roots: Vec<usize> = (0..n).filter(|&pc| !skipped[pc] && find(&mut parent, pc) == pc).collect();
    roots.sort_unstable_by(|&x, &y| size[y].cmp(&size[x]).then(x.cmp(&y)));
    let mut bins: Vec<usize> = Vec::new();
    let mut bin_of: HashMap<usize, u32> = HashMap::new();
    for r in roots {
        let b = match bins.iter().position(|&used| used + size[r] <= cap) {
            Some(b) => b,
            None => {
                bins.push(0);
                bins.len() - 1
            }
        };
        bins[b] += size[r];
        bin_of.insert(r, b as u32);
    }
    let of: Vec<u32> = (0..n)
        .map(|pc| {
            let r = find(&mut parent, at[pc]);
            bin_of.get(&r).copied().unwrap_or(0)
        })
        .collect();
    Cut { of, count: (bins.len() as u32).max(1), held: Vec::new(), reload: false }
}

/// The share of `traffic`'s steps that leave their cluster, of all its steps between two words.
pub fn crossings(native: &Structure, cut: &Cut, traffic: &HashMap<(u32, u32), u64>) -> f64 {
    let (at, _) = lowered_at(native);
    let n = native.structural.len();
    let (mut all, mut out) = (0u64, 0u64);
    for (&(a, b), &w) in traffic {
        let (a, b) = (a as usize, b as usize);
        if a >= n || b >= n {
            continue;
        }
        all += w;
        if cut.of[at[a]] != cut.of[at[b]] {
            out += w;
        }
    }
    if all == 0 { 0.0 } else { out as f64 / all as f64 }
}
