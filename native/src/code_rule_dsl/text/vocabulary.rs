//! The vocabulary engine: many plans over many distinct strings at once, each string read once per
//! family of plans, never per node.
//!
//! - Exact sets: one hash map from string to the plans that hold it.
//! - Prefix, suffix and substring sets: one multi-literal searcher per anchor over every plan's
//!   literals (suffixes over the reversed string), each hit naming its plan.
//! - Automata: small groups (`GROUP`) as one multi-pattern regex, each string asked which match.
//!   One automaton for every pattern blows up (regex.md §1.1: 639 patterns, 676 s to build).
//! - Backtracking patterns: one at a time, as `Compiled`.
//!
//! The store side (`fill`) keeps each key's matching terms, filled once over the vocabulary and then
//! only over terms new since (`text_pending`).

use super::plan::{Anchor, Compiled, Plan, literal_searcher};
use super::{Key, Lowered};
use crate::content_store::{ContentStore, StoreResult};
use std::collections::HashMap;
use std::sync::Arc;

/// Automata searched together.
const GROUP: usize = 16;

/// For each of `plans`, the indexes of the `terms` it matches, ascending. A term a plan cannot
/// decide (a backtracking pattern past its limits) is left out; `undecided` says which plans had one.
pub(in crate::code_rule_dsl) fn evaluate(plans: &[&Plan], terms: &[&[u8]]) -> (Vec<Vec<u32>>, Vec<bool>) {
    use rayon::prelude::*;
    let mut out = vec![Vec::new(); plans.len()];
    let mut undecided = vec![false; plans.len()];
    // Exact sets.
    let mut whole = HashMap::<&[u8], Vec<usize>>::new();
    // Literal sets by anchor: the literals, and each literal's plan.
    let mut affixes: [(Vec<Vec<u8>>, Vec<usize>); 3] = Default::default();
    let mut automata = Vec::new();
    for (at, plan) in plans.iter().enumerate() {
        match plan {
            Plan::Always => out[at] = (0..terms.len() as u32).collect(),
            Plan::Never => {}
            Plan::Literals { anchor: Anchor::Whole, literals } => literals.iter().for_each(|literal| whole.entry(literal.as_slice()).or_default().push(at)),
            Plan::Literals { anchor, literals } => {
                let slot = &mut affixes[match anchor {
                    Anchor::Start => 0,
                    Anchor::End => 1,
                    _ => 2,
                }];
                for literal in literals {
                    slot.0.push(if *anchor == Anchor::End { literal.iter().rev().copied().collect() } else { literal.clone() });
                    slot.1.push(at);
                }
            }
            Plan::Automaton { anchor, core } => automata.push((at, Plan::automaton_hir(*anchor, core))),
            Plan::Backtracking { .. } => {
                let compiled = Compiled::of(plan);
                let found = terms.par_iter().map(|term| compiled.is_match(term)).collect::<Vec<_>>();
                undecided[at] = found.iter().any(Option::is_none);
                out[at] = found.iter().enumerate().filter(|(_, m)| **m == Some(true)).map(|(i, _)| i as u32).collect();
            }
        }
    }
    if !whole.is_empty() {
        for (i, term) in terms.iter().enumerate() {
            if let Some(plans) = whole.get(term) {
                plans.iter().for_each(|&at| out[at].push(i as u32));
            }
        }
    }
    for (slot, (literals, owners)) in affixes.iter().enumerate() {
        if literals.is_empty() {
            continue;
        }
        let Some(searcher) = literal_searcher(literals) else {
            owners.iter().for_each(|&at| undecided[at] = true);
            continue;
        };
        // A prefix (suffix) literal lies within the first (last) `longest` bytes: only those are
        // searched, and only hits at the edge count (overlapping search is not anchored).
        let longest = literals.iter().map(Vec::len).max().unwrap_or(0);
        let hits = terms
            .par_iter()
            .map(|term| {
                let edge;
                let haystack: &[u8] = match slot {
                    0 => &term[..term.len().min(longest)],
                    1 => {
                        edge = term[term.len().saturating_sub(longest)..].iter().rev().copied().collect::<Vec<_>>();
                        &edge
                    }
                    _ => term,
                };
                let mut plans = searcher
                    .try_find_overlapping_iter(haystack)
                    .expect("an overlapping search of a standard searcher")
                    .filter(|hit| slot == 2 || hit.start() == 0)
                    .map(|hit| owners[hit.pattern().as_usize()])
                    .collect::<Vec<_>>();
                plans.sort_unstable();
                plans.dedup();
                plans
            })
            .collect::<Vec<_>>();
        for (i, plans) in hits.into_iter().enumerate() {
            plans.into_iter().for_each(|at| out[at].push(i as u32));
        }
    }
    for group in automata.chunks(GROUP) {
        let hirs = group.iter().map(|(_, hir)| hir).collect::<Vec<_>>();
        // Every pattern that matches, not the leftmost-first winner: `MatchKind::All`.
        let Ok(regex) = regex_automata::meta::Builder::new()
            .configure(regex_automata::meta::Config::new().match_kind(regex_automata::MatchKind::All))
            .build_many_from_hir(&hirs)
        else {
            // One at a time where the group does not build together.
            for (at, _) in group {
                let compiled = Compiled::of(plans[*at]);
                let found = terms.par_iter().map(|term| compiled.is_match(term)).collect::<Vec<_>>();
                undecided[*at] |= found.iter().any(Option::is_none);
                out[*at] = found.iter().enumerate().filter(|(_, m)| **m == Some(true)).map(|(i, _)| i as u32).collect();
            }
            continue;
        };
        let hits = terms
            .par_iter()
            .map_init(
                || regex_automata::PatternSet::new(regex.pattern_len()),
                |set, term| {
                    set.clear();
                    regex.which_overlapping_matches(&regex_automata::Input::new(*term), set);
                    set.iter().map(|pattern| pattern.as_usize()).collect::<Vec<_>>()
                },
            )
            .collect::<Vec<_>>();
        for (i, patterns) in hits.into_iter().enumerate() {
            patterns.into_iter().for_each(|pattern| out[group[pattern].0].push(i as u32));
        }
    }
    for found in &mut out {
        found.sort_unstable();
        found.dedup();
    }
    (out, undecided)
}

