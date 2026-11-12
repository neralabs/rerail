//! Canonical persistent indexes. Equal row sets have equal Merkle roots independent of insertion order.
use sha2::{Digest, Sha256};
use std::sync::Arc;
pub mod bulk;
#[derive(Clone, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct Cells {
    values: Vec<Option<i64>>,
    hash: [u8; 32],
}
impl std::ops::Deref for Cells {
    type Target = Vec<Option<i64>>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}
pub(super) type Key = Arc<Cells>;
pub(super) fn key(values: Vec<Option<i64>>) -> Key {
    let mut h = Sha256::new();
    h.update((values.len() as u64).to_be_bytes());
    for v in &values {
        match v {
            Some(v) => {
                h.update([1]);
                h.update(v.to_be_bytes());
            }
            None => h.update([0]),
        }
    }
    Arc::new(Cells {
        values,
        hash: h.finalize().into(),
    })
}
pub(super) type Link<V> = Option<Arc<Node<V>>>;
pub struct Node<V> {
    pub key: Key,
    pub value: V,
    pub left: Link<V>,
    pub right: Link<V>,
    pub size: usize,
    pub digest: [u8; 32],
    hashed: bool,
}
pub(super) fn size<V>(n: &Link<V>) -> usize {
    n.as_ref().map_or(0, |n| n.size)
}
impl<V: Clone> Node<V> {
    fn make(key: Key, value: V, left: Link<V>, right: Link<V>, hashed: bool) -> Arc<Self> {
        let priority = key.hash;
        // Only the complete-row index participates in version comparisons. Secondary
        // indexes share their paths but need no Merkle hash at every copied node.
        let digest = if hashed {
            let mut h = Sha256::new();
            h.update([0x73]);
            h.update(left.as_ref().map_or([0; 32], |n| n.digest));
            h.update(priority);
            h.update(right.as_ref().map_or([0; 32], |n| n.digest));
            h.finalize().into()
        } else {
            [0; 32]
        };
        Arc::new(Self {
            key,
            value,
            size: 1 + size(&left) + size(&right),
            left,
            right,
            digest,
            hashed,
        })
    }
    fn before(a: &Self, b: &Self) -> bool {
        (a.key.hash, &a.key) < (b.key.hash, &b.key)
    }
    pub fn split(root: &Link<V>, key: &Key) -> (Link<V>, Option<V>, Link<V>) {
        Self::split_counted(root, key, &mut 0)
    }
    fn split_counted(
        root: &Link<V>,
        key: &Key,
        visited: &mut usize,
    ) -> (Link<V>, Option<V>, Link<V>) {
        *visited += 1;
        let Some(n) = root else {
            return (None, None, None);
        };
        match n.key.cmp(key) {
            std::cmp::Ordering::Equal => (n.left.clone(), Some(n.value.clone()), n.right.clone()),
            std::cmp::Ordering::Less => {
                let (left, found, right) = Self::split_counted(&n.right, key, visited);
                (
                    Some(Self::make(
                        n.key.clone(),
                        n.value.clone(),
                        n.left.clone(),
                        left,
                        n.hashed,
                    )),
                    found,
                    right,
                )
            }
            std::cmp::Ordering::Greater => {
                let (left, found, right) = Self::split_counted(&n.left, key, visited);
                (
                    left,
                    found,
                    Some(Self::make(
                        n.key.clone(),
                        n.value.clone(),
                        right,
                        n.right.clone(),
                        n.hashed,
                    )),
                )
            }
        }
    }
    fn merge(left: &Link<V>, right: &Link<V>) -> Link<V> {
        match (left, right) {
            (None, _) => right.clone(),
            (_, None) => left.clone(),
            (Some(l), Some(r)) => {
                if Self::before(l, r) {
                    Some(Self::make(
                        l.key.clone(),
                        l.value.clone(),
                        l.left.clone(),
                        Self::merge(&l.right, right),
                        l.hashed,
                    ))
                } else {
                    Some(Self::make(
                        r.key.clone(),
                        r.value.clone(),
                        Self::merge(left, &r.left),
                        r.right.clone(),
                        r.hashed,
                    ))
                }
            }
        }
    }
    pub fn set(root: &Link<V>, key: Key, value: Option<V>) -> Link<V> {
        Self::set_inner(root, key, value, true)
    }
    pub fn set_index(root: &Link<V>, key: Key, value: Option<V>) -> Link<V> {
        Self::set_inner(root, key, value, false)
    }
    fn set_inner(root: &Link<V>, key: Key, value: Option<V>, hashed: bool) -> Link<V> {
        let Some(n) = root else {
            return value.map(|v| Self::make(key, v, None, None, hashed));
        };
        debug_assert_eq!(hashed, n.hashed);
        if key == n.key {
            return value.map_or_else(
                || Self::merge(&n.left, &n.right),
                |v| Some(Self::make(key, v, n.left.clone(), n.right.clone(), hashed)),
            );
        }
        if let Some(value) = value.as_ref() {
            let priority = key.hash;
            if (priority, &key) < (n.key.hash, &n.key) {
                let (left, _, right) = Self::split(root, &key);
                return Some(Self::make(key, value.clone(), left, right, hashed));
            }
        }
        if key < n.key {
            Some(Self::make(
                n.key.clone(),
                n.value.clone(),
                Self::set_inner(&n.left, key, value, hashed),
                n.right.clone(),
                hashed,
            ))
        } else {
            Some(Self::make(
                n.key.clone(),
                n.value.clone(),
                n.left.clone(),
                Self::set_inner(&n.right, key, value, hashed),
                hashed,
            ))
        }
    }
    pub fn get<'a>(root: &'a Link<V>, key: &[Option<i64>]) -> Option<&'a V> {
        let mut at = root.as_deref();
        while let Some(n) = at {
            match n.key.as_slice().cmp(key) {
                std::cmp::Ordering::Equal => return Some(&n.value),
                std::cmp::Ordering::Less => at = n.right.as_deref(),
                std::cmp::Ordering::Greater => at = n.left.as_deref(),
            }
        }
        None
    }
    pub fn nth(root: &Link<V>, mut rank: usize) -> &V {
        let mut n = root.as_deref().expect("rank in version");
        loop {
            let left = size(&n.left);
            if rank < left {
                n = n.left.as_deref().unwrap();
            } else if rank == left {
                return &n.value;
            } else {
                rank -= left + 1;
                n = n.right.as_deref().unwrap();
            }
        }
    }
    pub fn rank(root: &Link<V>, key: &Key) -> Option<usize> {
        let mut at = root.as_deref();
        let mut base = 0;
        while let Some(n) = at {
            match n.key.cmp(key) {
                std::cmp::Ordering::Equal => return Some(base + size(&n.left)),
                std::cmp::Ordering::Less => {
                    base += size(&n.left) + 1;
                    at = n.right.as_deref();
                }
                std::cmp::Ordering::Greater => at = n.left.as_deref(),
            }
        }
        None
    }
    pub fn visit(root: &Link<V>, f: &mut impl FnMut(&V)) {
        if let Some(n) = root {
            Self::visit(&n.left, f);
            f(&n.value);
            Self::visit(&n.right, f);
        }
    }
    pub fn prefix(root: &Link<V>, prefix: &[Option<i64>], out: &mut Vec<V>) {
        let Some(n) = root else {
            return;
        };
        let cmp = n.key[..prefix.len().min(n.key.len())].cmp(prefix);
        if cmp != std::cmp::Ordering::Less {
            Self::prefix(&n.left, prefix, out);
        }
        if n.key.starts_with(prefix) {
            out.push(n.value.clone());
        }
        if cmp != std::cmp::Ordering::Greater {
            Self::prefix(&n.right, prefix, out);
        }
    }
    pub fn has_prefix(root: &Link<V>, prefix: &[Option<i64>]) -> bool {
        let mut at = root.as_deref();
        while let Some(n) = at {
            if n.key.starts_with(prefix) {
                return true;
            }
            at = if n.key.as_slice() < prefix {
                n.right.as_deref()
            } else {
                n.left.as_deref()
            };
        }
        false
    }
    pub fn prefix_ranks(root: &Link<V>, prefix: &[Option<i64>]) -> std::ops::Range<usize> {
        let bound = |upper: bool| {
            let mut at = root.as_deref();
            let mut rank = 0;
            while let Some(n) = at {
                let cmp = n.key[..prefix.len()].cmp(prefix);
                if cmp == std::cmp::Ordering::Less || (upper && cmp == std::cmp::Ordering::Equal) {
                    rank += size(&n.left) + 1;
                    at = n.right.as_deref();
                } else {
                    at = n.left.as_deref();
                }
            }
            rank
        };
        bound(false)..bound(true)
    }
    /// The digest intentionally excludes payloads. Call this on an index keyed by the
    /// complete row, rather than a primary-key index whose payload can change.
    pub fn difference(
        old: &Link<V>,
        new: &Link<V>,
        plus: &mut Vec<V>,
        minus: &mut Vec<V>,
        visited: &mut usize,
    ) {
        // Compare borrowed bounded subtrees. Splitting an immutable tree just to
        // inspect it would copy paths and calculate Merkle hashes during a read.
        fn bounded<'a, V>(
            mut root: &'a Link<V>,
            lower: Option<&Key>,
            upper: Option<&Key>,
            visited: &mut usize,
        ) -> &'a Link<V> {
            while let Some(n) = root {
                if lower.is_some_and(|k| n.key <= *k) {
                    *visited += 1;
                    root = &n.right;
                } else if upper.is_some_and(|k| n.key >= *k) {
                    *visited += 1;
                    root = &n.left;
                } else {
                    break;
                }
            }
            root
        }
        fn walk<V: Clone>(
            old: &Link<V>,
            new: &Link<V>,
            lower: Option<&Key>,
            upper: Option<&Key>,
            plus: &mut Vec<V>,
            minus: &mut Vec<V>,
            visited: &mut usize,
        ) {
            *visited += 1;
            let old = bounded(old, lower, upper, visited);
            let new = bounded(new, lower, upper, visited);
            match (old, new) {
                (None, None) => {}
                (Some(a), Some(b)) if a.size == b.size && a.digest == b.digest => {}
                (Some(a), Some(b)) if a.key == b.key => {
                    walk(&a.left, &b.left, lower, Some(&a.key), plus, minus, visited);
                    walk(
                        &a.right,
                        &b.right,
                        Some(&a.key),
                        upper,
                        plus,
                        minus,
                        visited,
                    );
                }
                (Some(a), _) if new.is_none() || Node::before(a, new.as_ref().unwrap()) => {
                    // The least-priority old root cannot occur in the new range.
                    walk(&a.left, new, lower, Some(&a.key), plus, minus, visited);
                    minus.push(a.value.clone());
                    walk(&a.right, new, Some(&a.key), upper, plus, minus, visited);
                }
                (_, Some(b)) => {
                    walk(old, &b.left, lower, Some(&b.key), plus, minus, visited);
                    plus.push(b.value.clone());
                    walk(old, &b.right, Some(&b.key), upper, plus, minus, visited);
                }
                _ => unreachable!(),
            }
        }
        assert!(
            old.as_ref().is_none_or(|n| n.hashed) && new.as_ref().is_none_or(|n| n.hashed),
            "version differences require complete-row indexes"
        );
        walk(old, new, None, None, plus, minus, visited);
    }
}
