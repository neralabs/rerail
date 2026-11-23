//! A compiled program's plan digests, named by one digest: what a kept entry is held by (`kept_facts::HOLDER`) and
//! what collection asks is live (`corpus_plans`). The facts memo stored these sets in a table; nothing stores them
//! now (t33 step 3).

use sha2::{Digest as _, Sha256};
use std::sync::Arc;

/// A compiled policy set's clause plan digests, in clause order; `digest` names the set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactsPlanSet {
    pub digest: [u8; 32],
    pub plans: Arc<[[u8; 32]]>,
}

impl FactsPlanSet {
    pub fn new(plans: Vec<[u8; 32]>) -> Self {
        let mut hash = Sha256::new();
        hash.update(b"rerail.facts-plan-set/v1\0");
        hash.update((plans.len() as u64).to_le_bytes());
        for plan in &plans {
            hash.update(plan);
        }
        Self { digest: hash.finalize().into(), plans: plans.into() }
    }
}
