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

mod kinds;
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

/// A tape's prefilter rows, from the tape alone.
pub(crate) fn image(tape_id: TapeId, doc: &TapeDoc, interrupted: &impl Fn() -> bool) -> StoreResult<TapePrefilter> {
    let lang = u32::from(language::tape_tag(*doc.get_lang()).ok_or_else(|| StoreError::Invalid("unregistered tape language".to_owned()))?);
    let mut positions = kinds::Builder::default();
    let mut counts = [0_u32; LANG_STRIDE as usize];
    for pre in 0..doc.transducer_len() as u32 {
        if pre.is_multiple_of(256) && interrupted() {
            return Err(StoreError::Invalid("index construction interrupted".to_owned()));
        }
        let kind = doc.transducer_kind(pre) & 1023;
        positions.push(kind, pre);
        counts[usize::from(kind)] += 1;
    }
    let kinds = counts
        .iter()
        .enumerate()
        .filter(|(_, n)| **n != 0)
        .map(|(code, &n)| (lang * LANG_STRIDE + code as u32, n, std::mem::take(&mut positions.segments[code])))
        .collect();
    let mut set = TapePrefilter {
        tape_id,
        lang,
        kinds,
        postings: Default::default(),
        cover: Default::default(),
        missing: Default::default(),
        residue: Default::default(),
    };
    values::build(&mut set, doc, interrupted)?;
    Ok(set)
}

/// Builds and publishes the prefilter rows of `docs`, the tapes `tapes` name, 64 at a time.
pub(crate) fn publish(store: &ContentStore, tapes: &[TapeId], docs: &[TapeDoc]) -> StoreResult<()> {
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

#[cfg(test)]
mod tests;
