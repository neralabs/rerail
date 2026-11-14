//! The vocabulary engine: many plans over many distinct strings at once, each string read once per
//! family of plans, never per node.
//!
//! - Exact sets: one hash map from string to the plans that hold it.
//! - Prefix, suffix and substring sets: one multi-literal searcher per anchor over every plan's
//!   literals (suffixes over the reversed string), each hit naming its plan.
//! - Automata: small groups (`GROUP`) as one multi-pattern DFA, each string asked which match.
//!   One automaton for every pattern blows up (regex.md §1.1: 639 patterns, 676 s to build). A group
//!   is determinized once, within `DFA_LIMIT`, and kept between processes (`DfaCache`, decision
//!   0075); a group that does not fit runs on the lazy DFA.
//!
//! `Searchers` is the compiled form: made once for a set of plans, run over every batch.
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

/// The largest dense DFA a group of automata is determinized into, in bytes; a group whose DFA would be larger runs on
/// the lazy one (`meta`) instead, and is never stored.
const DFA_LIMIT: usize = 4 << 20;

/// Compiled groups of automata, kept between processes (decision 0075): `get` gives a group's DFA as `to_bytes` made
/// it, `Some(None)` for a group known not to determinize within `DFA_LIMIT`, and `None` for one never compiled.
pub trait DfaCache {
    fn get(&self, key: &str) -> Option<Option<Vec<u8>>>;
    fn put(&self, key: &str, dfa: Option<&[u8]>);
}

/// What compiling the searchers did, for the run's report: groups read from the cache, determinized, and run lazily.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compiling {
    pub read: usize,
    pub built: usize,
    pub lazy: usize,
}

type Dfa = regex_automata::dfa::dense::DFA<Vec<u32>>;

/// A group of automata searched together for every pattern that matches (`MatchKind::All`).
#[allow(clippy::large_enum_variant)]
enum Engine {
    /// Determinized, from the cache or built now. A string the DFA gives up on (a quit byte) is asked of `lazy`.
    Dfa(std::sync::Arc<Dfa>),
    /// The lazy DFA, for a group that does not determinize within `DFA_LIMIT`.
    Lazy,
    /// One at a time: the group does not build together.
    Each(Vec<Compiled>),
}

struct Group {
    /// Each pattern's plan, by its place in the group.
    owners: Vec<usize>,
    hirs: Vec<regex_syntax::hir::Hir>,
    engine: Engine,
    lazy: std::sync::OnceLock<Option<regex_automata::meta::Regex>>,
}

impl Group {
    fn lazy(&self) -> Option<&regex_automata::meta::Regex> {
        self.lazy
            .get_or_init(|| {
                regex_automata::meta::Builder::new()
                    .configure(regex_automata::meta::Config::new().match_kind(regex_automata::MatchKind::All))
                    .build_many_from_hir(&self.hirs)
                    .ok()
            })
            .as_ref()
    }
}

/// The key a group's DFA is kept under: its plans' canonical forms, the engine that searches it and its version.
fn group_key(hirs: &[regex_syntax::hir::Hir]) -> String {
    use sha2::Digest as _;
    let mut hash = sha2::Sha256::new();
    hash.update(format!("rerail.dfa/v1\0regex-automata 0.4.18\0{}\0{}\0", DFA_LIMIT, if cfg!(target_endian = "little") { "le" } else { "be" }).as_bytes());
    for hir in hirs {
        hash.update(hir.to_string().as_bytes());
        hash.update(b"\0");
    }
    format!("dfa1:{}", crate::code_rule_dsl::prof::lower_hex(&hash.finalize()))
}

