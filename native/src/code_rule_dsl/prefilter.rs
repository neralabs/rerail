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
mod gates;
mod graph;
mod kinds;
mod lab;
mod lower;
pub(crate) mod plans;
/// The store's crate holds it (its vocabulary uses it too).
#[path = "prefilter/lower/audit.rs"]
mod audit;
pub(in crate::code_rule_dsl) use audit::candidate_frontier_audit;
// P's incremental BLOB reader and `nera_tree`, for the §17 pure-SQL fallback; `tapes` is the
// store's tapes as `nera_tree` reads them.
#[allow(dead_code)]
mod reader;
pub(in crate::code_rule_dsl) mod sets;
pub(in crate::code_rule_dsl) mod terms;
pub(crate) mod tapes;
mod tree;
mod values;

pub(crate) use crate::code_rule_dsl::facts::core::KIND_STRIDE as LANG_STRIDE;

pub(crate) use super::search_index::kind_code;
#[cfg(any(test, feature = "test-hooks"))]
use super::search_index::kind_token;


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
        .map(|(code, &n)| (lang * LANG_STRIDE + code as u32, n, statistics.sizes[code], positions.segment(code)))
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
#[cfg_attr(not(any(test, feature = "test-hooks")), allow(dead_code))]
pub(in crate::code_rule_dsl) fn select(
    store: &ContentStore,
    lang: u32,
    plans: &[&crate::code_rule_dsl::matcher::AnchorPlan],
    tapes: &[TapeId],
) -> StoreResult<Vec<Vec<(i64, u32)>>> {
    let mut rows = select_indexed(store, lang, plans, tapes)?
        .into_iter()
        .map(|rows| rows.into_iter().map(|(file, pre)| (tapes[file as usize].row(), pre)).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    // In order per plan (the statement's arms stream unsorted, `lower::batch`).
    use rayon::prelude::*;
    rows.par_iter_mut().for_each(|rows| rows.sort_unstable());
    Ok(rows)
}

/// `select`'s rows as (index in `tapes`, preorder), unordered: 8 bytes a row, not 16. On Metal the
/// rows are held while the labels' GPU runs are (06 S7: 1.15 M rows, at the footprint's peak).
pub(in crate::code_rule_dsl) fn select_indexed(
    store: &ContentStore,
    lang: u32,
    plans: &[&crate::code_rule_dsl::matcher::AnchorPlan],
    tapes: &[TapeId],
) -> StoreResult<Vec<Vec<(u32, u32)>>> {
    let (space, sets) = select_sets(store, lang, plans, tapes, sets::Device::Cpu)?;
    let rows = sets
        .iter()
        .map(|set| (0..space.files()).flat_map(|file| set.file_nodes(&space, file).map(move |pre| (file as u32, pre))).collect::<Vec<_>>())
        .collect::<Vec<_>>();
    crate::code_rule_dsl::prof::count("select.rows", rows.iter().map(Vec::len).sum());
    Ok(rows)
}

/// Each plan's candidates over `tapes` as sets over their node space (`sets`; decision 0024 §2).
pub(in crate::code_rule_dsl) fn select_sets(
    store: &ContentStore,
    lang: u32,
    plans: &[&crate::code_rule_dsl::matcher::AnchorPlan],
    tapes: &[TapeId],
    device: sets::Device,
) -> StoreResult<(crate::code_rule_dsl::space::NodeSpace, Vec<crate::code_rule_dsl::space::NodeSet>)> {
    let files = tapes.iter().map(|tape| tape.row()).collect::<Vec<_>>();
    crate::code_rule_dsl::prof::time("select.sets", || sets::select(store, lang, plans, &files, device))
}

/// `select_indexed` by the candidate SQL (`lower::batch`, `execute`): the reference the set
/// program is checked against, and the anchor lab's subject.
pub(in crate::code_rule_dsl) fn select_indexed_sql(
    store: &ContentStore,
    lang: u32,
    plans: &[&crate::code_rule_dsl::matcher::AnchorPlan],
    tapes: &[TapeId],
) -> StoreResult<Vec<Vec<(u32, u32)>>> {
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
    let index = files.iter().enumerate().map(|(file, &row)| (row, file as u32)).collect::<std::collections::HashMap<_, _>>();
    let mut rows = vec![Vec::new(); plans.len()];
    let mut outside = None;
    time("select.execute", || {
        execute::candidates(store, &statement, |plan, tape, pre| match index.get(&tape) {
            Some(&file) => rows[plan].push((file, pre as u32)),
            None => outside = Some(tape),
        })
    })?;
    if let Some(tape) = outside {
        return Err(StoreError::Corrupt(format!("candidate of tape {tape} outside the chunk")));
    }
    crate::code_rule_dsl::prof::count("select.rows", rows.iter().map(Vec::len).sum());
    Ok(rows)
}

/// The pure-SQL fallback (03 §17): the same plans lowered to SQL alone, relations read from the
/// tape (`nera_tree`), over `tapes`. Per plan, its (index in `tapes`, preorder) rows and whether
/// they are exactly its rule's roots; where not, they are a superset, as `select`'s. The wave's
/// `Candidates::Sql` route (`RERAIL_CANDIDATES=sql`) drives it.
pub(in crate::code_rule_dsl) fn fallback(
    store: &ContentStore,
    lang: u32,
    plans: &[&crate::code_rule_dsl::matcher::AnchorPlan],
    tapes: &[TapeId],
) -> StoreResult<Vec<(Vec<(u32, u32)>, bool)>> {
    let files = tapes.iter().map(|tape| tape.row()).collect::<Vec<_>>();
    let statistics = store.candidate_transaction(false, |db| {
        crate::code_rule_dsl::stats::inputs::Statistics::read(db, plans.iter().map(|plan| (lang, *plan)))
    })?;
    let statement = lower::fallback(plans, lang, &files, &statistics);
    let index = files.iter().enumerate().map(|(file, &row)| (row, file as u32)).collect::<std::collections::HashMap<_, _>>();
    let mut rows = vec![Vec::new(); plans.len()];
    let mut outside = None;
    execute::candidates(store, &statement, |plan, tape, pre| match index.get(&tape) {
        Some(&file) => rows[plan].push((file, pre as u32)),
        None => outside = Some(tape),
    })?;
    if let Some(tape) = outside {
        return Err(StoreError::Corrupt(format!("candidate of tape {tape} outside the chunk")));
    }
    Ok(rows.into_iter().zip(statement.exact).map(|(mut rows, exact)| {
        rows.sort_unstable();
        (rows, exact)
    }).collect())
}

#[cfg(test)]
pub(in crate::code_rule_dsl) mod tests;
