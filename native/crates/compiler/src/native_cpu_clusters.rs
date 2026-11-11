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
    /// `SPAN`'s common case inline: the token's end in the mask's current word, found in the
    /// word itself; the helper only searches past it.
    pub inline_span: bool,
    /// The held fields (`held`, at most four) passed from cluster to cluster as the tail
    /// call's arguments, in registers, instead of through `ctx`.
    pub pass: Vec<u32>,
    /// Cranelift's optimizer on (`opt_level = speed`).
    pub speed: bool,
    /// How a switch picks its arm, and the profile it is weighted by (`traffic`).
    pub pick: Pick,
    /// The profile switches and entries are weighted by (`traffic`). With it, a function with
    /// several entries picks one by compares, the hottest first, not one indirect branch.
    pub pick_traffic: HashMap<(u32, u32), u64>,
    /// The functions in one arena, so a crossing is a direct branch.
    pub near: bool,
    /// Return addresses not signed.
    pub unsigned: bool,
    /// A call of the grammar (`PUSH`) is a native call of its callee's function, and `RETURN`
    /// a native return (`by_rules`): no stack of places, no compare tree over them.
    pub calls: bool,
}

/// How a switch on a byte picks its arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A balanced search over the byte's runs.
    Tree,
    /// A search weighted by the profile: the hottest run first when it has half the weight.
    Weighted,
    /// One indirect branch through a table of the 256 bytes.
    Table,
}

impl Cut {
    /// Every word in one function: tier 2 as it was before clusters.
    pub fn one(words: usize) -> Self {
        Self { of: vec![0; words], count: 1, held: Vec::new(), reload: false, inline_span: false, pass: Vec::new(), speed: false, pick: Pick::Tree, pick_traffic: HashMap::new(), near: false, unsigned: false, calls: false }
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
    Cut { of, count: (bins.len() as u32).max(1), held: Vec::new(), reload: false, inline_span: false, pass: Vec::new(), speed: false, pick: Pick::Tree, pick_traffic: HashMap::new(), near: false, unsigned: false, calls: false }
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

/// One function a token state (docs/parser-generators/28-tier-2-in-clusters.md): each `TOKEN`
/// word and the words it reaches before the next `TOKEN`. A word two states reach is the
/// first's (in order of place); words no state reaches are the first state's.
pub fn by_states(native: &Structure) -> Cut {
    use crate::native_ir::StructuralOperation as Op;
    let n = native.structural.len();
    if n == 0 {
        return Cut::one(0);
    }
    let (at, skipped) = lowered_at(native);
    let pushes = crate::native_cpu_stream::push_targets(native);
    let mut of: Vec<Option<u32>> = vec![None; n];
    let mut count = 0u32;
    for s in (0..n).filter(|&pc| !skipped[pc] && matches!(native.structural[pc].operation, Op::Token { .. })) {
        let c = count;
        count += 1;
        let mut work = vec![s];
        while let Some(pc) = work.pop() {
            if of[pc].is_some() {
                continue;
            }
            of[pc] = Some(c);
            let mut next = crate::native_cpu_stream::successors(native, &pushes, pc);
            if matches!(native.structural[pc].operation, Op::Token { .. }) {
                next.push(pc + 1);
            }
            for t in next {
                if t < n && !skipped[t] {
                    let t = at[t];
                    if of[t].is_none() && !matches!(native.structural[t].operation, Op::Token { .. }) {
                        work.push(t);
                    }
                }
            }
        }
    }
    let count = count.max(1);
    let of: Vec<u32> = (0..n).map(|pc| of[at[pc]].unwrap_or(0)).collect();
    Cut { of, count, held: Vec::new(), reload: false, inline_span: false, pass: Vec::new(), speed: false, pick: Pick::Tree, pick_traffic: HashMap::new(), near: false, unsigned: false, calls: false }
}

/// One function a grammar rule (docs/parser-generators/33-compiled-against-lean-shape.md): the
/// program's rules where it has them (`Words::rules`). A state machine's are found: a
/// rule is what a call (`PUSH`) runs, from the word after it (its callee, past `GOTO`s) to the
/// `RETURN`s it reaches, and the file's start is one too. Inside a rule, a call goes on at its
/// return's place (`Cut::calls`: the callee is called natively).
///
/// Rules share code: the states after a value are reached from every rule that has one. A word
/// more than one rule reaches is a function of its own, entered by a tail call, where the rules
/// that reach it meet; its `RETURN` returns from the rule that tail-called it. So no word is
/// lowered twice, and each function has one entry.
pub fn by_rules(native: &Structure) -> Cut {
    use crate::native_cpu_stream::{callee, rule_successors};
    use std::collections::BTreeSet;
    let n = native.structural.len();
    if n == 0 {
        return Cut { calls: true, ..Cut::one(0) };
    }
    // A program with its rules (`Words::rules`, a function a grammar rule from the grammar):
    // each rule a function, as the program says (the verifier checked only calls cross).
    if !native.rules.is_empty() {
        let of = (0..n).map(|pc| (native.rules.partition_point(|&s| usize::from(s) <= pc) - 1) as u32).collect();
        return Cut { of, count: native.rules.len() as u32, calls: true, ..Cut::one(n) };
    }
    // Else a state machine's rules, found: from each call's callee.
    let mut entries: BTreeSet<usize> = std::iter::once(0)
        .chain((0..n).filter(|&pc| matches!(native.structural[pc].operation, crate::native_ir::StructuralOperation::Push { .. })).map(|pc| callee(native, pc)))
        .filter(|&e| e < n)
        .collect();
    let owners = |entries: &BTreeSet<usize>| -> Vec<Vec<usize>> {
        let mut owners: Vec<Vec<usize>> = vec![Vec::new(); n];
        for &e in entries {
            let mut seen = vec![false; n];
            let mut work = vec![e];
            while let Some(pc) = work.pop() {
                if seen[pc] || (pc != e && entries.contains(&pc)) {
                    continue;
                }
                seen[pc] = true;
                owners[pc].push(e);
                work.extend(rule_successors(native, pc));
            }
        }
        owners
    };
    // Where rules meet: a word whose rules are not those of a word before it.
    let owned = loop {
        let owned = owners(&entries);
        let mut meet = BTreeSet::new();
        for pc in (0..n).filter(|&pc| !owned[pc].is_empty()) {
            for q in rule_successors(native, pc) {
                if owned[q].len() > 1 && owned[q] != owned[pc] {
                    meet.insert(crate::native_cpu_stream::past_gotos(native, q));
                }
            }
        }
        let before = entries.len();
        entries.extend(meet);
        if entries.len() == before {
            break owned;
        }
    };
    let index: HashMap<usize, u32> = entries.iter().enumerate().map(|(i, &e)| (e, i as u32)).collect();
    let mut of: Vec<u32> = (0..n).map(|pc| owned[pc].first().map_or(0, |e| index[e])).collect();
    // A `GOTO` goes with the word it goes to: a jump to it is a jump there.
    for pc in 0..n {
        if matches!(native.structural[pc].operation, crate::native_ir::StructuralOperation::Goto { .. }) {
            let t = crate::native_cpu_stream::past_gotos(native, pc);
            if t < n {
                of[pc] = of[t];
            }
        }
    }
    Cut { of, count: entries.len() as u32, calls: true, ..Cut::one(n) }
}
