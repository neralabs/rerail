//! Moved from `matcher.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

use crate::code_rule_dsl::transducer::packed::device::{ClauseRoots, Outcome, RootJobs};

/// The batch's roots with every pass on Metal in one command buffer
/// (`metal_program_backend::label_passes`), pattern leaves included (`MatcherDevice::matcher_roots`).
pub(in crate::code_rule_dsl) fn matcher_roots(this: &LoweredMatcher, tapes: &[TapeDoc]) -> Result<(Vec<BTreeSet<u32>>, Outcome), Error> {
    let no_labels = empty_sets(tapes, this.labels.defs.len());
    // A label domain is not known before the passes run: its kinds' nodes.
    let listed = match &this.domain {
        Domain::Label(_) | Domain::Within(..) => this.labels.bound(&this.domain, &mut Vec::new()).map_or(Domain::All, |kinds| Domain::Kinds(kinds.into_iter().collect())),
        other => other.clone(),
    };
    let roots = tapes.iter().zip(&no_labels).map(|(tape, sets)| domain(&listed, tape, sets).collect()).collect();
    let (mut found, outcome) = metal(&[this], this.labels.defs.len(), tapes, Some(ClauseRoots::Listed(&[roots])), None, None, None)?;
    Ok((found.pop().expect("one clause"), outcome))
}

/// Starts compiling the Metal label passes' pipelines, so a wave's first labels need not wait
/// for them (`label_passes::warm`).
pub(in crate::code_rule_dsl) fn warm() {
    metal_program_backend::label_passes::warm();
}

/// Every clause's labels over the batch on Metal, in one command buffer (as `wave_roots`'s,
/// without the roots), as views of its tapes the root programs read: `demand(batch)` then runs
/// the roots with their results (captures, spans), which a label keeps only as bits.
/// `active` as `LoweredWave::labelled`'s.
pub(in crate::code_rule_dsl) fn wave_labelled(
    this: &LoweredWave,
    tapes: &[TapeDoc],
    active: &[Vec<bool>],
    roots: Option<&RootJobs<'_>>,
) -> Result<(Vec<TapeDoc>, Outcome), Error> {
    let active = this.spread(active);
    let (_, outcome) = metal(&this.clauses.iter().collect::<Vec<_>>(), this.labels, tapes, None, Some(&active), roots, None)?;
    crate::code_rule_dsl::prof::release();
    crate::code_rule_dsl::prof::peak("3e.before_views");
    let _views = crate::code_rule_dsl::prof::Stage::start("labels.views");
    // By words, as the sweeps' sets (06 S7: bit by bit, 0.3 s).
    let sets = to_sets(&outcome.labels, this.labels, outcome.words, tapes);
    let views = tapes.iter().zip(sets).map(|(tape, sets)| tape.with_labels(sets)).collect();
    Ok((views, outcome))
}

/// The sources of the wave's pattern kernels on Metal, in the plan's order (`metal`'s passes: the patterns as
/// `levels` visits them, and the fact case when a clause has facts), for tapes of `format`.
pub(in crate::code_rule_dsl) fn wave_pattern_sources(this: &LoweredWave, format: u32) -> Result<Vec<String>, Error> {
    let clauses = this.clauses.iter().collect::<Vec<_>>();
    let mut patterns = Vec::new();
    let mut facts = false;
    levels(&clauses, |clause, id| {
        match clause.labels.def(id) {
            Label::Fact(..) => facts = true,
            Label::Pattern(pattern, _) | Label::Constrained(pattern, _, _) => patterns.push(pattern.ast_pattern.as_ref().ok_or(Error::UnsupportedPattern)?),
            _ => {}
        }
        Ok(())
    })?;
    Ok(metal_program_backend::label_passes::pattern_sources(&patterns, facts, format))
}

