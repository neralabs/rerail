//! The chunks joined: each chunk's leaves checked against its neighbours', not propagated.
//!
//! A chunk solved from an unknown start (`solve`) is a function from its entry to its leaves.
//! The file's first chunk starts from the file's start (every carry zero), and a chunk's entry
//! must agree with an end the chunk before can have; the back fills' carries must agree the
//! other way (a chunk's carry into its last block is the next chunk's out of its first, and zero
//! past the file's end). A forward pass keeps the leaves an end before allows, a backward pass
//! the leaves an entry after allows. Of the leaves that follow an end, a leaf whose entry
//! another's refines is dropped: it is the same start, said less exactly. A chunk left with one leaf, or with leaves that agree on
//! every output bit, is decided; any other is run again from its now known start.

use crate::chunk::Prepared;
use crate::solve::{Leaf, Solved, T, TStart};

/// Two three-valued words that can be one value: no bit known in both and different.
fn agree(a: T, b: T) -> bool {
    a.k & b.k & (a.v ^ b.v) == 0
}

/// Two states that can be one, the back fills' carries aside.
pub fn agree_state(a: &TStart, b: &TStart) -> bool {
    a.ops.iter().zip(&b.ops).all(|(x, y)| agree(*x, *y)) && a.groups.iter().zip(&b.groups).all(|(g, h)| g.iter().zip(h).all(|(x, y)| agree(*x, *y)))
}

fn follows(before: &Leaf, after: &Leaf) -> bool {
    agree_state(&before.end, &after.entry) && before.back_in.iter().zip(&after.back_out).all(|(x, y)| agree(*x, *y))
}

/// `b` refines `a`: known wherever `a` is, the same there, and known somewhere `a` is not.
fn refines(a: &TStart, b: &TStart) -> bool {
    let words = |s: &TStart| s.ops.iter().chain(s.groups.iter().flatten()).copied().collect::<Vec<T>>();
    let (x, y) = (words(a), words(b));
    x.iter().zip(&y).all(|(p, q)| p.k & !q.k == 0 && (p.v ^ q.v) & p.k == 0) && x.iter().zip(&y).any(|(p, q)| q.k & !p.k != 0)
}

/// The leaves of `after` an end of `before` allows: those that follow it, less the ones whose
/// entry another such leaf's entry refines (the same start, said less exactly).
fn allowed<'a>(before: &Leaf, after: &'a Solved) -> Vec<usize> {
    let follow: Vec<usize> = (0..after.leaves.len()).filter(|&i| follows(before, &after.leaves[i])).collect();
    follow.iter().copied().filter(|&i| !follow.iter().any(|&j| refines(&after.leaves[i].entry, &after.leaves[j].entry))).collect()
}

/// The leaves each chunk keeps, joined in order from the file's start.
pub fn join(p: &Prepared<'_>, chunks: &[Solved]) -> Vec<Vec<usize>> {
    let zero = {
        let z = p.zero();
        TStart { ops: z.ops.iter().map(|&v| T::known(v)).collect(), groups: z.groups.iter().map(|g| g.iter().map(|&v| T::known(v)).collect()).collect(), backs: z.backs.iter().map(|_| T::X).collect() }
    };
    let n = chunks.len();
    let mut alive: Vec<Vec<usize>> = Vec::with_capacity(n);
    for (c, s) in chunks.iter().enumerate() {
        let keep: Vec<usize> = (0..s.leaves.len())
            .filter(|&i| {
                let l = &s.leaves[i];
                if c == 0 {
                    agree_state(&zero, &l.entry)
                } else {
                    alive[c - 1].iter().any(|&j| allowed(&chunks[c - 1].leaves[j], s).contains(&i))
                }
            })
            .filter(|&i| c + 1 < n || chunks[c].leaves[i].back_in.iter().all(|b| agree(*b, T::known(0))))
            .collect();
        alive.push(keep);
    }
    for c in (0..n.saturating_sub(1)).rev() {
        let next = alive[c + 1].clone();
        alive[c].retain(|&i| { let ok = allowed(&chunks[c].leaves[i], &chunks[c + 1]); next.iter().any(|j| ok.contains(j)) });
    }
    alive
}

/// A chunk's outputs from the leaves it keeps: known where every one agrees.
pub fn outputs(s: &Solved, keep: &[usize]) -> Option<Vec<Vec<T>>> {
    let mut it = keep.iter().map(|&i| &s.leaves[i].outputs);
    let first = it.next()?.clone();
    Some(it.fold(first, |mut acc, o| {
        for (a, b) in acc.iter_mut().zip(o) {
            for (x, y) in a.iter_mut().zip(b) {
                *x = x.merge(*y);
            }
        }
        acc
    }))
}
