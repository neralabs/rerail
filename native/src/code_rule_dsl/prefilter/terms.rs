//! `R[tid]` (03 §16 item 4, list step 17): a walk's text test, as a node set read from the term
//! index instead of each node's text. A test's predicate runs over the language's vocabulary once
//! (`vocabulary_term`, filled by the prefilter's vocabulary stages, incrementally over new terms),
//! and its nodes on a tape are the union of the matched terms' postings plus the nodes the terms
//! do not cover (`term_coverage.missing`), confirmed on their text (decision 0026: no term id is
//! stored per node). Literal tests are a join on the term's atom.
//!
//! The rows are attached to the wave's tapes (`TapeDoc::with_texts`), where every walk's text
//! slots read them (`Micro::slot`) by the slot's key (`Micro::slot_key`); a slot the index cannot
//! answer (a Lua pattern) is computed on the text as before.

use super::lower::VOCABULARY;
use crate::code_rule_dsl::tape::TapeDoc;
use crate::content_store::{ContentStore, StoreResult};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// A text test the term index answers.
pub(in crate::code_rule_dsl) struct TextSlot<'a> {
    /// The slot's key (`Micro::slot_key`): equal tests share their rows.
    pub(in crate::code_rule_dsl) key: String,
    /// The vocabulary's predicate: a literal term, or a regex.
    pub(in crate::code_rule_dsl) expression: String,
    pub(in crate::code_rule_dsl) literal: bool,
    /// The kind the test's node has, where the walk tests it first.
    pub(in crate::code_rule_dsl) kind: Option<u16>,
    /// The test on one node's text (an uncovered node).
    pub(in crate::code_rule_dsl) confirm: Box<dyn Fn(&TapeDoc, u32) -> Option<bool> + Sync + 'a>,
}

/// Per tape (in `tapes`' order), each slot's bits by key; a slot that failed on a tape (its
/// confirmation could not run) is left out there, and the walk computes it on the text.
pub(in crate::code_rule_dsl) fn rows(
    store: &ContentStore,
    lang: u32,
    tapes: &[(i64, &TapeDoc)],
    slots: &[TextSlot<'_>],
) -> StoreResult<Vec<BTreeMap<String, Arc<Vec<u64>>>>> {
    let mut out = vec![BTreeMap::new(); tapes.len()];
    if slots.is_empty() || tapes.is_empty() {
        return Ok(out);
    }
    let lang = i64::from(lang);
    let index = tapes.iter().enumerate().map(|(at, (id, _))| (*id, at)).collect::<HashMap<_, _>>();
    // Per slot, per tape: the matched terms' nodes; and per tape the uncovered nodes.
    let (found, missing) = store.candidate_transaction(true, |db| {
        let regexes = slots.iter().filter(|slot| !slot.literal).map(|slot| slot.expression.as_str()).collect::<std::collections::BTreeSet<_>>();
        if !regexes.is_empty() {
            let _stage = crate::code_rule_dsl::prof::Stage::start("terms.vocabulary");
            let mut register = db.prepare_cached("INSERT OR IGNORE INTO vocabulary_predicate(lang,expression,ready) VALUES (?1,?2,0)")?;
            for expression in &regexes {
                register.execute(rusqlite::params![lang, expression])?;
            }
            for (_, sql) in VOCABULARY {
                db.execute_batch(sql)?;
            }
        }
        let _stage = crate::code_rule_dsl::prof::Stage::start("terms.postings");
        let mut literal = db.prepare_cached("SELECT v.term FROM atoms a CROSS JOIN term_freq v ON v.lang=?1 AND v.term=a.atom WHERE a.value=?2")?;
        let mut regex = db.prepare_cached("SELECT term FROM vocabulary_term WHERE lang=?1 AND expression=?2")?;
        let mut postings = db.prepare_cached("SELECT tape_id, postings FROM term_postings WHERE lang=?1 AND term=?2")?;
        let mut terms_of = HashMap::<(bool, &str), Vec<Vec<u32>>>::new();
        let mut found = Vec::with_capacity(slots.len());
        for slot in slots {
            let key = (slot.literal, slot.expression.as_str());
            if !terms_of.contains_key(&key) {
                let statement = if slot.literal { &mut literal } else { &mut regex };
                let terms = statement
                    .query_map(rusqlite::params![lang, slot.expression], |row| row.get::<_, i64>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut nodes = vec![Vec::new(); tapes.len()];
                for term in terms {
                    let mut rows = postings.query(rusqlite::params![lang, term])?;
                    while let Some(row) = rows.next()? {
                        if let Some(&at) = index.get(&row.get::<_, i64>(0)?) {
                            decode(row.get_ref(1)?.as_blob()?, &mut nodes[at])?;
                        }
                    }
                }
                terms_of.insert(key, nodes);
            }
            found.push(terms_of[&key].clone());
        }
        let mut uncovered = db.prepare_cached("SELECT missing FROM term_coverage WHERE tape_id=?1 AND length(missing)>0")?;
        let mut missing = vec![Vec::new(); tapes.len()];
        for (at, (id, _)) in tapes.iter().enumerate() {
            let mut rows = uncovered.query([id])?;
            while let Some(row) = rows.next()? {
                decode(row.get_ref(0)?.as_blob()?, &mut missing[at])?;
            }
        }
        Ok((found, missing))
    })?;
    let _stage = crate::code_rule_dsl::prof::Stage::start("terms.rows");
    use rayon::prelude::*;
    out.par_iter_mut().enumerate().for_each(|(at, rows)| {
        let tape = tapes[at].1;
        let len = tape.transducer_len();
        for (slot, found) in slots.iter().zip(&found) {
            // A test of any kind would be confirmed on every uncovered node, whole classes and
            // functions among them; the walk tests the few it reaches on their text instead.
            if slot.kind.is_none() && !missing[at].is_empty() && std::env::var_os("RERAIL_TEXTS_ANY").is_none() {
                continue;
            }
            let mut bits = vec![0_u64; len.div_ceil(64)];
            let of_kind = |node: u32| slot.kind.is_none_or(|kind| tape.transducer_kind(node) == kind);
            for &node in &found[at] {
                if (node as usize) < len && of_kind(node) {
                    bits[node as usize / 64] |= 1 << (node % 64);
                }
            }
            let mut confirmed = true;
            for &node in &missing[at] {
                if (node as usize) >= len || !of_kind(node) {
                    continue;
                }
                match (slot.confirm)(tape, node) {
                    Some(true) => bits[node as usize / 64] |= 1 << (node % 64),
                    Some(false) => {}
                    None => {
                        confirmed = false;
                        break;
                    }
                }
            }
            if confirmed {
                rows.insert(slot.key.clone(), Arc::new(bits));
            }
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
