//! Planner statistics built at ingestion and persisted in the content store (docs/final-shape/03
//! §6; 05 Phase 3 row 7), ported from Typing-support's `stats.py`, which follows ast-index-spike
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
//! - `fact_stats`/`fact_value_stats` (`stat_col`/`stat_val`) keep T's signed merge for fact
//!   relations stored as SQL tables with a document column ([`merge_facts`]).
//!
//! Not kept: `stat_docrange` (a tape is its own document and its preorders are `0..nodes`) and the
//! fingerprint in `stat_meta` (the counts change in the transaction that changes the rows, so they
//! cannot describe other rows).

use crate::code_rule_dsl::prefilter::{LANG_STRIDE, kind_code};
use crate::code_rule_dsl::tape::TapeDoc;
use rusqlite::{Connection, Result, params};
use std::collections::BTreeMap;

/// One tape's kind sizes and direct edges, accumulated node by node.
pub(in crate::code_rule_dsl) struct TapeStatistics {
    lang: u32,
    /// Per kind code, the sum of its nodes' descendant counts.
    pub sizes: Vec<u64>,
    /// (parent kind atom, child kind atom, field) → edges.
    pub edges: BTreeMap<(u32, u32, u16), u32>,
}

impl TapeStatistics {
    pub(in crate::code_rule_dsl) fn new(lang: u32) -> Self {
        Self { lang, sizes: vec![0; LANG_STRIDE as usize], edges: BTreeMap::new() }
    }

    /// Adds node `pre`, of kind code `kind`.
    pub(in crate::code_rule_dsl) fn node(&mut self, doc: &TapeDoc, pre: u32, kind: u16) {
        self.sizes[usize::from(kind)] += u64::from(doc.transducer_descendants(pre).len() as u32);
        if let Some(parent) = doc.transducer_parent(pre) {
            let atom = |code: u16| self.lang * LANG_STRIDE + u32::from(code);
            let key = (atom(kind_code(doc, parent)), atom(kind), doc.transducer_field(pre));
            *self.edges.entry(key).or_default() += 1;
        }
    }

    pub(in crate::code_rule_dsl) fn edges(&self) -> Vec<(u32, u32, u16, u32)> {
        self.edges.iter().map(|(&(parent, child, field), &n)| (parent, child, field, n)).collect()
    }
}

/// A fact relation stored as a SQL table: its document column, its document-local columns (node
/// ids and offsets, which mean something only within their document) and its value columns.
pub(in crate::code_rule_dsl) struct FactTable<'a> {
    pub name: &'a str,
    pub doc: &'a str,
    pub local: &'a [&'a str],
    pub values: &'a [&'a str],
}

/// Adds (`sign` = 1) or removes (`sign` = -1) the contribution of documents `docs` to the column
/// statistics of `tables`, computed from their rows, which must be present. T's `merge`, minus kinds
/// and edges, which come from tapes.
pub(in crate::code_rule_dsl) fn merge_facts(db: &Connection, tables: &[FactTable<'_>], docs: &[i64], sign: i64) -> Result<()> {
    db.execute_batch("DROP TABLE IF EXISTS temp.fact_batch; CREATE TEMP TABLE fact_batch(d INTEGER PRIMARY KEY)")?;
    {
        let mut insert = db.prepare("INSERT OR IGNORE INTO temp.fact_batch VALUES (?1)")?;
        for doc in docs {
            insert.execute([doc])?;
        }
    }
    for table in tables {
        let (tbl, d) = (table.name, table.doc);
        let src = format!("temp.fact_batch b CROSS JOIN \"{tbl}\" x ON x.\"{d}\" = b.d");
        for col in table.local {
            // Grouped per document, so per-document sums are corpus sums (an offset in two
            // documents is two values).
            let (n, ndv, sq): (i64, i64, i64) = db.query_row(
                &format!(
                    "SELECT coalesce(sum(c), 0), count(*), coalesce(sum(c * c), 0) FROM \
                     (SELECT count(*) AS c FROM {src} GROUP BY x.\"{d}\", x.\"{col}\")"
                ),
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?;
            add_col(db, tbl, col, sign * n, sign * ndv, sign * sq, sign * sq)?;
        }
        for col in table.values {
            // Keyed, so each touched value is one probe into fact_value_stats' key and back (an
            // unkeyed temp table made the UPDATE ... FROM a scan of it per row: quadratic in the
            // vocabulary).
            db.execute_batch(
                "DROP TABLE IF EXISTS temp.fact_dv; CREATE TEMP TABLE fact_dv(val PRIMARY KEY, n, docs, sqd) WITHOUT ROWID",
            )?;
            db.execute(
                &format!(
                    "INSERT INTO temp.fact_dv SELECT v, sum(c), count(*), sum(c * c) FROM \
                     (SELECT x.\"{d}\" AS d, x.\"{col}\" AS v, count(*) AS c FROM {src} GROUP BY 1, 2) GROUP BY v"
                ),
                [],
            )?;
            // sq and ndv change only through the values this batch touches: sum(n^2) moves by
            // new^2 - old^2.
            let touched = |condition: &str| -> Result<(i64, i64)> {
                db.query_row(
                    &format!(
                        "SELECT coalesce(sum(v.n * v.n), 0), count(v.n) FROM temp.fact_dv t LEFT JOIN fact_value_stats v \
                         ON v.tbl = ?1 AND v.col = ?2 AND v.val = t.val {condition}"
                    ),
                    params![tbl, col],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
            };
            let old = touched("")?;
            db.execute(
                "INSERT INTO fact_value_stats SELECT ?1, ?2, val, 0, 0, 0 FROM temp.fact_dv WHERE true \
                 ON CONFLICT (tbl, col, val) DO NOTHING",
                params![tbl, col],
            )?;
            db.execute(
                "UPDATE fact_value_stats AS v SET n = v.n + ?1 * t.n, docs = v.docs + ?1 * t.docs, sqd = v.sqd + ?1 * t.sqd \
                 FROM temp.fact_dv t WHERE v.tbl = ?2 AND v.col = ?3 AND v.val = t.val",
                params![sign, tbl, col],
            )?;
            let new = touched("AND v.n > 0")?;
            db.execute("DELETE FROM fact_value_stats WHERE tbl = ?1 AND col = ?2 AND n = 0", params![tbl, col])?;
            let (n, sqd): (i64, i64) = db.query_row(
                "SELECT coalesce(sum(n), 0), coalesce(sum(sqd), 0) FROM temp.fact_dv",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            add_col(db, tbl, col, sign * n, new.1 - old.1, new.0 - old.0, sign * sqd)?;
        }
    }
    db.execute("DELETE FROM fact_stats WHERE rows = 0", [])?;
    db.execute_batch("DROP TABLE temp.fact_batch; DROP TABLE IF EXISTS temp.fact_dv")
}

fn add_col(db: &Connection, tbl: &str, col: &str, rows: i64, ndv: i64, sq: i64, sqd: i64) -> Result<()> {
    db.execute(
        "INSERT INTO fact_stats VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT (tbl, col) DO UPDATE SET \
         rows = rows + excluded.rows, ndv = ndv + excluded.ndv, sq = sq + excluded.sq, sqd = sqd + excluded.sqd",
        params![tbl, col, rows, ndv, sq, sqd],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests;
