//! `R[tid]` (03 §16 item 4, list step 17): a walk's text test, as a node set read from the term
//! index instead of each node's text. A test is lowered by the text layer (decision 0042); its plan
//! runs over the language's vocabulary once (`text::vocabulary::fill`, kept by plan key and brought
//! up to date over new terms), and its nodes on a tape are the union of the matched terms' postings
//! plus the nodes the terms do not cover (`term_coverage.missing`), decided by one pass over the
//! file (`text::scan`). Decision 0026: no term id is stored per node.
//!
//! The rows are attached to the wave's tapes (`TapeDoc::with_texts`), where every walk's text
//! slots read them (`Micro::slot`) by the slot's key (`Micro::slot_key`); a slot the text layer
//! cannot lower (a Lua pattern) is computed on the text as before.

use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::text::{Lowered, scan, vocabulary};
use crate::content_store::{ContentStore, StoreResult};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// A text test the term index answers.
pub(in crate::code_rule_dsl) struct TextSlot {
    /// The slot's key (`Micro::slot_key`): equal tests share their rows.
    pub(in crate::code_rule_dsl) key: String,
    /// The test in the text layer: equal plans share their terms. The slot's bits cover every node,
    /// whatever kind the walks test first.
    pub(in crate::code_rule_dsl) lowered: Arc<Lowered>,
}

/// Per tape (in `tapes`' order), each slot's bits by key; a slot the text layer could not decide on
/// a tape (a backtracking pattern past its limits) is left out there, and the walk computes it.
pub(in crate::code_rule_dsl) fn rows(
    store: &ContentStore,
    lang: u32,
    tapes: &[(i64, &TapeDoc)],
    slots: &[TextSlot],
) -> StoreResult<Vec<BTreeMap<String, Arc<Vec<u64>>>>> {
    let mut out = vec![BTreeMap::new(); tapes.len()];
    if slots.is_empty() || tapes.is_empty() {
        return Ok(out);
    }
    let lowered = slots.iter().map(|slot| Arc::clone(&slot.lowered)).collect::<Vec<_>>();
    // Each plan's matching terms, by plan key.
    let terms = vocabulary::fill(store, lang, &lowered)?;
    let lang = i64::from(lang);
    let index = tapes.iter().enumerate().map(|(at, (id, _))| (*id, at)).collect::<HashMap<_, _>>();
    // Per plan key, per tape: the matched terms' nodes; and per tape the uncovered nodes.
    let (found, missing) = store.candidate_transaction(false, |db| {
        let _stage = crate::code_rule_dsl::prof::Stage::start("terms.postings");
        let mut postings = db.prepare_cached("SELECT tape_id, postings FROM term_postings WHERE lang=?1 AND term=?2")?;
        let mut found = HashMap::<&crate::code_rule_dsl::text::Key, Vec<Vec<u32>>>::new();
        for slot in slots {
            let key = &slot.lowered.key;
            if found.contains_key(key) {
                continue;
            }
            let mut nodes = vec![Vec::new(); tapes.len()];
            for &term in terms.get(key).map(Vec::as_slice).unwrap_or(&[]) {
                let mut rows = postings.query(rusqlite::params![lang, term])?;
                while let Some(row) = rows.next()? {
                    if let Some(&at) = index.get(&row.get::<_, i64>(0)?) {
                        decode(row.get_ref(1)?.as_blob()?, &mut nodes[at])?;
                    }
                }
            }
            found.insert(key, nodes);
        }
        let mut uncovered = db.prepare_cached("SELECT missing FROM term_coverage WHERE tape_id=?1 AND length(missing)>0")?;
        let mut missing = vec![Vec::new(); tapes.len()];
        for (at, (id, _)) in tapes.iter().enumerate() {
            let mut rows = uncovered.query([id])?;
            while let Some(row) = rows.next()? {
                decode(row.get_ref(0)?.as_blob()?, &mut missing[at])?;
            }
            missing[at].sort_unstable();
            missing[at].dedup();
        }
        Ok((found, missing))
    })?;
    let _stage = crate::code_rule_dsl::prof::Stage::start("terms.rows");
    // Every slot's scan of a file in one batch: their literal plans share one pass.
    let batch = scan::Batch::of(lowered);
    use rayon::prelude::*;
    out.par_iter_mut().enumerate().for_each(|(at, rows)| {
        let tape = tapes[at].1;
        let len = tape.transducer_len();
        let span = |node: u32| tape.transducer_span(node);
        let uncovered = missing[at].iter().copied().filter(|&node| (node as usize) < len).collect::<Vec<_>>();
        // The uncovered nodes, decided by one pass over the file's text for every slot.
        let nodes = scan::Nodes { text: tape.transducer_source(), len, candidates: &uncovered, span: &span };
        let scanned = batch.bits(&nodes);
        for (slot, bits) in slots.iter().zip(scanned) {
            let Some(mut bits) = bits else { continue };
            for &node in found[&slot.lowered.key][at].iter().filter(|&&node| (node as usize) < len) {
                bits[node as usize / 64] |= 1 << (node % 64);
            }
            rows.insert(slot.key.clone(), Arc::new(bits));
        }
    });
    crate::code_rule_dsl::prof::count("terms.slots", slots.len());
    crate::code_rule_dsl::prof::count("terms.uncovered", missing.iter().map(Vec::len).sum());
    Ok(out)
}

/// A posting segment's preorders (varint deltas from 0), appended to `nodes`.
fn decode(bytes: &[u8], nodes: &mut Vec<u32>) -> rusqlite::Result<()> {
    let invalid = || rusqlite::Error::UserFunctionError("invalid posting".into());
    let (mut at, mut pre) = (0, 0_u32);
    while at < bytes.len() {
        let (mut delta, mut shift) = (0_u32, 0);
        loop {
            let byte = *bytes.get(at).ok_or_else(invalid)?;
            at += 1;
            if shift == 28 && byte > 15 {
                return Err(invalid());
            }
            delta |= u32::from(byte & 127) << shift;
            if byte & 128 == 0 {
                break;
            }
            shift += 7;
        }
        pre = pre.checked_add(delta).ok_or_else(invalid)?;
        nodes.push(pre);
    }
    Ok(())
}
