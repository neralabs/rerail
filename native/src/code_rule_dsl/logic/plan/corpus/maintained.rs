//! One maintained run of a program over its inputs (decisions 0077, 0090): the strata whose reads changed run, from
//! their deltas where they can (`maintain::rewrite`), the others' rows are bound from what was kept; what the run
//! derived is kept for the next (`retain::after`).
//!
//! The cross-file part's session path does the same with more beside it (statistics kept by delta, frames, the
//! measurement aids); this is its core, for a program run many times over growing inputs: the cone's rounds
//! (decision 0145).

use super::{Result, held_rows, lower, maintain, retain};
use crate::code_rule_dsl::facts::vtab::{Modules, Source, syntax};
use crate::code_rule_dsl::logic::native::component::{self, InRel};
use rusqlite::Connection;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// What a maintained run did.
pub(in crate::code_rule_dsl) struct Ran {
    /// What it keeps for the next run.
    pub(in crate::code_rule_dsl) next: retain::Retained,
    /// Strata run, of all; maintained from deltas; derived again.
    pub(in crate::code_rule_dsl) strata: (usize, usize),
    pub(in crate::code_rule_dsl) maintained: usize,
    pub(in crate::code_rule_dsl) derived_again: usize,
    /// The inputs whose rows changed.
    pub(in crate::code_rule_dsl) changed: Vec<String>,
    /// Each stratum derived again, by one of its relations, and why (`maintain::Maintained::again`).
    pub(in crate::code_rule_dsl) again: Vec<(String, &'static str)>,
    /// The outputs of the strata that ran, as the run decoded them: an output of a stratum cut away is not among them.
    pub(in crate::code_rule_dsl) outputs: BTreeMap<String, Arc<crate::code_rule_dsl::logic::plan::output::Rows>>,
    /// The tables it bound beside the caller's inputs (kept relations, old rows, deltas): the caller clears them
    /// when its rounds end.
    pub(in crate::code_rule_dsl) bound: Vec<String>,
}

/// An input's delta given by the caller, who knows how its rows changed (the guests' rounds, decision 0331): from its
/// old rows and its new, the rows gone and the rows new. `None` leaves it to `retain::document_delta`.
pub(in crate::code_rule_dsl) type Delta<'d> = &'d dyn Fn(&str, &dyn Source, &dyn Source) -> Option<(Arc<dyn Source>, Arc<dyn Source>)>;

/// Runs `prog` over `inputs` on `db`, maintaining what `kept` holds of it (of the same `key`); `keep` the relations
/// the caller reads after (held, not inlined). Each input is given with its digest, and whether it is bound already
/// (as it is: a run before this one on `db` and `modules` bound it, and `stats` holds its statistics).
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn run(
    store: Option<&crate::content_store::ContentStore>,
    db: &Connection,
    modules: &Modules,
    prog: &lower::Lowered,
    key: [u8; 32],
    inputs: &[(String, Arc<dyn Source>, u64, bool)],
    kept: Option<retain::Retained>,
    stats: &mut crate::code_rule_dsl::logic::plan::Stats,
    keep: &BTreeSet<String>,
    modes: &str,
    texts: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
) -> Result<Ran> {
    run_with(store, db, modules, prog, key, inputs, kept, stats, keep, modes, texts, None)
}

/// `run`, each changed input's delta from `delta` where it gives one.
#[allow(clippy::too_many_arguments)]
pub(in crate::code_rule_dsl) fn run_with(
    store: Option<&crate::content_store::ContentStore>,
    db: &Connection,
    modules: &Modules,
    prog: &lower::Lowered,
    key: [u8; 32],
    inputs: &[(String, Arc<dyn Source>, u64, bool)],
    mut kept: Option<retain::Retained>,
    stats: &mut crate::code_rule_dsl::logic::plan::Stats,
    keep: &BTreeSet<String>,
    modes: &str,
    texts: &Arc<crate::code_rule_dsl::logic::strings::Vocabulary>,
    delta: Option<Delta<'_>>,
) -> Result<Ran> {
    let span = |name: &'static str| crate::code_rule_dsl::logic::profile::span(move || format!("maintained/{name}"));
    // What a run keeps is each relation's whole rows, as a session's (decision 0077): one computed only for the keys
    // its readers reach would miss a key a later run demands, whose derivation no delta touches (spike O125). The
    // demand split's relations (`demand_*`) were of no stratum the run kept, so every later round derived them
    // and their readers again (decision 0331).
    let _whole = super::super::demand::whole_relations();
    let digests: HashMap<String, u64> = inputs.iter().map(|(rel, _, d, _)| (rel.clone(), *d)).collect();
    let sources: Vec<(String, Arc<dyn Source>)> = inputs.iter().map(|(r, s, ..)| (r.clone(), Arc::clone(s))).collect();
    // A cone's program has no split: its inputs' rows are grouped by their first node column (`group_col`).
    let no_locations = retain::Locations::new();
    let s = span("cut and rewrite");
    // The run reads the program's version, not its tree (t36 item 7).
    let placed = {
        let _s = span("cut and rewrite: the program placed");
        prog.placed()?
    };
    let sccs = {
        let _s = span("cut and rewrite: its strata");
        placed.components()?
    };
    let cut = {
        let _s = span("cut and rewrite: the cut");
        retain::cut(kept.as_ref(), key, &placed, &sccs, &digests)?
    };
    // What the strata that run read of a part kept in the frames is read back now; the rest stays in the store.
    if let (Some(k), Some(store)) = (kept.as_mut(), store) {
        k.load(store, &cut, &placed, &no_locations, false)?;
    }
    let frames = kept.as_mut().and_then(retain::Retained::take_frames);
    let by_name: HashMap<&str, &Arc<dyn Source>> = inputs.iter().map(|(r, s, ..)| (r.as_str(), s)).collect();
    let mut input_docs: HashMap<String, Arc<retain::DocDigests>> = HashMap::new();
    // An input's delta, found from its old rows by document: hashed by content, as the frames hold its documents'.
    let atoms = store.map_or_else(|| crate::content_store::atoms::Atoms::for_connection(db), |s| s.atoms_handle());
    let mut given = |rel: &str| -> Option<(Arc<dyn Source>, Arc<dyn Source>)> {
        let _g = crate::code_rule_dsl::logic::profile::span(|| "maintained/an input's delta".into());
        let kept = kept.as_ref()?;
        let new = by_name.get(rel)?;
        let old = kept.input_source(rel)?;
        if let Some(given) = delta.and_then(|d| d(rel, old.as_ref(), new.as_ref())) {
            return Some(given);
        }
        let (delta, docs) = retain::document_delta(rel, old.as_ref(), kept.input_docs(rel).map(|d| d.as_ref()), new.as_ref(), retain::group_col(&placed, &no_locations, rel), &retain::cells_of(&atoms, &placed, &prog.domains, rel, new.columns().len()))?;
        input_docs.insert(rel.to_owned(), docs);
        Some(delta)
    };
    // Decision 0117 step 3, as a session's run: with a kept part, the run walks the cut's base program and makes a
    // stratum's maintained form when a changed read reaches it (`Driver`), so a stratum no delta reaches is neither
    // written nor set up. The guests' rounds rewrote the whole slice each round, and set up its ~1,000 strata, for the
    // few a round's guests reach (decision 0331). `RERAIL_MAINTAIN_WHOLE` rewrites the whole program first, as before.
    let gated = !placed.fields()?.gates().is_empty();
    let driven = kept.is_some() && !gated && std::env::var_os("RERAIL_MAINTAIN_WHOLE").is_none();
    let (mut maintained, seeds, given_rels) = {
        let _s = span("cut and rewrite: the rewrite");
        if driven {
            super::driven_part(&cut, prog, &mut given)?
        } else {
            (maintain::rewrite(&cut, kept.as_ref(), prog, &sccs, &mut given)?, Vec::new(), BTreeSet::new())
        }
    };
    let derived = {
        let _s = span("cut and rewrite: the derived relations");
        cut.derived(&sccs)?
    };
    let q = span("cut and rewrite: the qualifiers");
    // On the part's version (`qualify`): no tree decoded or copied.
    let levels = maintain::levels_wanted() && !driven;
    super::qualify(&mut maintained.program, |heads, qualifiers| {
        if levels {
            for rel in heads {
                qualifiers.entry(rel.clone()).or_default().insert("levels".to_owned());
            }
        }
        // What is kept for the next run is stored, not inlined into its readers (decision 0099).
        for rel in derived.iter().chain(keep) {
            let q = qualifiers.entry(rel.clone()).or_default();
            q.remove("inline");
            q.insert("no_inline".to_owned());
        }
    });
    drop(q);
    drop(s);
    let s = span("bind and measure");
    let part = &maintained.program;
    // The part's fields, read from its version: no reader below decodes its tree.
    let part_placed = part.placed()?;
    let part_f = part_placed.fields()?;
    // An input not bound yet: its statistics made when a stratum first plans with it (decision 0101 §4).
    for (rel, source, _, bound) in inputs {
        if !bound {
            let measured = Arc::clone(source);
            let lazy = crate::code_rule_dsl::logic::plan::LazyStats::new(move || {
                let _m = crate::code_rule_dsl::logic::profile::span(|| "maintained/measure".into());
                (syntax::statistics(measured.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])).collect(), Vec::new(), ())
            });
            stats.lazily(rel, source.len(), lazy);
            modules.bind(db, rel, Arc::clone(source))?;
        }
    }
    // The kept relations, old rows and deltas, bound as a session binds them (`bind_kept`): a kept relation's
    // statistics are kept with it while it is kept as it is, by delta where the run maintains it (decisions 0094 §4,
    // 0096 step 4, 0123), not measured again each run.
    let mut binder = super::Binder {
        constants: super::constants_tested_in(&part_placed.v).into_iter().map(|(r, c)| (r.to_owned(), c)).collect(),
        cell: kept.as_ref(),
        measured_kept: kept.as_ref().map(retain::Retained::measured_all).unwrap_or_default(),
        bound_old: HashMap::new(),
        lazy_kept: true,
        lazies: Vec::new(),
        copies: HashMap::new(),
    };
    let clean = kept.as_ref().map(|k| cut.sources(k, &|r| part_f.is_input(r))).unwrap_or_default();
    let mut bound: Vec<String> = Vec::new();
    for (rel, source, held) in clean.into_iter().chain(maintained.sources.iter().cloned()) {
        bound.push(rel.clone());
        let maintains = rel.strip_suffix("__old").is_some_and(|of| part_f.declared(&format!("{of}__del")));
        super::bind_kept(&mut binder, modules, db, stats, rel, source, held, maintains)?;
    }
    let mut wanted = keep.clone();
    wanted.extend(derived.iter().filter(|rel| !part_f.is_output(rel)).cloned());
    drop(s);
    let s = span("plan and evaluate");
    let got = match kept.as_ref().filter(|_| driven) {
        Some(k) => {
            let mut driver = super::Driver {
                cross: prog,
                cross_placed: &placed,
                kept: k,
                store: None,
                clean_reads: BTreeSet::new(),
                at_end: wanted.clone(),
                binder: &mut binder,
                modules,
                db,
                inputs: part_f.inputs().map(str::to_owned).collect(),
                has_delta: given_rels,
                bound_olds: BTreeSet::new(),
                no_delta: BTreeSet::new(),
                maintains: BTreeSet::new(),
                seeds,
                counts: (0, 0, 0),
                again: Vec::new(),
                walked: (0, 0, 0),
                atoms: None,
            };
            let hooks = super::super::Hooks { maintain: Some(&mut driver), texts: Some(Arc::clone(texts)), ..Default::default() };
            let got = super::super::run_atoms_native_hooked(part, db, "main", stats.clone(), wanted, modes, hooks)?;
            (maintained.maintained, maintained.derived_again, maintained.recursive) = driver.counts;
            maintained.again = std::mem::take(&mut driver.again);
            // The old rows the forms bound, cleared by the caller with the rest.
            bound.extend(driver.bound_olds.iter().map(|of| format!("{of}__old")));
            let (reached, quiet, of) = driver.walked;
            let mut why: BTreeMap<&str, usize> = BTreeMap::new();
            for (_, w) in &maintained.again {
                *why.entry(w).or_default() += 1;
            }
            crate::code_rule_dsl::logic::profile::decision("a maintained run driven by deltas (decision 0331)", || {
                format!("{reached} of {of} strata reached, {quiet} quiet: {} maintained, {} derived again {why:?}; no delta for {:?}", maintained.maintained, maintained.derived_again, driver.no_delta.iter().take(8).collect::<Vec<_>>())
            });
            got
        }
        None => super::super::run_atoms_native_hooked(part, db, "main", stats.clone(), wanted, modes, super::super::Hooks { texts: Some(Arc::clone(texts)), ..Default::default() })?,
    };
    drop(s);
    let super::Binder { constants, mut measured_kept, bound_old, lazies, .. } = binder;
    let s = span("collect");
    let ran_native: BTreeSet<&str> = got.native.iter().flat_map(|k| k.split('/')).collect();
    let rows = derived
        .iter()
        .filter(|rel| !got.inlined.contains(rel.as_str()) && !maintained.views.contains(rel.as_str()))
        .map(|rel| (rel, part_f.width(rel)))
        .filter(|(_, width)| *width > 0)
        .map(|(rel, width)| {
            let held = got
                .kept
                .contains(rel.as_str())
                .then(|| component::held(db, rel))
                .flatten()
                .or_else(|| ran_native.contains(rel.as_str()).then(|| component::held(db, rel)).flatten().filter(|h| h.has_levels()));
            let held = match held {
                Some(h) => h,
                None => {
                    let types = retain::types_of(&part_placed, &part.domains, rel);
                    InRel::flat(&held_rows(db, &got, rel, width)?, types)
                }
            };
            Ok((rel.clone(), width, held))
        })
        .collect::<Result<Vec<_>>>()?;
    let outputs: BTreeMap<String, (std::sync::Arc<crate::code_rule_dsl::logic::plan::output::Rows>, Vec<usize>)> = got.outputs.iter().map(|(rel, rows)| (rel.clone(), (std::sync::Arc::clone(rows), got.nodes[rel].clone()))).collect();
    drop(s);
    let _s = span("retain");
    let (patched, _, _) = super::kept_statistics(lazies, bound_old, &mut measured_kept, &rows, &HashMap::new(), &constants);
    let mut next = retain::after(kept, key, digests, &cut, rows, &outputs, &sources, measured_kept, HashMap::new(), input_docs, Default::default(), patched, retain::budget());
    if let Some(frames) = frames {
        next.with_frames(frames, &cut);
    }
    Ok(Ran { next, strata: cut.strata, maintained: maintained.maintained, derived_again: maintained.derived_again, changed: cut.changed.clone(), again: maintained.again.clone(), outputs: got.outputs, bound })
}

/// A relation's rows as `next` keeps them (`None`: not kept).
pub(in crate::code_rule_dsl) fn rows_of(next: &retain::Retained, rel: &str) -> Option<Vec<Vec<i64>>> {
    let (held, width) = next.kept(rel)?;
    let flat = held.live_rows()?;
    Some(flat.chunks_exact(width.max(1)).map(<[i64]>::to_vec).collect())
}