/// `hirs` determinized into one DFA over all of them, if it fits in `DFA_LIMIT`.
fn determinize(hirs: &[regex_syntax::hir::Hir]) -> Option<Dfa> {
    use regex_automata::dfa::dense;
    let nfa = regex_automata::nfa::thompson::Compiler::new().build_many_from_hir(hirs).ok()?;
    dense::Builder::new()
        .configure(
            dense::Config::new()
                .match_kind(regex_automata::MatchKind::All)
                .start_kind(regex_automata::dfa::StartKind::Unanchored)
                .dfa_size_limit(Some(DFA_LIMIT))
                .determinize_size_limit(Some(DFA_LIMIT)),
        )
        .build_from_nfa(&nfa)
        .ok()
}

/// The groups this process determinized, by `group_key`: their DFA, or `None` for one that runs lazily. A few
/// hundred at a time (a data-layer policy makes a handful), cleared when full.
const KEPT_GROUPS: usize = 512;

fn kept_groups() -> &'static std::sync::Mutex<HashMap<String, Option<std::sync::Arc<Dfa>>>> {
    static KEPT: std::sync::OnceLock<std::sync::Mutex<HashMap<String, Option<std::sync::Arc<Dfa>>>>> = std::sync::OnceLock::new();
    KEPT.get_or_init(Default::default)
}

/// A DFA read back from the bytes `to_bytes_native_endian` made; `None` when they no longer read.
fn read_dfa(bytes: &[u8]) -> Option<Dfa> {
    // `from_bytes` wants the words aligned: copied into a word buffer first.
    let mut words = vec![0u32; bytes.len().div_ceil(4)];
    let view: &mut [u8] = bytemuck_bytes(&mut words);
    view[..bytes.len()].copy_from_slice(bytes);
    let (dfa, _) = regex_automata::dfa::dense::DFA::from_bytes(&view[..bytes.len()]).ok()?;
    Some(dfa.to_owned())
}

/// `words` as bytes.
fn bytemuck_bytes(words: &mut [u32]) -> &mut [u8] {
    // SAFETY: a u32 slice is a valid u8 slice four times as long, of the same memory and alignment at least 1.
    unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), words.len() * 4) }
}

/// `plans` compiled for `evaluate`: made once for a set of plans and run over every batch of strings after it. The
/// literal searchers are made again in each process (building one is linear in its literals); a group of automata is
/// read from `cache` when an earlier process determinized it.
pub(in crate::code_rule_dsl) struct Searchers {
    plans: usize,
    always: Vec<usize>,
    /// Exact sets.
    whole: HashMap<Vec<u8>, Vec<usize>>,
    /// Literal sets by anchor (start, end, within): the searcher, the longest literal, each literal's plan.
    affixes: Vec<(usize, Option<aho_corasick::AhoCorasick>, usize, Vec<usize>)>,
    groups: Vec<Group>,
    backtracking: Vec<(usize, Compiled)>,
    pub(in crate::code_rule_dsl) compiling: Compiling,
}

