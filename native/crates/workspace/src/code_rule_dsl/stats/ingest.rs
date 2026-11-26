//! Planner statistics built at ingestion and persisted in the content store (docs/final-shape/03
//! §6; 05 Phase 3 row 7), ported from Typing-support's `stats.py`, which follows the SQL index prototype
//! (prior-work history index §2.6, Part 3 item 15):
//! - **Small aggregates maintained at load and persisted** (`t_kfreq` S178, `efreq` S277). Statistics
//!   held only in process memory made a reopened store plan differently (S174, S178).
//! - **Merged per batch, touching only the keys a batch changes**, so a one-shot build and an
//!   incremental one are identical (DH L8706-8708). A batch adds its documents' contribution or
//!   removes it, computed from their rows while the rows are present.
//! - **Read by the planner in one statement** (`stats/inputs.rs`, S259).
//!
//! T kept its facts as SQL tables per document and derived every aggregate from them. Here:
//! - `kind_size` (T's `stat_kind`) comes from the tape as its prefilter set is built (`prefilter::image`), in the
//!   same pass, and is counted with the store's other prefilter counts (`content_store::prefilter::kept`): a
//!   publication adds a tape's, and a removal or a collection takes them out again. `edge_freq` (`stat_edge`) went
//!   (t33 step 6): nothing read it.
//! - T's signed merge for fact relations stored as SQL tables (`fact_stats`, `fact_value_stats`, its `stat_col` and
//!   `stat_val`) went (t33 step 6): no fact relation is stored as a SQL table with a document column.
//!
//! Not kept: `stat_docrange` (a tape is its own document and its preorders are `0..nodes`) and the
//! fingerprint in `stat_meta` (the counts change in the transaction that changes the rows, so they
//! cannot describe other rows).

use crate::code_rule_dsl::facts::core::KIND_STRIDE as LANG_STRIDE;
use crate::code_rule_dsl::tape::TapeDoc;

/// One tape's kind sizes, accumulated node by node.
pub struct TapeStatistics {
    /// Per kind code, the sum of its nodes' descendant counts.
    pub sizes: Vec<u64>,
}

impl TapeStatistics {
    pub fn new() -> Self {
        Self { sizes: vec![0; LANG_STRIDE as usize] }
    }

    /// Adds node `pre`, of kind code `kind`.
    pub fn node(&mut self, doc: &TapeDoc, pre: u32, kind: u16) {
        self.sizes[usize::from(kind)] += u64::from(doc.transducer_descendants(pre).len() as u32);
    }
}

impl Default for TapeStatistics {
    fn default() -> Self {
        Self::new()
    }
}

// cratify: `tests` is rerail-engine's (it mounts it)
