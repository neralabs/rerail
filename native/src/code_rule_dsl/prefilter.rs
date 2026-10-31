//! The prefilter (docs/final-shape/03-architecture.md §6, §16; 05 Phase 3): SQL over per-tape
//! access paths selects candidate (tape, preorder) sets; transducers confirm them. Adapted from
//! Pulling-it-all-together's anchor store (`matcher/anchors/sql/store.rs`), which followed
//! ast-index-spike's `tape_store.py`.
//!
//! P's store owned a private database: a `file` table with a second copy of every tape, language,
//! kind and field catalogs, overflow tables and decoded-property tables. Here a tape's index image
//! is built from the tape alone (`image`) and published into the content store keyed by its
//! `tape_id` (`content_store::prefilter`), where the rows die with the tape. The catalogs are the
//! grammar's own, the tape keeps its overflows, and decoding moves into transducers (§7).

use crate::code_rule_dsl::{language, tape::TapeDoc};
use crate::content_store::{ContentStore, StoreError, StoreResult, TapeId, TapePrefilter};
use ast_grep_core::Doc;

mod candidates;
mod vocabulary;
mod execute;
mod graph;
mod kinds;
mod lab;
mod lower;
pub(crate) mod plans;
pub(crate) mod probes;
#[path = "prefilter/lower/audit.rs"]
mod audit;
pub(in crate::code_rule_dsl) use audit::candidate_frontier_audit;
// P's incremental BLOB reader and `nera_tree`, for the §17 pure-SQL fallback.
#[allow(dead_code)]
mod reader;
#[allow(dead_code)]
mod tree;
mod values;

pub(crate) use crate::code_rule_dsl::facts::core::KIND_STRIDE as LANG_STRIDE;

/// A kind atom as an FTS token: `k` and base 36.
pub(crate) fn kind_token(code: u32) -> String {
    let mut digits = [0_u8; 7];
    let mut at = digits.len();
    let mut code = code;
    loop {
        at -= 1;
        digits[at] = b"0123456789abcdefghijklmnopqrstuvwxyz"[(code % 36) as usize];
        code /= 36;
        if code == 0 {
            break;
        }
    }
    format!("k{}", std::str::from_utf8(&digits[at..]).expect("base36 ASCII"))
}

/// The kind code of node `pre` in the prefilter's atoms. A restored tape reports ERROR as
/// tree-sitter's symbol (u16::MAX); the atoms use its tape code, 0, which no other node has.
pub(in crate::code_rule_dsl) fn kind_code(doc: &TapeDoc, pre: u32) -> u16 {
    if doc.transducer_flags(pre) & 8 != 0 { 0 } else { doc.transducer_kind(pre) }
}

/// A tape's prefilter rows, from the tape alone.
pub(in crate::code_rule_dsl) fn image(tape_id: TapeId, doc: &TapeDoc, interrupted: &impl Fn() -> bool) -> StoreResult<TapePrefilter> {
    let lang = u32::from(language::tape_tag(*doc.get_lang()).ok_or_else(|| StoreError::Invalid("unregistered tape language".to_owned()))?);
    let mut positions = kinds::Builder::default();
    let mut counts = [0_u32; LANG_STRIDE as usize];
    let mut statistics = crate::code_rule_dsl::stats::ingest::TapeStatistics::new(lang);
    for pre in 0..doc.transducer_len() as u32 {
        if pre.is_multiple_of(256) && interrupted() {
            return Err(StoreError::Invalid("index construction interrupted".to_owned()));
        }
        let kind = kind_code(doc, pre);
        positions.push(kind, pre);
        counts[usize::from(kind)] += 1;
        statistics.node(doc, pre, kind);
    }
    let kinds = counts
        .iter()
        .enumerate()
        .filter(|(_, n)| **n != 0)
        .map(|(code, &n)| (lang * LANG_STRIDE + code as u32, n, statistics.sizes[code], std::mem::take(&mut positions.segments[code])))
        .collect();
    let mut set = TapePrefilter {
        tape_id,
        lang,
        nodes: doc.transducer_len() as u32,
        kinds,
        edges: statistics.edges(),
        postings: Default::default(),
        cover: Default::default(),
        missing: Default::default(),
        residue: Default::default(),
    };
    values::build(&mut set, doc, interrupted)?;
    Ok(set)
}

/// Builds and publishes the prefilter rows of `docs`, the tapes `tapes` name, 64 at a time.
pub(in crate::code_rule_dsl) fn publish(store: &ContentStore, tapes: &[TapeId], docs: &[TapeDoc]) -> StoreResult<()> {
    if tapes.len() != docs.len() {
        return Err(StoreError::Invalid("tape identity count does not match prefilter documents".to_owned()));
    }
    for (tapes, docs) in tapes.chunks(64).zip(docs.chunks(64)) {
        let sets = tapes
            .iter()
            .zip(docs)
            .map(|(&tape, doc)| image(tape, doc, &crate::code_rule_dsl::budget::matcher_budget_exhausted))
            .collect::<StoreResult<Vec<_>>>()?;
        store.put_prefilters_bulk(&sets)?;
    }
    Ok(())
}

/// The candidates of `plans`, all of language `lang` (a tape tag), over `tapes`: per plan, its
/// (tape, preorder) rows in order. Tapes of another language or without prefilter rows have none.
/// A superset of each plan's roots (`lower.rs`); the matcher confirms them.
pub(in crate::code_rule_dsl) fn select(
    store: &ContentStore,
    lang: u32,
    plans: &[&crate::code_rule_dsl::matcher::AnchorPlan],
    tapes: &[TapeId],
) -> StoreResult<Vec<Vec<(i64, u32)>>> {
    let files = tapes.iter().map(|tape| tape.row()).collect::<Vec<_>>();
    // Statistics only order the lowering, so reading them in their own snapshot is safe.
    use crate::code_rule_dsl::prof::time;
    let statistics = time("select.statistics", || {
        store.candidate_transaction(false, |db| {
            crate::code_rule_dsl::stats::inputs::Statistics::read(db, plans.iter().map(|plan| (lang, *plan)))
        })
    })?;
    let statement = time("select.lower", || lower::batch(plans, lang, &files, &statistics));
    time("select.capture", || lab::capture::save(store, &statement))?;
    let mut rows = vec![Vec::new(); plans.len()];
    time("select.execute", || execute::candidates(store, &statement, |plan, tape, pre| rows[plan].push((tape, pre as u32))))?;
    crate::code_rule_dsl::prof::count("select.rows", rows.iter().map(Vec::len).sum());
    Ok(rows)
}

#[cfg(test)]
pub(in crate::code_rule_dsl) mod tests;
