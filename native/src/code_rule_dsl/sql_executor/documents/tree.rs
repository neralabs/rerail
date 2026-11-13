//! Immutable AVL indexes. Edits copy one search path and retain all other subtrees.
use std::{cmp::Ordering, ops::Bound, sync::Arc};

pub(super) trait Measure<K>: Clone {
    type Summary: Default;
    fn measure(
        &self,
        key: &K,
        left: Option<&Self::Summary>,
        right: Option<&Self::Summary>,
    ) -> Self::Summary;
}
pub(super) type Link<K, V> = Option<Arc<Node<K, V>>>;
pub(super) struct Node<K, V: Measure<K>> {
    pub key: K,
    pub value: V,
    pub left: Link<K, V>,
    pub right: Link<K, V>,
    pub summary: V::Summary,
    pub size: usize,
    height: u8,
}
fn height<K, V: Measure<K>>(n: &Link<K, V>) -> u8 {
    n.as_ref().map_or(0, |n| n.height)
}
pub(super) fn size<K, V: Measure<K>>(n: &Link<K, V>) -> usize {
    n.as_ref().map_or(0, |n| n.size)
}
impl<K: Ord + Clone, V: Measure<K>> Node<K, V> {
    fn make(key: K, value: V, left: Link<K, V>, right: Link<K, V>, work: &mut usize) -> Arc<Self> {
        *work += 1;
        Arc::new(Self {
            summary: value.measure(
                &key,
                left.as_ref().map(|n| &n.summary),
                right.as_ref().map(|n| &n.summary),
            ),
            size: 1 + size(&left) + size(&right),
            height: 1 + height(&left).max(height(&right)),
            key,
            value,
            left,
            right,
        })
    }
    fn balance(
        key: K,
        value: V,
        left: Link<K, V>,
        right: Link<K, V>,
        work: &mut usize,
    ) -> Arc<Self> {
        if i16::from(height(&left)) - i16::from(height(&right)) > 1 {
            let l = left.as_ref().unwrap();
            if height(&l.left) >= height(&l.right) {
                let r = Some(Self::make(key, value, l.right.clone(), right, work));
                return Self::make(l.key.clone(), l.value.clone(), l.left.clone(), r, work);
            }
            let m = l.right.as_ref().unwrap();
            let a = Some(Self::make(
                l.key.clone(),
                l.value.clone(),
                l.left.clone(),
                m.left.clone(),
                work,
            ));
            let b = Some(Self::make(key, value, m.right.clone(), right, work));
            return Self::make(m.key.clone(), m.value.clone(), a, b, work);
        }
        if i16::from(height(&right)) - i16::from(height(&left)) > 1 {
            let r = right.as_ref().unwrap();
            if height(&r.right) >= height(&r.left) {
                let l = Some(Self::make(key, value, left, r.left.clone(), work));
                return Self::make(r.key.clone(), r.value.clone(), l, r.right.clone(), work);
            }
            let m = r.left.as_ref().unwrap();
            let a = Some(Self::make(key, value, left, m.left.clone(), work));
            let b = Some(Self::make(
                r.key.clone(),
                r.value.clone(),
                m.right.clone(),
                r.right.clone(),
                work,
            ));
            return Self::make(m.key.clone(), m.value.clone(), a, b, work);
        }
        Self::make(key, value, left, right, work)
    }
    pub fn set(root: &Link<K, V>, key: K, value: Option<V>, work: &mut usize) -> Link<K, V> {
        let Some(n) = root else {
            return value.map(|v| Self::make(key, v, None, None, work));
        };
        match key.cmp(&n.key) {
            Ordering::Less => Some(Self::balance(
                n.key.clone(),
                n.value.clone(),
                Self::set(&n.left, key, value, work),
                n.right.clone(),
                work,
            )),
            Ordering::Greater => Some(Self::balance(
                n.key.clone(),
                n.value.clone(),
                n.left.clone(),
                Self::set(&n.right, key, value, work),
                work,
            )),
            Ordering::Equal => match value {
                Some(v) => Some(Self::make(key, v, n.left.clone(), n.right.clone(), work)),
                None => match (&n.left, &n.right) {
                    (None, _) => n.right.clone(),
                    (_, None) => n.left.clone(),
                    (_, Some(r)) => {
                        let mut next = r.as_ref();
                        while let Some(l) = &next.left {
                            next = l;
                        }
                        let right = Self::set(&n.right, next.key.clone(), None, work);
                        Some(Self::balance(
                            next.key.clone(),
                            next.value.clone(),
                            n.left.clone(),
                            right,
                            work,
                        ))
                    }
                },
            },
        }
    }
    pub fn get<'a>(root: &'a Link<K, V>, key: &K) -> Option<&'a Self> {
        let mut at = root.as_deref();
        while let Some(n) = at {
            match key.cmp(&n.key) {
                Ordering::Equal => return Some(n),
                Ordering::Less => at = n.left.as_deref(),
                Ordering::Greater => at = n.right.as_deref(),
            }
        }
        None
    }
    pub fn nth(root: &Link<K, V>, mut rank: usize) -> &Self {
        let mut at = root.as_deref().expect("nonempty index");
        loop {
            let left = size(&at.left);
            if rank < left {
                at = at.left.as_deref().unwrap()
            } else if rank == left {
                return at;
            } else {
                rank -= left + 1;
                at = at.right.as_deref().unwrap()
            }
        }
    }
    pub fn rank(root: &Link<K, V>, key: &K) -> Option<usize> {
        let mut at = root.as_deref();
        let mut base = 0;
        while let Some(n) = at {
            match key.cmp(&n.key) {
                Ordering::Equal => return Some(base + size(&n.left)),
                Ordering::Less => at = n.left.as_deref(),
                Ordering::Greater => {
                    base += size(&n.left) + 1;
                    at = n.right.as_deref()
                }
            }
        }
        None
    }
    pub fn visit(root: &Link<K, V>, f: &mut impl FnMut(&Self)) {
        if let Some(n) = root {
            Self::visit(&n.left, f);
            f(n);
            Self::visit(&n.right, f)
        }
    }
    pub fn range(
        root: &Link<K, V>,
        lo: &Bound<K>,
        hi: &Bound<K>,
        base: usize,
        f: &mut impl FnMut(&Self, usize),
    ) {
        let Some(n) = root else { return };
        let above = match lo {
            Bound::Unbounded => true,
            Bound::Included(k) => &n.key >= k,
            Bound::Excluded(k) => &n.key > k,
        };
        let below = match hi {
            Bound::Unbounded => true,
            Bound::Included(k) => &n.key <= k,
            Bound::Excluded(k) => &n.key < k,
        };
        if above {
            Self::range(&n.left, lo, hi, base, f)
        }
        let rank = base + size(&n.left);
        if above && below {
            f(n, rank)
        }
        if below {
            Self::range(&n.right, lo, hi, rank + 1, f)
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[derive(Clone)]
    struct Number(i64);
    impl Measure<i32> for Number {
        type Summary = i64;
        fn measure(&self, _: &i32, l: Option<&i64>, r: Option<&i64>) -> i64 {
            self.0 + l.copied().unwrap_or(0) + r.copied().unwrap_or(0)
        }
    }
    #[test]
    fn immutable_index_matches_ordered_map_after_insertions_and_retractions() {
        let mut root = None;
        let mut reference = std::collections::BTreeMap::new();
        let mut work = 0;
        let mut seed = 7u32;
        let mut saved = vec![];
        for step in 0..1500 {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let key = (seed % 137) as i32;
            let value = if seed % 5 == 0 {
                None
            } else {
                Some(Number(step))
            };
            if let Some(v) = &value {
                reference.insert(key, v.0);
            } else {
                reference.remove(&key);
            }
            root = Node::set(&root, key, value, &mut work);
            if step % 50 == 0 {
                saved.push((root.clone(), reference.clone()));
            }
            assert_eq!(size(&root), reference.len());
            assert_eq!(
                root.as_ref().map_or(0, |n| n.summary),
                reference.values().sum::<i64>()
            );
            for (rank, (key, value)) in reference.iter().enumerate() {
                assert_eq!(Node::rank(&root, key), Some(rank));
                let node = Node::nth(&root, rank);
                assert_eq!((&node.key, node.value.0), (key, *value));
            }
        }
        for (root, reference) in saved {
            let mut actual = vec![];
            Node::visit(&root, &mut |n| actual.push((n.key, n.value.0)));
            assert_eq!(actual, reference.into_iter().collect::<Vec<_>>());
        }
        fn balanced(root: &Link<i32, Number>) -> u8 {
            if let Some(n) = root {
                let l = balanced(&n.left);
                let r = balanced(&n.right);
                assert!(l.abs_diff(r) <= 1);
                assert_eq!(n.height, 1 + l.max(r));
                n.height
            } else {
                0
            }
        }
        balanced(&root);
    }
}