/// Each clause's matches of the batch from `roots` (the candidates its root program starts from: listed, or its
/// domain's nodes where it runs), every clause's passes and roots in one command buffer.
///
/// A pass or root walk left unfinished on a file (decision 0334) goes to W's matcher where that is cheaper
/// (`LoweredWave::label_w_cost`), in `Outcome::unfinished`.
pub(in crate::code_rule_dsl) fn wave_roots(this: &LoweredWave, tapes: &[TapeDoc], roots: ClauseRoots<'_>) -> Result<(Vec<Vec<BTreeSet<u32>>>, Outcome), Error> {
    let w_cost = |label: u32, file: usize| tapes.get(file).map_or(0, |tape| this.label_w_cost(label, tape));
    metal(&this.clauses.iter().collect::<Vec<_>>(), this.labels, tapes, Some(roots), None, None, Some(&w_cost))
}

/// `clauses`' matches of the batch on Metal, in one command buffer: their labels (disjoint, `labels`
/// in all) as passes, then each clause's root program from its `roots` (per tape) as one pass more.
/// The clauses' segments are merged by phase: a clause's runs of single-label steps take even
/// phases and its recursive groups odd ones, each past the clause's segment before it, and a
/// phase's segments of every clause are one segment. So non-recursive passes never re-run per
/// round, recursive groups of different clauses share their rounds, and the wave's levels are the
/// deepest clause's per phase, not their sum. `active` (per clause, per tape): where a clause's
/// passes start, none elsewhere (`LoweredWave::labelled`).
fn metal<'a>(
    clauses: &[&'a LoweredMatcher],
    labels: usize,
    tapes: &[TapeDoc],
    clause_roots: Option<ClauseRoots<'_>>,
    active: Option<&[Vec<bool>]>,
    records: Option<&metal_program_backend::label_passes::RootJobs<'_>>,
    w_cost: Option<&(dyn Fn(u32, usize) -> u64 + Sync)>,
) -> Result<(Vec<Vec<BTreeSet<u32>>>, metal_program_backend::label_passes::Outcome), Error> {
    use metal_program_backend::label_passes::{DeviceFormula, Pass, Plan, Roots, device_anchor, device_formula, device_targets, run};
    let bases = tapes
        .iter()
        .scan(0_u32, |base, tape| {
            let at = *base;
            *base += tape.transducer_len() as u32;
            Some(at)
        })
        .collect::<Vec<_>>();
    let nodes = tapes.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let words = nodes.div_ceil(32).max(1);
    // A pass's roots: its domain's nodes. A label domain's program tests the label first. On Metal
    // the label is not known before the command buffer runs, so the rule names the kinds the label
    // can have (`Labels::bound`); the walk's anchor, which reads the label (its first test), keeps
    // the walks to the label's members (decision 0365, 0424). They are a rule over each document's
    // codes (`Roots::Codes`, decision 0033), not a list: the device reads each root in place.
    // Shared by (domain, entry, the tapes the clause is active on), so passes of one domain have
    // one rule (06 S7: 605 passes, 70 M roots as lists, of 264 such keys).
    // A demand's set (`Domain::Within`, decision 0425) is not a rule over codes: the rule names its kinds, and the
    // pass's anchor holds the set (`demands`, by the roots it keys, read below).
    let mut shared = HashMap::<(Domain, Option<Vec<(u8, u16)>>, Vec<bool>), Arc<Roots>>::new();
    let mut demands = HashMap::<usize, compile_sets::Set>::new();
    let index = |label: u16| u32::from(label - LABEL_BASE);
    let mut roots = |clause: &LoweredMatcher, starts: &Domain, program: Option<&'a Program>| -> Arc<Roots> {
        let entry = program.map(Entry::of);
        let at = clauses.iter().position(|c| std::ptr::eq(*c, clause)).expect("a clause of the batch");
        let active = (0..tapes.len()).map(|tape| active.is_none_or(|active| active[at][tape])).collect::<Vec<_>>();
        let key = (starts.clone(), entry.as_ref().map(|entry| entry.0.clone()), active.clone());
        let roots = Arc::clone(shared.entry(key).or_insert_with(|| {
            let kinds = match starts {
                Domain::All => None,
                Domain::Kinds(kinds) => Some(kinds.clone()),
                Domain::Label(_) | Domain::Within(..) => clause.labels.bound(starts, &mut Vec::new()).map(|kinds| kinds.into_iter().collect()),
            };
            Arc::new(Roots::Codes { kinds, entry: entry.map(|entry| entry.0).unwrap_or_default(), active, anchor: None })
        }));
        if let Some(set) = starts.demand() {
            demands.insert(Arc::as_ptr(&roots) as usize, set.clone());
        }
        roots
    };
    // The facts' rows, before any pass: made on the host, tested on the device (`Pass::Fact`).
    let facts = fact_rows(clauses.iter().flat_map(|clause| clause.labels.facts()), tapes);
    let mut slots = 0_u32;
    let mut patterns = Vec::new();
    let mut pass = |clause: &'a LoweredMatcher, id: u16| -> Result<Pass<'a>, Error> {
        let target = index(id);
        Ok(match clause.labels.def(id) {
            Label::Fact(fact, labels) => {
                let at = clauses.iter().position(|c| std::ptr::eq(*c, clause)).expect("a clause of the batch");
                // Where each active file's findings are in the fact block, which the device reads in place (Q251).
                let found = tapes
                    .iter()
                    .enumerate()
                    .filter(|&(file, _)| active.is_none_or(|active| active[at][file]))
                    .filter_map(|(file, tape)| {
                        let key = (Arc::as_ptr(fact) as usize, Arc::as_ptr(tape.document_source()) as usize);
                        facts.get(&key).map(|found| (file as u32, *found))
                    })
                    .collect();
                Pass::Fact { target, block: Arc::clone(&facts.block), found, labels: labels.iter().map(|label| index(*label)).collect() }
            }
            Label::Pattern(pattern, starts) | Label::Constrained(pattern, starts, _) => {
                let constraints = match clause.labels.def(id) {
                    Label::Constrained(_, _, constraints) => {
                        constraints.iter().map(|(name, label)| (name.clone(), index(*label))).collect()
                    }
                    _ => Vec::new(),
                };
                let packed = pattern.ast_pattern.as_ref().ok_or(Error::UnsupportedPattern)?;
                patterns.push(packed);
                Pass::Pattern {
                    target,
                    pattern: patterns.len() - 1,
                    roots: roots(clause, starts, None),
                    constraints,
                }
            }
            Label::Roots(program, starts) | Label::Targets(program, starts) => Pass::Program {
                target,
                program,
                roots: roots(clause, starts, Some(program)),
                captured: matches!(clause.labels.def(id), Label::Targets(..)),
            },
            Label::Count {
                program,
                domain: starts,
                by,
                min,
                max,
            } => {
                slots += 1;
                Pass::Count {
                    target,
                    slot: slots - 1,
                    program,
                    roots: roots(clause, starts, Some(program)),
                    captured: matches!(by, Tally::Capture),
                    min: u32::try_from(*min).unwrap_or(u32::MAX),
                    max: max.map_or(u32::MAX, |max| u32::try_from(max).unwrap_or(u32::MAX)),
                }
            }
            Label::Complement(of) => Pass::Complement { target, of: index(*of) },
            Label::Empty => Pass::Empty { target },
        })
    };
    let (mut steps, mut chains) = crate::code_rule_dsl::prof::time("labels.plan", || levels(clauses, &mut pass))?;
    drop(pass);
    drop(shared);
    crate::code_rule_dsl::prof::release();
    crate::code_rule_dsl::prof::peak("3a.plan");
    let all = clauses;
    // A clause's roots: one label past every pass's (none where the roots run apart, `labelled_metal`). Listed, or
    // the clause's domain where it runs, as a rule over the codes (as `LoweredWave::starts` tests it: a label
    // domain's roots are every node).
    let clauses = if clause_roots.is_some() { clauses } else { &[] };
    if let Some(clause_roots) = clause_roots {
        let root = |at: usize, clause: &LoweredMatcher| match clause_roots {
            ClauseRoots::Listed(listed) => Roots::Listed(listed[at].clone(), None),
            ClauseRoots::Rule(active) => Roots::Codes {
                kinds: match &clause.domain {
                    Domain::Kinds(kinds) | Domain::Within(Some(kinds), _) => Some(kinds.clone()),
                    Domain::All | Domain::Label(_) | Domain::Within(None, _) => None,
                },
                entry: Vec::new(),
                active: active[at].clone(),
                anchor: None,
            },
        };
        steps.push((
            false,
            vec![
                clauses
                    .iter()
                    .enumerate()
                    .map(|(at, clause)| match (clause.label_copy(), clause_roots) {
                        // Its roots are a label's members in its domain: read from the label's row below, not walked.
                        (Some(_), ClauseRoots::Rule(_)) => Pass::Empty { target: (labels + at) as u32 },
                        _ => Pass::Program { target: (labels + at) as u32, program: &clause.program, roots: Arc::new(root(at, clause)), captured: false },
                    })
                    .collect(),
            ],
        ));
        chains.push((None, false));
    }
    // Decision 0365 §9, §10 (0036 on Metal): a roots or targets pass past the recursive groups whose program is a
    // node-set formula (a targets pass's: the image of its starts) is that formula on the device (`Pass::Formula`), not
    // a walk; `RERAIL_SETS=walk` walks it, as on the CPU.
    if formulas_on() {
        for (_, levels) in steps.iter_mut().filter(|(recursive, _)| !recursive) {
            for pass in levels.iter_mut().flatten() {
                let Pass::Program { target, program, roots, captured } = pass else { continue };
                let formula = if *captured { device_targets(program, roots) } else { device_formula(program) };
                let Some(formula) = formula else { continue };
                *pass = Pass::Formula { target: *target, roots: Arc::clone(roots), formula, image: *captured };
            }
        }
    }
    // Decision 0365: a walk starts within its anchor, evaluated on the device (`label_phase`). A recursive group's
    // anchors read its own labels as every node (Q133), so they hold in every round: each is evaluated once, before
    // the group's rounds (`Runner::anchors`).
    if anchors_on() {
        let _stage = crate::code_rule_dsl::prof::Stage::start("labels.plan.anchors");
        let mut anchored = HashMap::<(usize, Arc<DeviceFormula>), Arc<Roots>>::new();
        for (recursive, levels) in steps.iter_mut() {
            let own = if *recursive {
                levels
                    .iter()
                    .flatten()
                    .map(|pass| match pass {
                        Pass::Program { target, .. }
                        | Pass::Count { target, .. }
                        | Pass::Complement { target, .. }
                        | Pass::Pattern { target, .. }
                        | Pass::Fact { target, .. }
                        | Pass::Empty { target }
                        | Pass::Formula { target, .. } => *target,
                    })
                    .collect::<BTreeSet<u32>>()
            } else {
                BTreeSet::new()
            };
            for pass in levels.iter_mut().flatten() {
                let (Pass::Program { program, roots, .. } | Pass::Count { program, roots, .. }) = pass else { continue };
                let demand = demands.get(&(Arc::as_ptr(roots) as usize));
                let Some(formula) = device_anchor(program, demand, &|label| own.contains(&u32::from(label))) else { continue };
                let key = (Arc::as_ptr(roots) as usize, formula);
                let with = anchored.entry(key.clone()).or_insert_with(|| {
                    let anchor = Some(key.1.clone());
                    Arc::new(match &**roots {
                        Roots::Codes { kinds, entry, active, .. } => Roots::Codes { kinds: kinds.clone(), entry: entry.clone(), active: active.clone(), anchor },
                        Roots::Listed(lists, _) => Roots::Listed(lists.clone(), anchor),
                    })
                });
                *roots = Arc::clone(with);
            }
        }
    }
    let outcome = match active {
        // The wave's labels: a recursive group that sweeps runs on the CPU's sweep, each file a
        // walk in node order, between the GPU's wide levels (O26).
        Some(active) => apart(all, labels, slots as usize, patterns, steps, chains, tapes, active, records, w_cost)?,
        None => {
            run(&Plan { labels: labels + clauses.len(), counts: slots as usize, patterns, steps, chains, initial: Default::default(), rows: None, buffers: None, roots: records, w_cost }, tapes)?
        }
    };
    // Each clause's roots: its label's set bits, a word at a time, as each tape's local nodes.
    let found = (0..clauses.len())
        .map(|at| {
            let clause = clauses[at];
            let row_of = |label: usize| &outcome.labels[label * words..(label + 1) * words];
            // A clause that copies a label (`label_copy`): that label's row, where the clause is active, in its domain.
            let copied = match clause_roots {
                Some(ClauseRoots::Rule(active)) => clause.label_copy().map(|label| (index(label) as usize, active)),
                _ => None,
            };
            let row = row_of(copied.map_or(labels + at, |(label, _)| label));
            let within = match &clause.domain {
                Domain::Label(label) if copied.is_some() => Some(row_of(index(*label) as usize)),
                _ => None,
            };
            tapes
                .iter()
                .zip(&bases)
                .enumerate()
                .map(|(file, (tape, &base))| {
                    let mut set = BTreeSet::new();
                    if copied.is_some_and(|(_, active)| !active[at][file]) {
                        return set;
                    }
                    let (from, to) = (base as usize, base as usize + tape.transducer_len());
                    for w in from / 32..to.div_ceil(32) {
                        let mut bits = row[w] & within.map_or(u32::MAX, |within| within[w]);
                        while bits != 0 {
                            let global = 32 * w + bits.trailing_zeros() as usize;
                            bits &= bits - 1;
                            if (from..to).contains(&global) {
                                let local = (global - from) as u32;
                                if copied.is_none() || clause.domain.admits_kind(tape.transducer_kind(local)) {
                                    set.insert(local);
                                }
                            }
                        }
                    }
                    set
                })
                .collect()
        })
        .collect();
    Ok((found, outcome))
}

