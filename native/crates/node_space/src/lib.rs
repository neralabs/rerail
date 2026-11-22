//! A wave's node space and its node sets (decision 0024 §1): `space::NodeSpace`, and the chunked sets over it
//! (`space::chunked`, decision 0045). It reads nothing of the engine's, so it is a crate of its own, below the
//! transducer (rerail-transducer-base), which re-exports it.
//!
//! Its sources are in its own `src/`, at their module paths in the engine's tree (the crate split, step 9).
// The postings' decoder, which a container test reads its segments back with.
#[cfg(test)]
use rerail_store_base::content_store;
#[path = "code_rule_dsl"]
pub mod code_rule_dsl {
    pub mod space;
}
