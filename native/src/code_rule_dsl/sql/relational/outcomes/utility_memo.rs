//! Native utility identity and logical capture columns across private SQL namespaces.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct Contract {
    pub identity: usize,
    /// None is native's whole-environment memo scope.
    pub scope: Option<BTreeSet<String>>,
}
#[derive(Clone)]
pub(super) struct Layout {
    pub contracts: BTreeMap<usize, Contract>,
    columns: Vec<usize>,
    names: Vec<Option<String>>,
}
impl Layout {
    pub(super) fn new(
        slots: &[super::Slot],
        names: &[Option<String>],
        metadata_origins: &[Option<usize>],
        contracts: BTreeMap<usize, Contract>,
    ) -> Self {
        let mut logical = BTreeMap::new();
        let mut following = BTreeMap::new();
        let mut next = 0;
        let columns = names
            .iter()
            .enumerate()
            .map(|(physical, name)| {
                if let Some(name) = name {
                    *logical
                        .entry((name.clone(), slots[physical].kind))
                        .or_insert_with(|| {
                            let id = next;
                            next += 1;
                            id
                        })
                } else if let Some(origin) = metadata_origins[physical] {
                    // Matcher addresses group equivalent FOLLOWING contexts only
                    // at compile time. The emitted column is a stable ordinal.
                    *following.entry(origin).or_insert_with(|| {
                        let id = next;
                        next += 1;
                        id
                    })
                } else {
                    let id = next;
                    next += 1;
                    id
                }
            })
            .collect();
        Self {
            contracts,
            columns,
            names: names.to_vec(),
        }
    }
    pub(super) fn column(&self, physical: usize) -> usize {
        self.columns[physical]
    }
    pub(super) fn capture(&self, physical: usize, scope: Option<&BTreeSet<String>>) -> bool {
        self.names[physical]
            .as_ref()
            .is_some_and(|name| scope.is_none_or(|scope| scope.contains(name)))
    }
}
