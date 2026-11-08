//! The Cartesian tree of sorted keys has the same shape as individual canonical insertions.
use super::*;

impl<V: Clone> Node<V> {
    pub fn bulk(entries: Vec<(Key, V)>, hashed: bool) -> Link<V> {
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
        fn build<V: Clone>(
            at: Option<usize>,
            entries: &[(Key, V)],
            left: &[Option<usize>],
            right: &[Option<usize>],
            hashed: bool,
        ) -> Link<V> {
            at.map(|i| {
                Node::make(
                    entries[i].0.clone(),
                    entries[i].1.clone(),
                    build(left[i], entries, left, right, hashed),
                    build(right[i], entries, left, right, hashed),
                    hashed,
                )
            })
        }
        build(stack.first().copied(), &entries, &left, &right, hashed)
    }
}
