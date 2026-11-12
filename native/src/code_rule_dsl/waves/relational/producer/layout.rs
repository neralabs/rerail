//! Admission counts packed bytes before copying any input columns.
use super::{Demand, Resident};
use std::{collections::HashSet, sync::Arc};

#[derive(Default)]
pub(in crate::code_rule_dsl::waves::relational) struct Footprint {
    documents: HashSet<usize>,
    selectors: HashSet<(usize, usize)>,
    bytes: usize,
}
impl Footprint {
    pub(super) fn retained(resident: &Resident) -> Self {
        Self {
            documents: resident.documents.keys().copied().collect(),
            selectors: resident.selectors.keys().copied().collect(),
            bytes: resident.resident_bytes,
        }
    }
    pub(in crate::code_rule_dsl::waves::relational) fn admit(
        &mut self,
        demand: &Demand,
        limit: usize,
    ) -> bool {
        self.admit_parts(&demand.trees, demand.roots.len(), limit)
    }
    pub(in crate::code_rule_dsl::waves::relational) fn admit_trees(
        &mut self,
        trees: &[super::Tree],
        limit: usize,
    ) -> bool {
        self.admit_parts(trees, 0, limit)
    }
    fn admit_parts(&mut self, trees: &[super::Tree], roots: usize, limit: usize) -> bool {
        let mut documents = HashSet::new();
        let mut selectors = HashSet::new();
        let mut extra = roots.saturating_mul(24);
        for tree in trees {
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

#[cfg(test)]
mod tests;
