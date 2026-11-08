//! `components(d, c, key, edge, own)` (`core.rr`, decision 0074 §2): each document's component of the
//! graph `edge(d, e)` (d depends on e) and the component's Merkle key.
//!
//! - A component is a strongly connected component; `c` is its smallest document.
//! - Its key is the SHA-256 of its documents' `own` digests, sorted, then the keys of the components
//!   it depends on, sorted. Equal keys therefore mean equal digests over the whole closure.
//!
//! Components come from an iterative Tarjan pass, which finishes a component only after every
//! component it reaches, so keys are made bottom up in one pass: linear in the edges. A document
//! named only by an edge has the empty digest.
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The schema of the key: bumped when its meaning changes.
const DOMAIN: &[u8] = b"rerail.components/v1\0";

/// `(d, c, key)` for every document of `own` or `edge`, by document.
pub(in crate::code_rule_dsl) fn components(edges: &[(i64, i64)], own: &[(i64, String)]) -> Vec<(i64, i64, String)> {
    let mut digests: BTreeMap<i64, BTreeSet<&str>> = BTreeMap::new();
    for (d, h) in own {
        digests.entry(*d).or_default().insert(h.as_str());
    }
    for (a, b) in edges {
        digests.entry(*a).or_default();
        digests.entry(*b).or_default();
    }
    let docs: Vec<i64> = digests.keys().copied().collect();
    let index: BTreeMap<i64, usize> = docs.iter().enumerate().map(|(i, d)| (*d, i)).collect();
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); docs.len()];
    for (a, b) in edges {
        if a != b {
            out[index[a]].push(index[b]);
        }
    }
    for o in &mut out {
        o.sort_unstable();
        o.dedup();
    }
    let sccs = tarjan(&out);
    let mut component = vec![usize::MAX; docs.len()];
    let mut keys: Vec<String> = Vec::with_capacity(sccs.len());
    let mut rows = Vec::with_capacity(docs.len());
    for (k, members) in sccs.iter().enumerate() {
        for &m in members {
            component[m] = k;
        }
        // Every successor outside the component is finished, so its key is known.
        let below: BTreeSet<&str> = members
            .iter()
            .flat_map(|&m| &out[m])
            .filter(|&&s| component[s] != k)
            .map(|&s| keys[component[s]].as_str())
            .collect();
        let mut own: Vec<&str> = members.iter().flat_map(|&m| digests[&docs[m]].iter().copied()).collect();
        own.sort_unstable();
        let mut hash = Sha256::new();
        hash.update(DOMAIN);
        for (n, items) in [(own.len(), own), (below.len(), below.into_iter().collect())] {
            hash.update((n as u64).to_le_bytes());
            for item in items {
                hash.update((item.len() as u64).to_le_bytes());
                hash.update(item.as_bytes());
            }
        }
        let key: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
        let c = members.iter().map(|&m| docs[m]).min().expect("a component has a member");
        rows.extend(members.iter().map(|&m| (docs[m], c, key.clone())));
        keys.push(key);
    }
    rows.sort_unstable();
    rows
}

/// The strongly connected components of `out`, each finished after every component it reaches.
fn tarjan(out: &[Vec<usize>]) -> Vec<Vec<usize>> {
    const NONE: usize = usize::MAX;
    let n = out.len();
    let (mut order, mut low, mut on) = (vec![NONE; n], vec![0; n], vec![false; n]);
    let (mut stack, mut sccs, mut next) = (Vec::new(), Vec::new(), 0);
    for root in 0..n {
        if order[root] != NONE {
            continue;
        }
        // (node, next successor to visit)
        let mut frames = vec![(root, 0)];
        order[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on[root] = true;
        while let Some(&(v, at)) = frames.last() {
            if let Some(&w) = out[v].get(at) {
                frames.last_mut().expect("a frame").1 += 1;
                if order[w] == NONE {
                    order[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on[w] = true;
                    frames.push((w, 0));
                } else if on[w] {
                    low[v] = low[v].min(order[w]);
                }
                continue;
            }
            frames.pop();
            if let Some(&(parent, _)) = frames.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == order[v] {
                let mut scc = Vec::new();
                loop {
                    let w = stack.pop().expect("v is on the stack");
                    on[w] = false;
                    scc.push(w);
                    if w == v {
                        break;
                    }
                }
                sccs.push(scc);
            }
        }
    }
    sccs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(edges: &[(i64, i64)], own: &[(i64, &str)]) -> BTreeMap<i64, (i64, String)> {
        let own: Vec<(i64, String)> = own.iter().map(|(d, h)| (*d, (*h).to_owned())).collect();
        components(edges, &own).into_iter().map(|(d, c, k)| (d, (c, k))).collect()
    }

    #[test]
    fn cycles_share_a_component_and_keys_cover_the_closure() {
        // 1 -> 2 <-> 3 -> 4; 5 -> 4 (a diamond's other side); 6 alone.
        let edges = [(1, 2), (2, 3), (3, 2), (3, 4), (5, 4)];
        let own = [(1, "a"), (2, "b"), (3, "c"), (4, "d"), (5, "e"), (6, "f")];
        let base = run(&edges, &own);
        assert_eq!(base[&2].0, 2);
        assert_eq!(base[&3], base[&2], "a cycle is one component with one key");
        assert_eq!(base[&1].0, 1);
        assert_ne!(base[&1].1, base[&2].1);
        // Editing 4 changes the keys of everything that reaches it, and nothing else.
        let edited = run(&edges, &[(1, "a"), (2, "b"), (3, "c"), (4, "D"), (5, "e"), (6, "f")]);
        for d in [1, 2, 3, 4, 5] {
            assert_ne!(base[&d].1, edited[&d].1, "{d} reaches 4");
        }
        assert_eq!(base[&6], edited[&6]);
        // Editing 1, which nothing imports, changes only its own key.
        let top = run(&edges, &[(1, "A"), (2, "b"), (3, "c"), (4, "d"), (5, "e"), (6, "f")]);
        assert_ne!(base[&1].1, top[&1].1);
        for d in [2, 3, 4, 5, 6] {
            assert_eq!(base[&d], top[&d]);
        }
        // A new import changes the importer's key.
        let more = run(&[(1, 2), (2, 3), (3, 2), (3, 4), (5, 4), (6, 5)], &own);
        assert_ne!(base[&6].1, more[&6].1);
        assert_eq!(base[&5], more[&5]);
        // The key is a function of the facts, not of their order.
        let mut reversed = edges.to_vec();
        reversed.reverse();
        assert_eq!(run(&reversed, &own), base);
    }

    #[test]
    fn a_long_chain_does_not_recurse() {
        let edges: Vec<(i64, i64)> = (1..200_000).map(|d| (d, d + 1)).collect();
        let rows = components(&edges, &[]);
        assert_eq!(rows.len(), 200_000);
        assert!(rows.iter().all(|(d, c, _)| d == c));
    }
}
