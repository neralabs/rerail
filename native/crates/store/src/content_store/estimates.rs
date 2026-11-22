//! The demand's terminal matches' row estimate (t33 step 6): the most rows of one clause the last slice wave matched
//! (`waves::matches::LoweredWave::statistics`), which the next drive's demand relations plan with
//! (`demand::vtab::MatchRegistry::with_terminal_estimate`). One kept relation, `terminal_estimate(rows)`, one row in
//! partition 0 of its own space, where `fact_stats` held it.

use super::kept_facts::{BlockFlat, KeptPut, KeptWhat};
use super::{ContentStore, StoreResult};
use rerail_blocks::kept::{FactKey, KeptRead as _};
use sha2::{Digest as _, Sha256};

const ESTIMATE: &str = "terminal_estimate";

fn space() -> [u8; 32] {
    Sha256::digest(b"rerail.demand-estimates/v1").into()
}

fn ident() -> [u8; 32] {
    Sha256::digest(b"rerail.demand-estimates/terminal_estimate/v1").into()
}

impl ContentStore {
    /// Keeps `rows` as the terminal matches' estimate, in place of the last one, behind the run.
    pub fn keep_terminal_estimate(&self, rows: i64) {
        let put = KeptPut {
            rel: ESTIMATE.to_owned(),
            ident: ident(),
            part: 0,
            digest: rows as u64,
            reads: Vec::new(),
            types: b"i".to_vec(),
            what: KeptWhat::Picked(Box::new(BlockFlat::of_row(&[rows]))),
        };
        self.queue_kept(space(), vec![put]);
    }

    /// The terminal matches' estimate kept last (`keep_terminal_estimate`); `None` before one was. Read before a
    /// candidate transaction, as a kept read waits on the store's writer.
    pub fn terminal_estimate(&self) -> StoreResult<Option<i64>> {
        let Some(kept) = self.kept_space(&space())? else { return Ok(None) };
        let chain = kept.view();
        let ident = ident();
        let entry = chain.get(&FactKey { rel: ESTIMATE, ident: &ident, part: 0 });
        Ok(entry.filter(|e| !e.rows.is_empty()).map(|e| e.row(0)[0]))
    }
}
