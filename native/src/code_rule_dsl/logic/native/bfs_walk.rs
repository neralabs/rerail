//! Ordered breadth-first walks over one shared adjacency batch. Roots are demands,
//! not implicitly visited vertices: a cycle may reach its root through an edge.
use std::collections::HashMap;

/// `(root, reached vertex, zero-based visit ordinal)` for every demanded root.
/// Neighbours are ordered by `(edge ordinal, destination id)`. Duplicate arrivals
/// are ignored after the first; no depth limit or per-root SQL is involved.
pub(in crate::code_rule_dsl) fn bfs_walk(edges: &[(i64, i64, i64)], roots: &[i64]) -> Vec<(i64, i64, i64)> {
    let _profile = super::super::profile::span(|| "ordered breadth-first walks".into());
    if roots.is_empty() || edges.is_empty() { return Vec::new(); }
    let mut vertices: Vec<_> = edges.iter().flat_map(|&(a, b, _)| [a, b]).chain(roots.iter().copied()).collect();
    vertices.sort_unstable();
    vertices.dedup();
    let ids: HashMap<_, _> = vertices.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let mut ordered: Vec<_> = edges.iter().map(|&(a, b, rank)| (ids[&a], rank, ids[&b])).collect();
    ordered.sort_unstable();
    ordered.dedup();
    let mut offsets = vec![0usize; vertices.len() + 1];
    for &(a, _, _) in &ordered { offsets[a + 1] += 1; }
    for i in 0..vertices.len() { offsets[i + 1] += offsets[i]; }
    let destinations: Vec<_> = ordered.iter().map(|e| e.2).collect();
    let mut demands = roots.to_vec();
    demands.sort_unstable();
    demands.dedup();
    let mut seen = vec![0u64; vertices.len()];
    let mut epoch = 0u64;
    let mut queue = Vec::new();
    let mut rows = Vec::new();
    for root in demands {
        epoch = epoch.wrapping_add(1);
        if epoch == 0 { seen.fill(0); epoch = 1; }
        queue.clear();
        let source = ids[&root];
        for &v in &destinations[offsets[source]..offsets[source + 1]] {
            if seen[v] != epoch { seen[v] = epoch; queue.push(v); }
        }
        let mut cursor = 0;
        while cursor < queue.len() {
            let v = queue[cursor];
            rows.push((root, vertices[v], cursor as i64));
            cursor += 1;
            for &next in &destinations[offsets[v]..offsets[v + 1]] {
                if seen[next] != epoch { seen[next] = epoch; queue.push(next); }
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::bfs_walk;
    use std::collections::{HashMap, HashSet, VecDeque};

    fn reference(edges: &[(i64, i64, i64)], roots: &[i64]) -> Vec<(i64, i64, i64)> {
        let mut adjacency: HashMap<i64, Vec<(i64, i64)>> = HashMap::new();
        for &(a, b, rank) in edges { adjacency.entry(a).or_default().push((rank, b)); }
        for values in adjacency.values_mut() { values.sort_unstable(); }
        let mut roots = roots.to_vec(); roots.sort_unstable(); roots.dedup();
        let mut result = Vec::new();
        for root in roots {
            let mut queue = VecDeque::from(adjacency.get(&root).cloned().unwrap_or_default());
            let mut seen = HashSet::new();
            let mut ordinal = 0;
            while let Some((_, v)) = queue.pop_front() {
                if !seen.insert(v) { continue; }
                result.push((root, v, ordinal)); ordinal += 1;
                queue.extend(adjacency.get(&v).into_iter().flatten().copied());
            }
        }
        result
    }

    #[test]
    fn cycles_and_diamonds_keep_breadth_first_order_for_each_demand() {
        let edges = [(1, 2, 0), (1, 3, 1), (2, 4, 0), (3, 4, 0), (4, 1, 0), (7, 7, 0)];
        assert_eq!(bfs_walk(&edges, &[7, 1, 1, 99]), vec![(1, 2, 0), (1, 3, 1), (1, 4, 2), (1, 1, 3), (7, 7, 0)]);
        assert!(bfs_walk(&edges, &[]).is_empty());
        assert!(bfs_walk(&[], &[1]).is_empty());
        assert_eq!(bfs_walk(&[(1, 2, 5), (1, 3, -1), (1, 2, 0)], &[1]), vec![(1, 3, 0), (1, 2, 1)]);
    }

    #[test]
    fn random_batched_walks_match_the_queue_reference() {
        let mut seed = 0x758a9214_u64;
        let mut random = || { seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1); seed >> 32 };
        for _ in 0..2000 {
            let n = 1 + random() as usize % 9;
            let ids: Vec<_> = (0..n).map(|i| i as i64 * 1000000007 - 19).collect();
            let mut edges = Vec::new();
            for &a in &ids { for &b in &ids {
                if random() % 4 == 0 { edges.push((a, b, (random() % 4) as i64)); }
            } }
            let roots: Vec<_> = ids.iter().copied().filter(|_| random() % 3 != 0).collect();
            assert_eq!(bfs_walk(&edges, &roots), reference(&edges, &roots), "{edges:?}, {roots:?}");
        }
    }
}
