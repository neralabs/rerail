//! Dependency components and expression effects, independent of executor storage.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Components {
    pub members: BTreeMap<String, BTreeSet<String>>,
    pub recursive: BTreeSet<String>,
}
impl Components {
    pub fn order(&self, edges: &BTreeMap<String, BTreeSet<String>>) -> Vec<BTreeSet<String>> {
        let mut remaining = edges.keys().cloned().collect::<BTreeSet<_>>();
        let mut order = vec![];
        while let Some(name) = remaining
            .iter()
            .find(|name| {
                edges[*name]
                    .iter()
                    .all(|r| !remaining.contains(r) || self.members[*name].contains(r))
            })
            .cloned()
        {
            let members = self.members[&name].clone();
            remaining.retain(|r| !members.contains(r));
            order.push(members);
        }
        debug_assert!(remaining.is_empty(), "component graph must be acyclic");
        order
    }
    pub fn analyze(edges: &BTreeMap<String, BTreeSet<String>>) -> Self {
        let mut reachable = BTreeMap::new();
        for name in edges.keys() {
            let mut seen = BTreeSet::new();
            let mut todo = edges[name].iter().cloned().collect::<Vec<_>>();
            while let Some(read) = todo.pop() {
                if seen.insert(read.clone()) {
                    todo.extend(edges.get(&read).into_iter().flatten().cloned());
                }
            }
            reachable.insert(name.clone(), seen);
        }
        let recursive = reachable
            .iter()
            .filter(|(name, reads)| reads.contains(*name))
            .map(|(name, _)| name.clone())
            .collect();
        let members = reachable
            .iter()
            .map(|(name, reads)| {
                let mut component = BTreeSet::from([name.clone()]);
                component.extend(
                    reads
                        .iter()
                        .filter(|read| reachable.get(*read).is_some_and(|r| r.contains(name)))
                        .cloned(),
                );
                (name.clone(), component)
            })
            .collect();
        Self { members, recursive }
    }
}

/// A proof for movement across dependency stages. Checked arithmetic, registered
/// functions, and ordered aggregates retain their original evaluation boundaries.
pub(in crate::code_rule_dsl) fn total_function(name: &str) -> bool {
    matches!(
        name,
        "length"
            | "substr"
            | "coalesce"
            | "min"
            | "max"
            | "count"
            | "sym_text"
            | "sym_atom"
            | "sym_cat"
            | "tape_nearest"
    )
}

pub(in crate::code_rule_dsl) fn checked_function(name: &str) -> bool {
    // SQLite abs has a defined integer-overflow error at i64::MIN. Its
    // evaluation stays in its original rule gate behind a materialization fence.
    name == "abs"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separates_dependency_chains_from_recursive_components() {
        let edges = [
            ("a", vec!["input"]),
            ("b", vec!["a"]),
            ("p", vec!["b", "q"]),
            ("q", vec!["p"]),
            ("out", vec!["q"]),
        ]
        .into_iter()
        .map(|(n, r)| (n.into(), r.into_iter().map(str::to_owned).collect()))
        .collect();
        let result = Components::analyze(&edges);
        assert_eq!(result.recursive, BTreeSet::from(["p".into(), "q".into()]));
        assert_eq!(result.members["p"], result.recursive);
        assert_eq!(result.members["b"], BTreeSet::from(["b".into()]));
        assert!(!total_function("abs"));
        assert!(!total_function("rr_checked_add"));
    }
}
