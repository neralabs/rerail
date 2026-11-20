//! The Cartesian tree of sorted keys has the same shape as individual canonical insertions.
use super::*;

impl<V: Clone> Node<V> {
    pub fn bulk(entries: Vec<(Key, V)>, hashed: bool) -> Link<V> {
        Self::bulk_reuse(entries, hashed, &None, &|_, _| false)
    }
    pub fn bulk_reuse<F: Fn(&V, &V) -> bool>(
        entries: Vec<(Key, V)>,
        hashed: bool,
        old: &Link<V>,
        same: &F,
    ) -> Link<V> {
        if entries.is_empty() {
            return None;
        }
        debug_assert!(entries.windows(2).all(|w| w[0].0 < w[1].0));
        let mut left = vec![None; entries.len()];
        let mut right = left.clone();
        let mut stack = Vec::<usize>::new();
        for i in 0..entries.len() {
            let mut previous = None;
            while stack.last().is_some_and(|&j| {
                (entries[j].0.hash, &entries[j].0) > (entries[i].0.hash, &entries[i].0)
            }) {
                previous = stack.pop();
            }
            if let Some(&parent) = stack.last() {
                right[parent] = Some(i);
            }
            left[i] = previous;
            stack.push(i);
        }
        fn build<V: Clone, F: Fn(&V, &V) -> bool>(
            at: Option<usize>,
            entries: &[(Key, V)],
            left: &[Option<usize>],
            right: &[Option<usize>],
            hashed: bool,
            old: &Link<V>,
            same: &F,
        ) -> Link<V> {
            let i = at?;
            let matched = old.as_ref().filter(|n| n.key == entries[i].0);
            let (before, after) = if let Some(n) = matched {
                (n.left.clone(), n.right.clone())
            } else {
                let (l, _, r) = Node::split(old, &entries[i].0);
                (l, r)
            };
            let l = build(left[i], entries, left, right, hashed, &before, same);
            let r = build(right[i], entries, left, right, hashed, &after, same);
            if let Some(n) = matched {
                let identical = |a: &Link<V>, b: &Link<V>| match (a, b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                    _ => false,
                };
                if identical(&l, &n.left)
                    && identical(&r, &n.right)
                    && same(&entries[i].1, &n.value)
                {
                    return Some(n.clone());
                }
            }
            Some(Node::make(
                matched.map_or_else(|| entries[i].0.clone(), |n| n.key.clone()),
                entries[i].1.clone(),
                l,
                r,
                hashed,
            ))
        }
        build(
            stack.first().copied(),
            &entries,
            &left,
            &right,
            hashed,
            old,
            same,
        )
    }
}
