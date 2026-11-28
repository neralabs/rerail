//! The phases' prefix sum (decision 0726's tier 2 as phases): a phase runs a block operator over each chunk on its
//! own, in any order, and one exclusive prefix sum of their summaries by an associative merge places what each writes.
//! The same scan for every phase: Hillis and Steele's, each round's chunks on their own, as a device runs it.

/// The order a phase runs its chunks in: a permutation of `0..n`, asked each time. Any order gives the same answer
/// (`in_order`; the tests' random ones).
pub type Order<'a> = &'a dyn Fn(usize) -> Vec<usize>;

/// The chunks in order.
pub fn in_order(n: usize) -> Vec<usize> {
    (0..n).collect()
}

/// `f` of each of `n` chunks, run in `order`'s order: each chunk's result in its place.
pub fn each<T>(n: usize, order: Order<'_>, mut f: impl FnMut(usize) -> T) -> Vec<T> {
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    for c in order(n) {
        assert!(out[c].is_none(), "a chunk run twice");
        out[c] = Some(f(c));
    }
    out.into_iter().map(|x| x.expect("every chunk run")).collect()
}

/// The exclusive prefix sum of `outs` by `merge` (`a` the earlier; `unit` its identity), and the total: round `r`
/// merges each chunk's value after the one `2^r` before it, each round's chunks in `order`'s order.
pub fn prefix<T: Clone>(outs: &[T], unit: T, order: Order<'_>, merge: impl Fn(&T, &T) -> T) -> (Vec<T>, T) {
    let mut acc = outs.to_vec();
    let mut d = 1;
    while d < acc.len() {
        let next = each(acc.len(), order, |c| if c < d { acc[c].clone() } else { merge(&acc[c - d], &acc[c]) });
        acc = next;
        d *= 2;
    }
    let total = acc.last().cloned().unwrap_or_else(|| unit.clone());
    (each(acc.len(), order, |c| if c == 0 { unit.clone() } else { acc[c - 1].clone() }), total)
}
