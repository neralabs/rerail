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
//! - `kind_size` (T's `stat_kind`) and `edge_freq` (`stat_edge`) come from the tape as its prefilter
//!   set is built (`prefilter::image`), in the same pass. Publication adds them; removal, including
//!   a collected tape's cascade, subtracts them in triggers: kind sizes from `tape_kind_counts.size`,
//!   edges from the set's own `edges` segment (PIAT spec §2.5's per-document `edge_counts`).
//! - T's signed merge for fact relations stored as SQL tables (`fact_stats`, `fact_value_stats`, its `stat_col` and
//!   `stat_val`) went (t33 step 6): no fact relation is stored as a SQL table with a document column.
//!
//! Not kept: `stat_docrange` (a tape is its own document and its preorders are `0..nodes`) and the
//! fingerprint in `stat_meta` (the counts change in the transaction that changes the rows, so they
//! cannot describe other rows).

use crate::code_rule_dsl::facts::core::KIND_STRIDE as LANG_STRIDE;
use crate::code_rule_dsl::search_index::kind_code;
use crate::code_rule_dsl::tape::TapeDoc;
use rusqlite::{Connection, Result, params};
use std::collections::BTreeMap;

/// One tape's kind sizes and direct edges, accumulated node by node.
pub struct TapeStatistics {
    lang: u32,
    /// Per kind code, the sum of its nodes' descendant counts.
    pub sizes: Vec<u64>,
    /// (parent kind atom, child kind atom, field) → edges.
    pub edges: BTreeMap<(u32, u32, u16), u32>,
}

impl TapeStatistics {
    pub fn new(lang: u32) -> Self {
        Self { lang, sizes: vec![0; LANG_STRIDE as usize], edges: BTreeMap::new() }
    }

    /// Adds node `pre`, of kind code `kind`, its parent `parent` (a preorder pass's, `TapeDoc::transducer_parents`).
    pub fn node(&mut self, doc: &TapeDoc, pre: u32, kind: u16, parent: Option<u32>) {
        self.sizes[usize::from(kind)] += u64::from(doc.transducer_descendants(pre).len() as u32);
        if let Some(parent) = parent {
            let atom = |code: u16| self.lang * LANG_STRIDE + u32::from(code);
            let key = (atom(kind_code(doc, parent)), atom(kind), doc.transducer_field(pre));
            *self.edges.entry(key).or_default() += 1;
        }
    }

    pub fn edges(&self) -> Vec<(u32, u32, u16, u32)> {
        self.edges.iter().map(|(&(parent, child, field), &n)| (parent, child, field, n)).collect()
    }
}

// cratify: `tests` is rerail-engine's (it mounts it)