impl Searchers {
    /// `dense`: a group this process has not determinized and `cache` does not hold is determinized now (and kept);
    /// otherwise it runs on the lazy DFA, which builds the states its strings reach, and is kept nowhere (a later run
    /// that wants it dense builds it). A dense build costs more than a small run's search saves
    /// (O-vocabulary-dfa-reuse).
    pub(in crate::code_rule_dsl) fn new(plans: &[&Plan], cache: Option<&dyn DfaCache>, dense: bool) -> Self {
        let mut out = Searchers { plans: plans.len(), always: Vec::new(), whole: HashMap::new(), affixes: Vec::new(), groups: Vec::new(), backtracking: Vec::new(), compiling: Compiling::default() };
        let mut affixes: [(Vec<Vec<u8>>, Vec<usize>); 3] = Default::default();
        let mut automata = Vec::new();
        for (at, plan) in plans.iter().enumerate() {
            match plan {
                Plan::Always => out.always.push(at),
                Plan::Never => {}
                Plan::Literals { anchor: Anchor::Whole, literals } => literals.iter().for_each(|literal| out.whole.entry(literal.clone()).or_default().push(at)),
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
                Plan::Backtracking { .. } => out.backtracking.push((at, Compiled::of(plan))),
            }
        }
        for (slot, (literals, owners)) in affixes.into_iter().enumerate() {
            if !literals.is_empty() {
                let longest = literals.iter().map(Vec::len).max().unwrap_or(0);
                out.affixes.push((slot, literal_searcher(&literals), longest, owners));
            }
        }
        for group in automata.chunks(GROUP) {
            let owners: Vec<usize> = group.iter().map(|(at, _)| *at).collect();
            let hirs: Vec<regex_syntax::hir::Hir> = group.iter().map(|(_, hir)| hir.clone()).collect();
            // A group already determinized in this process (another policy of a batch, another run) is taken as it
            // is: by its key, which names its patterns (O-vocabulary-dfa-reuse). The store's cache is asked after.
            let named = group_key(&hirs);
            if let Some(kept) = kept_groups().lock().expect("the kept groups").get(&named).cloned() {
                crate::code_rule_dsl::prof::count("text.dfa_group_kept", 1);
                let engine = match kept {
                    Some(dfa) => {
                        out.compiling.read += 1;
                        Engine::Dfa(dfa)
                    }
                    None => {
                        out.compiling.lazy += 1;
                        Engine::Lazy
                    }
                };
                out.groups.push(Self::grouped(Group { owners, hirs, engine, lazy: std::sync::OnceLock::new() }, plans));
                continue;
            }
            let key = cache.map(|_| named.clone());
            let cached = cache.zip(key.as_deref()).and_then(|(c, k)| c.get(k));
            let engine = match cached {
                Some(Some(bytes)) if let Some(dfa) = read_dfa(&bytes) => {
                    out.compiling.read += 1;
                    Engine::Dfa(std::sync::Arc::new(dfa))
                }
                Some(None) => {
                    out.compiling.lazy += 1;
                    Engine::Lazy
                }
                _ if !dense => {
                    out.compiling.lazy += 1;
                    let group = Group { owners, hirs, engine: Engine::Lazy, lazy: std::sync::OnceLock::new() };
                    out.groups.push(Self::grouped(group, plans));
                    continue;
                }
                _ => match determinize(&hirs) {
                    Some(dfa) => {
                        out.compiling.built += 1;
                        if let (Some(c), Some(k)) = (cache, key.as_deref()) {
                            // The bytes start after `pad` bytes that align them.
                            let (bytes, pad) = dfa.to_bytes_native_endian();
                            c.put(k, Some(&bytes[pad..]));
                        }
                        Engine::Dfa(std::sync::Arc::new(dfa))
                    }
                    None => {
                        out.compiling.lazy += 1;
                        if let (Some(c), Some(k)) = (cache, key.as_deref()) {
                            c.put(k, None);
                        }
                        Engine::Lazy
                    }
                },
            };
            let kept = match &engine {
                Engine::Dfa(dfa) => Some(Some(std::sync::Arc::clone(dfa))),
                Engine::Lazy => Some(None),
                Engine::Each(_) => None,
            };
            if let Some(kept) = kept {
                let mut all = kept_groups().lock().expect("the kept groups");
                if all.len() >= KEPT_GROUPS {
                    all.clear();
                }
                all.insert(named, kept);
            }
            out.groups.push(Self::grouped(Group { owners, hirs, engine, lazy: std::sync::OnceLock::new() }, plans));
        }
        out
    }

    /// A group that runs lazily but does not build together runs one at a time.
    fn grouped(group: Group, plans: &[&Plan]) -> Group {
        if matches!(group.engine, Engine::Lazy) && group.lazy().is_none() {
            let each = group.owners.iter().map(|at| Compiled::of(plans[*at])).collect();
            Group { engine: Engine::Each(each), ..group }
        } else {
            group
        }
    }

