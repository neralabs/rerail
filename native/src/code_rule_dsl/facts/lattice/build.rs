//! Y2's dead-code product graph enumerated from the inputs Y2 reads: E1's `lattice build`, its
//! enumeration only. The column writer, Y2's timed reachability and the per-worktree layers of
//! E1's dump are not kept: the dead-code producer (`facts::deadcode`) builds and maintains its
//! own CSR from this enumeration.
//!
//! The product graph's states are Y2's `(node, layer)` (`y2::dc`: flow, annotated, dispatch).
//! Its edges are Y2's `succ`, evaluated once per state of the closure of every state that could
//! be reached: the roots, every definition's flow state and its module's. That closure is a
//! superset of the reached set (whatever the roots), so the fixpoint from the roots over it is
//! Y2's reachability. Rows keep `succ`'s repeats; the consumer sorts and drops them.
use crate::code_rule_dsl::facts::y2::dc::{self, Dc, St};
use crate::code_rule_dsl::facts::y2::facts::Facts;
use rustc_hash::FxHashSet;

/// One worktree's product graph: states in discovery order, their `succ` rows (as `St`, with
/// repeats), and the roots.
pub(crate) struct Enumerated {
    pub states: Vec<St>,
    pub off: Vec<usize>,
    pub tgt: Vec<St>,
    pub seeds: Vec<St>,
}

pub(crate) fn enumerate(f: &Facts) -> Enumerated {
    let mut seeds = Dc::root_states(f);
    seeds.sort_unstable();
    seeds.dedup();
    let mut seen: FxHashSet<St> = FxHashSet::default();
    let mut stack: Vec<St> = Vec::new();
    let push = |s: St, seen: &mut FxHashSet<St>, stack: &mut Vec<St>| {
        if seen.insert(s) {
            stack.push(s);
        }
    };
    for &s in &seeds {
        push(s, &mut seen, &mut stack);
    }
    for (&id, d) in f.defs.iter() {
        push(dc::st(id, dc::NONE), &mut seen, &mut stack);
        push(dc::st(d.module | dc::MOD, dc::NONE), &mut seen, &mut stack);
    }
    let mut e = Enumerated { states: Vec::new(), off: vec![0], tgt: Vec::new(), seeds };
    let mut buf = Vec::new();
    while let Some(s) = stack.pop() {
        buf.clear();
        dc::succ(f, s, &mut buf);
        e.states.push(s);
        e.tgt.extend_from_slice(&buf);
        e.off.push(e.tgt.len());
        for &t in &buf {
            push(t, &mut seen, &mut stack);
        }
    }
    e
}
