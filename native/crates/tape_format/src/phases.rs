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

/// The stable order of `keys` (their indexes, the keys ascending), by phases: a digit of 8 bits at a time from the
/// lowest of `bits`, each a keyed-count phase (each chunk of `size` items counts its items' digits, `[u32; 256]`) and
/// its prefix sum (elementwise), then each chunk's items placed at their digit's offset, in their order: the radix
/// sort's. Each phase's chunks in `order`'s order.
pub fn sort_by_key(keys: &[u64], bits: u32, size: usize, order: Order<'_>) -> Vec<u32> {
    let size = size.max(1);
    let n = keys.len();
    let chunks = n.div_ceil(size).max(1);
    let mut items: Vec<u32> = (0..n as u32).collect();
    let mut shift = 0;
    while shift < bits {
        let digit = |i: u32| ((keys[i as usize] >> shift) & 0xff) as usize;
        let range = |c: usize| c * size..((c + 1) * size).min(n);
        let counts: Vec<[u32; 256]> = each(chunks, order, |c| {
            let mut k = [0u32; 256];
            for &i in &items[range(c)] {
                k[digit(i)] += 1;
            }
            k
        });
        let (before, total) = prefix(&counts, [0; 256], order, |a, b| std::array::from_fn(|d| a[d] + b[d]));
        // Each digit's first place: the items of the digits below it (a scan of 256 counts, one chunk's).
        let (base, _) = prefix(&total.iter().map(|&t| t as usize).collect::<Vec<_>>(), 0, order, |a, b| a + b);
        let mut next = vec![0u32; n];
        let placed: Vec<Vec<(usize, u32)>> = each(chunks, order, |c| {
            let mut at = before[c];
            items[range(c)]
                .iter()
                .map(|&i| {
                    let d = digit(i);
                    at[d] += 1;
                    (base[d] + at[d] as usize - 1, i)
                })
                .collect()
        });
        for (p, i) in placed.into_iter().flatten() {
            next[p] = i;
        }
        items = next;
        shift += 8;
    }
    items
}
