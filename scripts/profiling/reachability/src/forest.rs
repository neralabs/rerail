//! Iterative forest indexing. Cycles and multiple parents are rejected explicitly.
use crate::graph::{Graph, failure};
use rusqlite::Result;
use std::collections::{HashMap, HashSet};
pub(crate) struct Forest {
    preorder: Vec<i64>,
    intervals: HashMap<i64, (usize, usize)>,
    parent: HashMap<i64, i64>,
}
impl Forest {
    pub(crate) fn build(graph: &mut Graph, nodes: &[i64], edges: &[[i64; 2]]) -> Result<Self> {
        let mut all: HashSet<i64> = nodes.iter().copied().collect();
        let mut parent = HashMap::new();
        let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
        for &[src, dst] in edges {
            graph.charge()?;
            all.insert(src);
            all.insert(dst);
            if let Some(old) = parent.insert(dst, src) {
                if old != src {
                    return Err(failure("tree requires a single parent"));
                }
            } else {
                children.entry(src).or_default().push(dst);
            }
        }
        let mut roots = all
            .iter()
            .filter(|n| !parent.contains_key(n))
            .copied()
            .collect::<Vec<_>>();
        roots.sort_unstable();
        let mut preorder = Vec::new();
        let mut intervals: HashMap<i64, (usize, usize)> = HashMap::new();
        let mut pending = roots
            .into_iter()
            .rev()
            .map(|n| (n, false))
            .collect::<Vec<_>>();
        while let Some((node, exit)) = pending.pop() {
            graph.charge()?;
            if exit {
                intervals.get_mut(&node).unwrap().1 = preorder.len();
                continue;
            }
            intervals.insert(node, (preorder.len(), 0));
            preorder.push(node);
            pending.push((node, true));
            if let Some(values) = children.get(&node) {
                pending.extend(values.iter().rev().map(|v| (*v, false)));
            }
        }
        if preorder.len() != all.len() {
            return Err(failure("tree contains a cycle"));
        }
        Ok(Self {
            preorder,
            intervals,
            parent,
        })
    }
    pub(crate) fn contains(&self, root: i64, node: i64) -> bool {
        match (self.intervals.get(&root), self.intervals.get(&node)) {
            (Some(&(a, b)), Some(&(n, _))) => a <= n && n < b,
            _ => false,
        }
    }
    pub(crate) fn reachable(&self, root: i64, reverse: bool) -> Vec<i64> {
        if reverse {
            if !self.intervals.contains_key(&root) {
                return vec![];
            }
            let mut result = vec![root];
            let mut next = root;
            while let Some(parent) = self.parent.get(&next) {
                result.push(*parent);
                next = *parent;
            }
            result
        } else {
            self.intervals
                .get(&root)
                .map_or_else(Vec::new, |&(a, b)| self.preorder[a..b].to_vec())
        }
    }
}
