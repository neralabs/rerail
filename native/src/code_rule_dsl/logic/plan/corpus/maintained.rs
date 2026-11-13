//! One maintained run of a program over its inputs (decisions 0077, 0090): the strata whose reads changed run, from
//! their deltas where they can (`maintain::rewrite`), the others' rows are bound from what was kept; what the run
//! derived is kept for the next (`retain::after`).
//!
//! The cross-file part's session path does the same with more beside it (statistics kept by delta, frames, the
//! measurement aids); this is its core, for a program run many times over growing inputs: the cone's rounds
//! (decision 0108).

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
}

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
    mut kept: Option<retain::Retained>,
    stats: &mut crate::code_rule_dsl::logic::plan::Stats,
    keep: &BTreeSet<String>,
    modes: &str,
) -> Result<Ran> {
    let span = |name: &'static str| crate::code_rule_dsl::logic::profile::span(move || format!("maintained/{name}"));
    let digests: HashMap<String, u64> = inputs.iter().map(|(rel, _, d, _)| (rel.clone(), *d)).collect();
    let sources: Vec<(String, Arc<dyn Source>)> = inputs.iter().map(|(r, s, ..)| (r.clone(), Arc::clone(s))).collect();
    // A cone's program has no split: its inputs' rows are grouped by their first node column (`group_col`).
    let no_locations = retain::Locations::new();
    let s = span("cut and rewrite");
    let (sccs, _) = crate::code_rule_dsl::logic::front::strata(&prog.program).map_err(|e| super::Error::Plan(e.to_string()))?;
    let cut = retain::cut(kept.as_ref(), key, &prog.program, &sccs, &digests)?;
    // What the strata that run read of a part kept in the frames is read back now; the rest stays in the store.
    if let (Some(k), Some(store)) = (kept.as_mut(), store) {
        k.load(store, &cut, &prog.program, &no_locations, false)?;
    }
    let frames = kept.as_mut().and_then(retain::Retained::take_frames);
    let by_name: HashMap<&str, &Arc<dyn Source>> = inputs.iter().map(|(r, s, ..)| (r.as_str(), s)).collect();
    let mut input_docs: HashMap<String, Arc<retain::DocDigests>> = HashMap::new();
    // An input's delta, found from its old rows by document.
    let mut given = |rel: &str| -> Option<(Arc<dyn Source>, Arc<dyn Source>)> {
        let kept = kept.as_ref()?;
        let new = by_name.get(rel)?;
        let old = kept.input_source(rel)?;
        let (delta, docs) = retain::document_delta(rel, old.as_ref(), kept.input_docs(rel).map(|d| d.as_ref()), new.as_ref(), retain::group_col(&prog.program, &no_locations, rel))?;
        input_docs.insert(rel.to_owned(), docs);
        Some(delta)
    };
    let mut maintained = maintain::rewrite(&cut, kept.as_ref(), prog, &sccs, &mut given)?;
    let derived = cut.derived(&sccs)?;
    {
        let program = &mut maintained.program.program;
        if maintain::levels_wanted() {
            for rel in program.rules.iter().map(|r| r.head.rel.clone()).collect::<BTreeSet<_>>() {
                program.qualifiers.entry(rel.to_string().to_string()).or_default().insert("levels".to_owned());
            }
        }
        // What is kept for the next run is stored, not inlined into its readers (decision 0099).
        for rel in derived.iter().chain(keep) {
            let q = program.qualifiers.entry(rel.clone()).or_default();
            q.remove("inline");
            q.insert("no_inline".to_owned());
        }
    }
    drop(s);
    let s = span("bind and measure");
    let part = &maintained.program;
    // Its statistics made when a stratum first plans with it (decision 0101 §4): most strata a round reaches are
    // quiet or join an empty delta, and never do.
    let mut bind = |rel: &str, source: Arc<dyn Source>| -> Result<()> {
        let measured = Arc::clone(&source);
        let lazy = crate::code_rule_dsl::logic::plan::LazyStats::new(move || {
            let _m = crate::code_rule_dsl::logic::profile::span(|| "maintained/measure".into());
            (syntax::statistics(measured.as_ref()).into_iter().map(|(c, rows, ndv, sq, sqd)| (c, [rows, ndv, sq, sqd])).collect(), Vec::new(), ())
        });
        stats.lazily(rel, source.len(), lazy);
        modules.bind(db, rel, source)?;
        Ok(())
    };
    for (rel, source, _, bound) in inputs {
        if !bound {
            bind(rel, Arc::clone(source))?;
        }
    }
    let clean = kept.as_ref().map(|k| cut.sources(k, &part.program)).unwrap_or_default();
    for (rel, source, held) in clean.into_iter().chain(maintained.sources.iter().cloned()) {
        // A derived relation kept is read in place, with its indexes (decision 0090 step 3).
        if let Some(held) = &held {
            component::adopt(db, &rel, Arc::clone(held));
        }
        bind(&rel, source)?;
    }
    let mut wanted = keep.clone();
    wanted.extend(derived.iter().filter(|rel| !part.program.outputs.contains(*rel)).cloned());
    drop(s);
    let s = span("plan and evaluate");
    let got = super::super::run_atoms_native_over(part, db, "main", stats.clone(), wanted, modes)?;
    drop(s);
    let s = span("collect");
    let ran_native: BTreeSet<&str> = got.native.iter().flat_map(|k| k.split('/')).collect();
    let rows = derived
        .iter()
        .filter(|rel| !got.inlined.contains(rel.as_str()) && !maintained.views.contains(rel.as_str()))
        .map(|rel| (rel, part.program.decls[rel].len()))
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
                    let types = part.program.decls[rel].iter().map(|(_, t)| if t == "sym" { b's' } else { b'i' }).collect();
                    InRel::flat(&held_rows(db, &got, rel, width)?, types)
                }
            };
            Ok((rel.clone(), width, held))
        })
        .collect::<Result<Vec<_>>>()?;
    let outputs: BTreeMap<String, (Vec<String>, Vec<usize>)> = got.outputs.iter().map(|(rel, lines)| (rel.clone(), (lines.clone(), got.nodes[rel].clone()))).collect();
    drop(s);
    let _s = span("retain");
    let mut next = retain::after(kept, key, digests, &cut, rows, &outputs, &sources, HashMap::new(), HashMap::new(), input_docs, Default::default(), HashMap::new(), retain::budget());
    if let Some(frames) = frames {
        next.with_frames(frames, &cut);
    }
    Ok(Ran { next, strata: cut.strata, maintained: maintained.maintained, derived_again: maintained.derived_again, changed: cut.changed.clone(), again: maintained.again.clone() })
}

/// A relation's rows as `next` keeps them (`None`: not kept).
pub(in crate::code_rule_dsl) fn rows_of(next: &retain::Retained, rel: &str) -> Option<Vec<Vec<i64>>> {
    let (held, width) = next.kept(rel)?;
    let flat = held.live_rows()?;
    Some(flat.chunks_exact(width.max(1)).map(<[i64]>::to_vec).collect())
}