/// The wave's label plan with its sweeping groups apart: the steps between them as Metal runs
/// (each from the labels so far), each sweeping group as the CPU sweeps it (`sweep`, on the tapes
/// where its clauses are active). A sweep follows a file's nodes in order, one thread a file on
/// the GPU, where the interpreter's thread is far slower than a CPU core; the levels between are
/// a task per node and program, as wide as the GPU.
/// `RERAIL_FOOTPRINT`: the footprint now and its high-water mark at a point in a labelling.
fn mark(at: &str) {
    if std::env::var_os("RERAIL_FOOTPRINT").is_some() {
        let _ = tikv_jemalloc_ctl::epoch::advance();
        eprintln!(
            "  FOOTPRINT {at}: {} MB (high-water {} MB); heap live {} MB",
            crate::code_rule_dsl::prof::footprint_mb(),
            crate::code_rule_dsl::prof::max_footprint_mb(),
            tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) >> 20
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn apart<'a>(
    clauses: &[&'a LoweredMatcher],
    labels: usize,
    counts: usize,
    patterns: Vec<&'a PackedPattern>,
    steps: Vec<(bool, Vec<Vec<metal_program_backend::label_passes::Pass<'a>>>)>,
    chains: Vec<(Option<Vec<u16>>, bool)>,
    tapes: &[TapeDoc],
    active: &[Vec<bool>],
    roots: Option<&metal_program_backend::label_passes::RootJobs<'_>>,
    w_cost: Option<&(dyn Fn(u32, usize) -> u64 + Sync)>,
) -> Result<metal_program_backend::label_passes::Outcome, Error> {
    use metal_program_backend::label_passes::{Outcome, Plan, run};
    let (host, _) = levels(clauses, |clause, id| Ok((clause, id)))?;
    if host.len() != steps.len() {
        return Err(Error::BackendFailure);
    }
    let nodes = tapes.iter().map(TapeDoc::transducer_len).sum::<usize>();
    let words = nodes.div_ceil(32).max(1);
    let mut outcome = Outcome { labels: Vec::new(), words, ..Outcome::default() };
    let mut segment = (Vec::new(), Vec::new());
    // The runs label the same tapes, so a text row evaluated by one serves the rest (and the
    // sweeps between them, each a file's slice of it).
    let rows = metal_program_backend::label_passes::TextRows::default();
    let kept = metal_program_backend::label_passes::RunBuffers::default();
    let bases = tapes
        .iter()
        .scan(0_usize, |base, tape| {
            let at = *base;
            *base += tape.transducer_len();
            Some(at)
        })
        .collect::<Vec<_>>();
    // Every walk's text rows, in one pass over the files before the first run (the runs and
    // sweeps then find them in `rows`).
    {
        let walks = host
            .iter()
            .flat_map(|(_, levels)| levels.iter().flatten().copied())
            .filter_map(|(clause, id)| match clause.labels.def(id) {
                Label::Roots(program, _) | Label::Targets(program, _) => Micro::of(program).map(|walk| (walk, program)),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        let mut slots = Vec::new();
        let mut keys = Vec::new();
        for (walk, program) in &walks {
            for slot in 0..walk.texts.len() {
                let key = walk.slot_key(program, slot);
                if seen.insert(key.clone()) {
                    slots.push((walk, *program, slot));
                    keys.push(key);
                }
            }
        }
        let row_bases = bases.iter().map(|&base| base as u32).collect::<Vec<_>>();
        let evaluated = metal_program_backend::label_passes::text_rows(&slots, tapes, &row_bases, words);
        rows.lock().map_err(|_| Error::BackendFailure)?.extend(keys.into_iter().zip(evaluated));
    }
    // The roots join the last run, after every label (`Plan::roots`).
    let flush = |segment: &mut (Vec<_>, Vec<_>), outcome: &mut Outcome, last: bool| -> Result<(), Error> {
        if segment.0.is_empty() && !(last && roots.is_some()) {
            return Ok(());
        }
        let plan = Plan {
            labels,
            counts,
            patterns: patterns.clone(),
            steps: std::mem::take(&mut segment.0),
            chains: std::mem::take(&mut segment.1),
            // The labels so far, none before the first run (`Plan::initial`).
            initial: std::cell::Cell::new(Some(std::mem::take(&mut outcome.labels)).filter(|labels| !labels.is_empty())),
            rows: Some(&rows),
            buffers: Some(&kept),
            roots: if last { roots } else { None },
            w_cost,
        };
        crate::code_rule_dsl::prof::release();
        let before = crate::code_rule_dsl::prof::footprint_mb();
        mark("plan");
        let ran = crate::code_rule_dsl::prof::time("labels.gpu", || run(&plan, tapes))?;
        if std::env::var_os("RERAIL_FOOTPRINT").is_some() {
            let _ = tikv_jemalloc_ctl::epoch::advance();
            eprintln!(
                "FOOTPRINT run: {before} -> {} MB (high-water {} MB); heap live {} MB, resident {} MB; Metal {} MB",
                crate::code_rule_dsl::prof::footprint_mb(),
                crate::code_rule_dsl::prof::max_footprint_mb(),
                tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) >> 20,
                tikv_jemalloc_ctl::stats::resident::read().unwrap_or(0) >> 20,
                crate::code_rule_dsl::transducer::packed::metal_backend::reserved_mb(),
            );
            if let Some(path) = std::env::var_os("RERAIL_HOLD_RUN") {
                eprintln!("RUN holding, pid {}", std::process::id());
                while !std::path::Path::new(&path).exists() {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
        crate::code_rule_dsl::prof::peak("3b.gpu");
        outcome.labels = ran.labels;
        if !ran.records.is_empty() {
            outcome.records = ran.records;
        }
        outcome.command_buffers += ran.command_buffers;
        outcome.dispatches += ran.dispatches;
        outcome.unfinished.extend(ran.unfinished);
        outcome.replanned += ran.replanned;
        outcome.rounds = outcome.rounds.max(ran.rounds);
        (0..3).for_each(|at| outcome.waits[at] += ran.waits[at]);
        Ok(())
    };
    for ((step, chain), (recursive, levels)) in steps.into_iter().zip(chains).zip(&host) {
        if !(*recursive && metal_program_backend::label_passes::host_sweeps() && sweeps(levels)) {
            segment.0.push(step);
            segment.1.push(chain);
            continue;
        }
        flush(&mut segment, &mut outcome, false)?;
        let mut sets = crate::code_rule_dsl::prof::time("labels.to_sets", || to_sets(&outcome.labels, labels, words, tapes));
        crate::code_rule_dsl::prof::release();
        crate::code_rule_dsl::prof::peak("3c.to_sets");
        mark("to_sets");
        // The group's text rows over every file, where no run evaluated them: once each, not
        // once a file.
        {
            let row_bases = bases.iter().map(|&base| base as u32).collect::<Vec<_>>();
            for (clause, id) in levels.iter().flatten().copied() {
                let (Label::Roots(program, _) | Label::Targets(program, _)) = clause.labels.def(id) else { continue };
                let Some(walk) = Micro::of(program) else { continue };
                for slot in 0..walk.texts.len() {
                    let key = walk.slot_key(program, slot);
                    if rows.lock().map_err(|_| Error::BackendFailure)?.contains_key(&key) {
                        continue;
                    }
                    let row = metal_program_backend::label_passes::text_row(&walk, program, slot, tapes, &row_bases, words);
                    rows.lock().map_err(|_| Error::BackendFailure)?.insert(key, row);
                }
            }
        }
        mark("text rows");
        // A file's sweep reads and marks its own labels only, so the files sweep in parallel.
        use rayon::prelude::*;
        let _sweep = crate::code_rule_dsl::prof::Stage::start("labels.sweep");
        let budget = crate::code_rule_dsl::transducer::packed::WorkBudget::current();
        let given = tapes.par_iter().zip(sets.par_iter_mut()).enumerate().map(|(at, (tape, sets))| {
            let levels = levels
                .iter()
                .map(|level| {
                    level
                        .iter()
                        .copied()
                        .filter(|&(clause, _)| {
                            clauses.iter().position(|c| std::ptr::eq(*c, clause)).is_some_and(|clause| active[clause][at])
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            if levels.iter().any(|level| !level.is_empty()) {
                // The text rows the GPU runs evaluated over every file, sliced to this one.
                let base = bases[at];
                let texts = |walk: &Micro, program: &Program| {
                    (0..walk.texts.len())
                        .map(|slot| metal_program_backend::label_passes::text_slot(Some((&rows, base)), walk, program, slot, tape))
                        .collect::<Option<Vec<_>>>()
                };
                // Gone to W's matcher (decision 0334): the group's labels on this file, as far as they got.
                // The group's anchors read the tape's column sets (decision 0365, Q133): this sweep's own.
                let index = compile_sets::Index::default();
                if !sweep_with(&levels, tape, sets, &index, &texts, budget, w_cost.is_some())? {
                    return Ok(levels.iter().flatten().map(|&(_, id)| (u32::from(id - LABEL_BASE), at)).collect());
                }
            }
            Ok::<_, Error>(Vec::new())
        })
        .collect::<Result<Vec<_>, _>>()?;
        outcome.unfinished.extend(given.into_iter().flatten());
        drop(_sweep);
        crate::code_rule_dsl::prof::release();
        crate::code_rule_dsl::prof::peak("3d.sweep");
        mark(&format!(
            "swept ({} levels, {} passes, {} complements)",
            levels.len(),
            levels.iter().map(Vec::len).sum::<usize>(),
            levels.iter().flatten().filter(|&&(clause, id)| matches!(clause.labels.def(id), Label::Complement(_))).count()
        ));
        outcome.labels = crate::code_rule_dsl::prof::time("labels.to_words", || to_words(&sets, labels, words, tapes));
        mark("to_words");
    }
    flush(&mut segment, &mut outcome, true)?;
    Ok(outcome)
}
