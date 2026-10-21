//! Admission counts packed bytes before copying any input columns.
use super::{Demand, Resident};
use std::{collections::HashSet, sync::Arc};

#[derive(Default)]
pub(in crate::code_rule_dsl::relational) struct Footprint {
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
    pub(in crate::code_rule_dsl::relational) fn admit(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        model::FileInput,
        relational::{
            producer::{Root, Tree},
            syntax::Facts,
        },
    };
    use std::sync::atomic::AtomicUsize;
    fn demand() -> Demand {
        let file = FileInput::new(
            "a.py".into(),
            "python".into(),
            Arc::new("def f(): return 1\n".into()),
            None,
            None,
            "test",
        )
        .unwrap();
        let facts = Facts::build(&file).unwrap();
        let visible = Arc::new((0..facts.subtree_ends().len() as u32).collect());
        Demand {
            roots: vec![Root { file: 0, node: 0 }],
            trees: vec![Tree { facts, visible }],
            paths: vec![file.path],
            work: Arc::new(AtomicUsize::new(100_000)),
        }
    }
    #[test]
    fn capacity_admission_is_transactional_and_shared_columns_are_counted_once() {
        let d = demand();
        let mut layout = Footprint::default();
        assert!(layout.admit(&d, usize::MAX));
        let first = layout.bytes;
        assert!(!layout.admit(&d, first + 23));
        assert_eq!(layout.bytes, first);
        assert!(layout.admit(&d, first + 24));
        assert_eq!(layout.bytes, first + 24);
        let different = demand();
        assert!(!layout.admit(&different, first + 48));
        assert_eq!(layout.bytes, first + 24);
    }
}
