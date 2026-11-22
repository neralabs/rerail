//! The tree's statistics, as the planner reads them (T's `stat_kind` and `stat_edge`): each kind's nodes and their
//! mean subtree size, and each child kind's parents with the share of its edges from each. A section (decision 0569
//! §4), each list sorted by kind and found by a search of it; shared by a component's model (`Model::fork`).
use rerail_section::{Archive, Section};
use std::sync::Arc;

/// A kind's nodes, and their mean subtree size (none for a kind with no nodes).
#[derive(Archive)]
#[archive(derive(Debug, PartialEq))]
pub struct Kind {
    pub name: String,
    pub nodes: i64,
    #[archive(flat)]
    pub size: Option<f64>,
}

/// A child kind's parents, each with the share of the child's edges from it, sorted by parent kind.
#[derive(Archive)]
#[archive(derive(Debug, PartialEq))]
pub struct Parents {
    pub child: String,
    pub parents: Vec<(String, f64)>,
}

/// The section: the kinds and the child kinds, each sorted by name; the nodes of every kind.
#[derive(Archive, Default)]
#[archive(derive(Debug, PartialEq))]
pub struct Of {
    pub kinds: Vec<Kind>,
    pub parents: Vec<Parents>,
    pub nodes: i64,
}

#[derive(Clone, PartialEq)]
pub struct TreeStats(Arc<Section<Of>>);

impl Default for TreeStats {
    fn default() -> TreeStats {
        TreeStats(Arc::new(Section::of(&Of::default())))
    }
}

impl std::fmt::Debug for TreeStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.get().fmt(f)
    }
}

impl TreeStats {
    /// The statistics of `kinds` (each kind's nodes and, where known, their summed subtree sizes) and `edges` (each
    /// (parent, child) pair's edges), by name.
    pub fn of(kinds: impl IntoIterator<Item = (String, i64, Option<i64>)>, edges: impl IntoIterator<Item = (String, String, i64)>) -> TreeStats {
        let mut ks: Vec<Kind> = kinds.into_iter().map(|(name, nodes, size)| Kind { name, nodes, size: size.filter(|_| nodes != 0).map(|size| size as f64 / nodes as f64) }).collect();
        ks.sort_by(|a, b| a.name.cmp(&b.name));
        ks.dedup_by(|a, b| a.name == b.name);
        let nodes = ks.iter().map(|k| k.nodes).sum();
        let mut es: Vec<(String, String, i64)> = edges.into_iter().map(|(p, c, n)| (c, p, n)).collect();
        es.sort();
        let mut parents: Vec<Parents> = Vec::new();
        for (c, p, n) in es {
            match parents.last_mut() {
                Some(last) if last.child == c => match last.parents.last_mut() {
                    Some((lp, ln)) if *lp == p => *ln += n as f64,
                    _ => last.parents.push((p, n as f64)),
                },
                _ => parents.push(Parents { child: c, parents: vec![(p, n as f64)] }),
            }
        }
        for c in &mut parents {
            let total: f64 = c.parents.iter().map(|(_, n)| n).sum();
            for (_, n) in &mut c.parents {
                *n /= total;
            }
        }
        TreeStats(Arc::new(Section::of(&Of { kinds: ks, parents, nodes })))
    }

    fn kind(&self, name: &str) -> Option<&ArchivedKind> {
        let ks = &self.0.kinds;
        ks.binary_search_by(|k| k.name.as_str().cmp(name)).ok().map(|i| &ks[i])
    }

    /// How many kinds it counts.
    pub fn len(&self) -> usize {
        self.0.kinds.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.kinds.is_empty()
    }
    /// The nodes of every kind.
    pub fn nodes(&self) -> i64 {
        self.0.nodes
    }
    /// `kind`'s nodes.
    pub fn freq(&self, kind: &str) -> Option<i64> {
        self.kind(kind).map(|k| k.nodes)
    }
    /// `kind`'s mean subtree size.
    pub fn size(&self, kind: &str) -> Option<f64> {
        self.kind(kind).and_then(|k| k.size)
    }
    /// Each kind, its nodes and its mean subtree size, by name.
    pub fn kinds(&self) -> impl Iterator<Item = (&str, i64, Option<f64>)> + '_ {
        self.0.kinds.iter().map(|k| (k.name.as_str(), k.nodes, k.size))
    }
    /// Each child kind and its parents with their shares, by name.
    pub fn parents(&self) -> impl Iterator<Item = (&str, impl Iterator<Item = (&str, f64)> + '_)> + '_ {
        self.0.parents.iter().map(|c| (c.child.as_str(), c.parents.iter().map(|p| (p.0.as_str(), p.1))))
    }
}