    /// For each plan, the indexes of the `terms` it matches, ascending. A term a plan cannot decide (a backtracking
    /// pattern past its limits) is left out; `undecided` says which plans had one.
    pub(in crate::code_rule_dsl) fn evaluate(&self, terms: &[&[u8]]) -> (Vec<Vec<u32>>, Vec<bool>) {
        use rayon::prelude::*;
        let mut out = vec![Vec::new(); self.plans];
        let mut undecided = vec![false; self.plans];
        for &at in &self.always {
            out[at] = (0..terms.len() as u32).collect();
        }
        for (at, compiled) in &self.backtracking {
            let found = terms.par_iter().map(|term| compiled.is_match(term)).collect::<Vec<_>>();
            undecided[*at] = found.iter().any(Option::is_none);
            out[*at] = found.iter().enumerate().filter(|(_, m)| **m == Some(true)).map(|(i, _)| i as u32).collect();
        }
        if !self.whole.is_empty() {
            for (i, term) in terms.iter().enumerate() {
                if let Some(plans) = self.whole.get(*term) {
                    plans.iter().for_each(|&at| out[at].push(i as u32));
                }
            }
        }
        for (slot, searcher, longest, owners) in &self.affixes {
            let Some(searcher) = searcher else {
                owners.iter().for_each(|&at| undecided[at] = true);
                continue;
            };
            let (slot, longest) = (*slot, *longest);
            // A prefix (suffix) literal lies within the first (last) `longest` bytes: only those are searched, and only
            // hits at the edge count (overlapping search is not anchored).
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
        for group in &self.groups {
            if let Engine::Each(each) = &group.engine {
                for (at, compiled) in group.owners.iter().zip(each) {
                    let found = terms.par_iter().map(|term| compiled.is_match(term)).collect::<Vec<_>>();
                    undecided[*at] |= found.iter().any(Option::is_none);
                    out[*at] = found.iter().enumerate().filter(|(_, m)| **m == Some(true)).map(|(i, _)| i as u32).collect();
                }
                continue;
            }
            let hits = terms
                .par_iter()
                .map_init(
                    || regex_automata::PatternSet::new(group.owners.len()),
                    |set, term| {
                        set.clear();
                        let input = regex_automata::Input::new(*term);
                        let asked_lazily = match &group.engine {
                            Engine::Dfa(dfa) => regex_automata::dfa::Automaton::try_which_overlapping_matches(&**dfa, &input, set).is_err(),
                            _ => true,
                        };
                        if asked_lazily {
                            set.clear();
                            if let Some(regex) = group.lazy() {
                                regex.which_overlapping_matches(&input, set);
                            }
                        }
                        set.iter().map(|pattern| pattern.as_usize()).collect::<Vec<_>>()
                    },
                )
                .collect::<Vec<_>>();
            for (i, patterns) in hits.into_iter().enumerate() {
                patterns.into_iter().for_each(|pattern| out[group.owners[pattern]].push(i as u32));
            }
        }
        for found in &mut out {
            found.sort_unstable();
            found.dedup();
        }
        (out, undecided)
    }
}

/// For each of `plans`, the indexes of the `terms` it matches, ascending. A term a plan cannot
/// decide (a backtracking pattern past its limits) is left out; `undecided` says which plans had one.
pub(in crate::code_rule_dsl) fn evaluate(plans: &[&Plan], terms: &[&[u8]]) -> (Vec<Vec<u32>>, Vec<bool>) {
    Searchers::new(plans, None, true).evaluate(terms)
}

/// Each of `lowered`'s matching terms of language `lang` (atoms), kept in the store by key: a key
/// seen for the first time is evaluated over the whole vocabulary, and a known key over the terms
/// added since it last read the log (`text_pending`). Exact sets are looked up by value, never
/// evaluated or kept.
pub fn fill(store: &ContentStore, lang: u32, lowered: &[Arc<Lowered>]) -> StoreResult<HashMap<Key, Vec<i64>>> {
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
        let mut starts = upto.to_vec();
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
