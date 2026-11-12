//! Admission counts packed bytes before copying any input columns.
use super::{Demand, Resident};
use std::{collections::HashSet, sync::Arc};

#[derive(Default)]
pub struct Footprint {
    documents: HashSet<usize>,
    selectors: HashSet<(usize, usize)>,
    pub bytes: usize,
}
impl Footprint {
    pub(super) fn retained(resident: &Resident) -> Self {
        Self {
            documents: resident.documents.keys().copied().collect(),
            selectors: resident.selectors.keys().copied().collect(),
            bytes: resident.resident_bytes,
        }
    }
    pub fn admit(
        &mut self,
        demand: &Demand,
        limit: usize,
    ) -> bool {
        let mut documents = HashSet::new();
        let mut selectors = HashSet::new();
        let mut extra = demand.roots.len().saturating_mul(24);
        for tree in &demand.trees {
            let document = Arc::as_ptr(&tree.facts) as usize;
            let selector = (document, Arc::as_ptr(&tree.visible) as usize);
            if !self.documents.contains(&document) && documents.insert(document) {
                extra = extra.saturating_add(tree.facts.subtree_ends().len().saturating_mul(4));
            }
            if !self.selectors.contains(&selector) && selectors.insert(selector) {
                extra =
                    extra.saturating_add(tree.visible.len().saturating_mul(4).saturating_add(16));
            }
        }
        let Some(bytes) = self.bytes.checked_add(extra).filter(|&n| n <= limit) else {
            return false;
        };
        self.bytes = bytes;
        self.documents.extend(documents);
        self.selectors.extend(selectors);
        true
    }
}

// cratify: `tests` is rerail-engine's (it mounts it)