/// Each of `lowered`'s matching terms of language `lang` (atoms), kept in the store by key: a key
/// seen for the first time is evaluated over the whole vocabulary, and a known key over the terms
/// added since it last read the log (`text_pending`). Exact sets are looked up by value, never
/// evaluated or kept.
pub(in crate::code_rule_dsl) fn fill(store: &ContentStore, lang: u32, lowered: &[Arc<Lowered>]) -> StoreResult<HashMap<Key, Vec<i64>>> {
    let lang = i64::from(lang);
    store.candidate_transaction(true, |db| {
        let _stage = crate::code_rule_dsl::prof::Stage::start("text.vocabulary");
        let mut out = HashMap::new();
        let mut kept: Vec<&Arc<Lowered>> = Vec::new();
        for lowered in lowered {
            if out.contains_key(&lowered.key) || kept.iter().any(|known| known.key == lowered.key) {
                continue;
            }
            match &lowered.plan {
                // A literal is text: found by `atoms_by_value`, whatever terms came since.
                Plan::Literals { anchor: Anchor::Whole, literals } => {
                    let mut lookup = db.prepare_cached("SELECT v.term FROM atoms a CROSS JOIN term_freq v ON v.lang=?1 AND v.term=a.atom WHERE a.value=?2")?;
                    let mut terms = Vec::new();
                    for literal in literals {
                        let Ok(text) = std::str::from_utf8(literal) else { continue };
                        terms.extend(lookup.query_map(rusqlite::params![lang, text], |row| row.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?);
                    }
                    terms.sort_unstable();
                    out.insert(lowered.key.clone(), terms);
                }
                Plan::Never => {
                    out.insert(lowered.key.clone(), Vec::new());
                }
                _ => kept.push(lowered),
            }
        }
        if kept.is_empty() {
            return Ok(out);
        }
        let head: i64 = db.query_row("SELECT coalesce(max(seq), 0) FROM text_pending", [], |row| row.get(0))?;
        // Each kept key's place in the log: `None` for a key not seen before.
        let upto = {
            let mut statement = db.prepare_cached("SELECT upto FROM text_matcher WHERE lang=?1 AND key=?2")?;
            kept.iter()
                .map(|lowered| statement.query_row(rusqlite::params![lang, lowered.key.0], |row| row.get::<_, i64>(0)).map(Some).or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    e => Err(e),
                }))
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let read = |sql: &str, from: i64| -> rusqlite::Result<(Vec<i64>, Vec<String>)> {
            let mut statement = db.prepare_cached(sql)?;
            let rows = statement.query_map(rusqlite::params![lang, from], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows.into_iter().unzip())
        };
        let mut insert = db.prepare_cached("INSERT OR IGNORE INTO text_term(lang,key,term) VALUES (?1,?2,?3)")?;
        let mut register = db.prepare_cached("INSERT INTO text_matcher(lang,key,plan,upto) VALUES (?1,?2,?3,?4) ON CONFLICT(lang,key) DO UPDATE SET upto=excluded.upto")?;
        // Keys group by where they start reading: a new key reads every term; a known key the log
        // past its place (terms still in the vocabulary).
        let mut starts = upto.iter().copied().collect::<Vec<_>>();
        starts.sort_unstable();
        starts.dedup();
        for from in starts {
            let group = kept.iter().zip(&upto).filter(|(_, at)| **at == from).map(|(lowered, _)| *lowered).collect::<Vec<_>>();
            let (terms, texts) = match from {
                None => read("SELECT v.term, a.value FROM term_freq v CROSS JOIN atoms a ON a.atom=v.term WHERE v.lang=?1 AND ?2=?2", 0)?,
                Some(at) if at >= head => Default::default(),
                Some(at) => read(
                    "SELECT DISTINCT p.term, a.value FROM text_pending p CROSS JOIN term_freq v ON v.lang=p.lang AND v.term=p.term CROSS JOIN atoms a ON a.atom=p.term WHERE p.lang=?1 AND p.seq>?2",
                    at,
                )?,
            };
            if !terms.is_empty() {
                let plans = group.iter().map(|lowered| &lowered.plan).collect::<Vec<_>>();
                let bytes = texts.iter().map(|text| text.as_bytes()).collect::<Vec<_>>();
                let (found, _) = evaluate(&plans, &bytes);
                crate::code_rule_dsl::prof::count("text.vocabulary.tests", terms.len() * group.len());
                for (lowered, found) in group.iter().zip(found) {
                    if from.is_none() {
                        register.execute(rusqlite::params![lang, lowered.key.0, lowered.plan.canonical(), head])?;
                    }
                    for i in found {
                        insert.execute(rusqlite::params![lang, lowered.key.0, terms[i as usize]])?;
                    }
                }
            }
            for lowered in &group {
                register.execute(rusqlite::params![lang, lowered.key.0, lowered.plan.canonical(), head])?;
            }
        }
        // Log entries every key has read.
        db.execute("DELETE FROM text_pending WHERE seq <= (SELECT coalesce(min(upto), 0) FROM text_matcher)", [])?;
        let mut terms_of = db.prepare_cached("SELECT term FROM text_term WHERE lang=?1 AND key=?2 ORDER BY term")?;
        for lowered in kept {
            let terms = terms_of.query_map(rusqlite::params![lang, lowered.key.0], |row| row.get::<_, i64>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            out.insert(lowered.key.clone(), terms);
        }
        Ok(out)
    })
}
