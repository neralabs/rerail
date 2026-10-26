//! Ordered binding continuations. Only atomic projections enter matcher TLS.
use super::{Context, Cost, NativeHost};
use crate::code_rule_dsl::matcher::{
    Semantic,
    cohort::{self, Programs, Request, SemanticRequest},
    semantic_batch as facts,
};
use ast_grep_core::{Doc, meta_var::MetaVarEnv};
use std::{collections::BTreeMap, sync::Arc};

fn leaf<'t, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>>(
    q: &SemanticRequest<'t, D>,
    env: Arc<MetaVarEnv<'t, D>>,
) -> SemanticRequest<'t, D> {
    let mut request = q.request.clone();
    request.env = env;
    SemanticRequest {
        semantic: q.semantic.clone(),
        request,
        utility_depth: q.utility_depth,
    }
}
fn truth(v: facts::Verdict) -> u8 {
    match v {
        facts::Verdict::Pass => 2,
        facts::Verdict::Unknown => 1,
        facts::Verdict::Fail => 0,
    }
}

/// One ordered conjunction per lane; different lanes' current children form one cohort.
fn blocks<'r, 't, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>, C: Context>(
    host: &mut NativeHost<C>,
    programs: &Programs,
    jobs: Vec<(
        &SemanticRequest<'t, D>,
        &facts::Children<'r, 't, D>,
        Arc<MetaVarEnv<'t, D>>,
    )>,
) -> Vec<cohort::Result<'t, D>> {
    let mut out: Vec<_> = jobs
        .iter()
        .map(|(_, b, e)| Cost::default().answer(if b.complete { 2 } else { 0 }, e.clone()))
        .collect();
    let count = jobs
        .iter()
        .map(|(_, b, _)| b.demands.len())
        .max()
        .unwrap_or(0);
    for step in 0..count {
        let ids: Vec<_> = (0..jobs.len())
            .filter(|&i| {
                out[i].truth == 2 && out[i].failure.is_none() && step < jobs[i].1.demands.len()
            })
            .collect();
        let batch: Vec<_> = ids
            .iter()
            .map(|&i| leaf(jobs[i].0, out[i].env.clone()))
            .collect();
        let positions: BTreeMap<_, _> = batch
            .iter()
            .enumerate()
            .map(|(i, q)| (q as *const _ as usize, i))
            .collect();
        let starts = host.project(&batch, |q| {
            let i = ids[positions[&(q as *const _ as usize)]];
            jobs[i].1.demands[step].start(&q.request.following)
        });
        let mut runnable = BTreeMap::<u32, Vec<(usize, Request<'t, D>)>>::new();
        for ((&i, q), p) in ids.iter().zip(&batch).zip(starts) {
            let mut cost = Cost::of(&out[i]);
            cost.add(p.cost);
            out[i] = cost.answer(out[i].truth, out[i].env.clone());
            match p.value {
                Some(facts::Started::Ready { node, guards }) if out[i].failure.is_none() => {
                    let child = &jobs[i].1.demands[step];
                    let mut req = q.request.clone();
                    req.program = programs
                        .lookup(child.rule)
                        .expect("binding child registered");
                    req.node = node;
                    req.following = Arc::new(guards);
                    runnable.entry(q.utility_depth).or_default().push((i, req));
                }
                _ => out[i].truth = 0,
            }
        }
        for (depth, requests) in runnable {
            let (ids, requests): (Vec<_>, Vec<_>) = requests.into_iter().unzip();
            for (i, r) in ids.into_iter().zip(host.run(programs, requests, depth)) {
                let mut cost = Cost::of(&out[i]);
                cost.add(Cost::of(&r));
                out[i] = cost.answer(r.truth, r.env);
            }
        }
    }
    out
}

/// Occurrences are ordered alternatives with rollback, each exclusions and capture-sensitive
/// unknown stopping. All searches advance one visit per round.
fn occurrences<
    'r,
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
    C: Context,
>(
    host: &mut NativeHost<C>,
    programs: &Programs,
    jobs: Vec<(
        &SemanticRequest<'t, D>,
        Option<&facts::Occurrences<'r, 't, D>>,
        Arc<MetaVarEnv<'t, D>>,
    )>,
) -> Vec<cohort::Result<'t, D>> {
    let mut out: Vec<_> = jobs
        .iter()
        .map(|(_, s, e)| Cost::default().answer(if s.is_none() { 2 } else { 0 }, e.clone()))
        .collect();
    let mut done: Vec<_> = jobs.iter().map(|(_, s, _)| s.is_none()).collect();
    let mut unknown = vec![false; jobs.len()];
    let count = jobs
        .iter()
        .filter_map(|(_, s, _)| s.map(|s| s.demands.len()))
        .max()
        .unwrap_or(0);
    for step in 0..count {
        let ids: Vec<_> = (0..jobs.len())
            .filter(|&i| {
                !done[i] && out[i].failure.is_none() && step < jobs[i].1.unwrap().demands.len()
            })
            .collect();
        let batch: Vec<_> = ids
            .iter()
            .map(|&i| leaf(jobs[i].0, jobs[i].2.clone()))
            .collect();
        let positions: BTreeMap<_, _> = batch
            .iter()
            .enumerate()
            .map(|(i, q)| (q as *const _ as usize, i))
            .collect();
        let starts = host.project(&batch, |q| {
            let i = ids[positions[&(q as *const _ as usize)]];
            jobs[i].1.unwrap().demands[step].start(&q.request.following)
        });
        let mut runnable = BTreeMap::<u32, Vec<(usize, Request<'t, D>)>>::new();
        for ((&i, q), p) in ids.iter().zip(&batch).zip(starts) {
            let mut cost = Cost::of(&out[i]);
            cost.add(p.cost);
            out[i] = cost.answer(out[i].truth, out[i].env.clone());
            if let Some(facts::Started::Ready { node, guards }) = p.value {
                let mut req = q.request.clone();
                req.program = programs
                    .lookup(jobs[i].1.unwrap().demands[step].rule)
                    .expect("occurrence child registered");
                req.node = node;
                req.following = Arc::new(guards);
                runnable.entry(q.utility_depth).or_default().push((i, req));
            }
        }
        for (depth, requests) in runnable {
            let (ids, requests): (Vec<_>, Vec<_>) = requests.into_iter().unzip();
            let answers = host.run(programs, requests, depth);
            let checks: Vec<_> = ids
                .iter()
                .zip(&answers)
                .map(|(&i, r)| leaf(jobs[i].0, r.env.clone()))
                .collect();
            let excluded = host.project(&checks, |q| facts::excluded(&q.request.env));
            for ((i, r), e) in ids.into_iter().zip(answers).zip(excluded) {
                let mut cost = Cost::of(&out[i]);
                cost.add(Cost::of(&r));
                cost.add(e.cost);
                if r.truth == 2 && !e.value.unwrap_or(true) {
                    done[i] = true;
                    out[i] = cost.answer(2, r.env);
                } else {
                    if r.truth == 1 {
                        unknown[i] = true;
                        if jobs[i].1.unwrap().strict_unknown {
                            done[i] = true;
                        }
                    }
                    out[i] = cost.answer(if unknown[i] { 1 } else { 0 }, jobs[i].2.clone());
                }
            }
        }
    }
    for i in 0..out.len() {
        if !done[i] {
            out[i].truth = if unknown[i] { 1 } else { 0 };
        }
    }
    out
}

struct Lane<'r, 't, D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>> {
    candidates: Vec<facts::Candidate<'r, 't, D>>,
    binds: Option<facts::BindsProjection<'r, 't, D>>,
    next: usize,
    unknown: bool,
    done: bool,
    cost: Cost,
    env: Arc<MetaVarEnv<'t, D>>,
    truth: u8,
}

pub(super) fn evaluate<
    't,
    D: Doc<Source: ast_grep_core::source::Content<Underlying = u8>>,
    C: Context,
>(
    host: &mut NativeHost<C>,
    programs: &Programs,
    leaves: Vec<SemanticRequest<'t, D>>,
) -> Vec<cohort::Result<'t, D>> {
    let initial = host.project(&leaves, |q| match q.semantic.as_ref() {
        Semantic::Binding(t) => match facts::binding(t, &q.request.node, &q.request.env) {
            facts::BindingProjection::No => (Vec::new(), None, false),
            facts::BindingProjection::Unknown => (Vec::new(), None, true),
            facts::BindingProjection::Found { own, occurrences } => (
                vec![facts::Candidate {
                    ordinal: 0,
                    pre: facts::Verdict::Pass,
                    own,
                    occurrences,
                    shadow: facts::Verdict::Pass,
                    previous: None,
                }],
                None,
                false,
            ),
        },
        Semantic::Binds(t) => {
            let b = facts::binds(t, &q.request.node);
            let c = b
                .as_ref()
                .map_or_else(Vec::new, |b| b.candidates(&q.request.env));
            (c, b, false)
        }
        _ => unreachable!("binding host family"),
    });
    let mut lanes: Vec<_> = initial
        .into_iter()
        .zip(&leaves)
        .map(|(p, q)| {
            let (candidates, binds, unknown) = p.value.unwrap_or((Vec::new(), None, false));
            Lane {
                candidates,
                binds,
                next: 0,
                unknown: unknown || p.cost.unknown,
                done: false,
                cost: p.cost,
                env: q.request.env.clone(),
                truth: 0,
            }
        })
        .collect();
    loop {
        let mut ids = Vec::new();
        for i in 0..lanes.len() {
            let l = &mut lanes[i];
            if l.done {
                continue;
            }
            if l.cost.failure.is_some() || l.next == l.candidates.len() {
                l.done = true;
                l.truth = if l.unknown { 1 } else { 0 };
                continue;
            }
            let c = &l.candidates[l.next];
            l.next += 1;
            if c.pre == facts::Verdict::Unknown {
                l.unknown = true;
                if c.ordinal == super::super::binds::PRIVATE {
                    l.done = true;
                    l.truth = 1;
                }
                continue;
            }
            if c.pre == facts::Verdict::Pass {
                ids.push(i);
            }
        }
        if ids.is_empty() {
            if lanes.iter().all(|l| l.done) {
                break;
            }
            continue;
        }
        let own = blocks(
            host,
            programs,
            ids.iter()
                .map(|&i| {
                    (
                        &leaves[i],
                        &lanes[i].candidates[lanes[i].next - 1].own,
                        leaves[i].request.env.clone(),
                    )
                })
                .collect(),
        );
        let mut trials: Vec<_> = own;
        let occ_ids: Vec<_> = (0..ids.len())
            .filter(|&j| trials[j].truth == 2 && trials[j].failure.is_none())
            .collect();
        let occ = occurrences(
            host,
            programs,
            occ_ids
                .iter()
                .map(|&j| {
                    let i = ids[j];
                    (
                        &leaves[i],
                        lanes[i].candidates[lanes[i].next - 1].occurrences.as_ref(),
                        trials[j].env.clone(),
                    )
                })
                .collect(),
        );
        for (j, r) in occ_ids.into_iter().zip(occ) {
            let mut c = Cost::of(&trials[j]);
            c.add(Cost::of(&r));
            trials[j] = c.answer(r.truth, r.env);
        }
        let mut prev_ids = Vec::new();
        for j in 0..ids.len() {
            let i = ids[j];
            let c = &lanes[i].candidates[lanes[i].next - 1];
            if trials[j].truth == 2 {
                if c.shadow != facts::Verdict::Pass {
                    trials[j].truth = truth(c.shadow);
                } else if c.previous.is_some() {
                    prev_ids.push(j);
                }
            }
        }
        let prev = blocks(
            host,
            programs,
            prev_ids
                .iter()
                .map(|&j| {
                    let i = ids[j];
                    (
                        &leaves[i],
                        lanes[i].candidates[lanes[i].next - 1]
                            .previous
                            .as_ref()
                            .unwrap(),
                        trials[j].env.clone(),
                    )
                })
                .collect(),
        );
        for (j, r) in prev_ids.into_iter().zip(prev) {
            let mut c = Cost::of(&trials[j]);
            c.add(Cost::of(&r));
            trials[j] = c.answer(r.truth, r.env);
        }
        // Other-definition filters read the original caller, after own demands complete.
        let other_ids: Vec<_> = (0..ids.len())
            .filter(|&j| {
                let i = ids[j];
                trials[j].truth == 2 && trials[j].failure.is_none() && lanes[i].binds.is_some()
            })
            .collect();
        let batch: Vec<_> = other_ids
            .iter()
            .map(|&j| leaf(&leaves[ids[j]], leaves[ids[j]].request.env.clone()))
            .collect();
        let positions: BTreeMap<_, _> = batch
            .iter()
            .enumerate()
            .map(|(i, q)| (q as *const _ as usize, i))
            .collect();
        let others = host.project(&batch, |q| {
            let j = other_ids[positions[&(q as *const _ as usize)]];
            let i = ids[j];
            let c = &lanes[i].candidates[lanes[i].next - 1];
            lanes[i]
                .binds
                .as_ref()
                .unwrap()
                .others(&q.request.env, c.ordinal)
        });
        let mut streams = Vec::new();
        for (j, p) in other_ids.into_iter().zip(others) {
            let i = ids[j];
            let mut c = Cost::of(&trials[j]);
            c.add(p.cost);
            trials[j] = c.answer(trials[j].truth, trials[j].env.clone());
            // Python shadow redefines have already run; no other-definition reduction applies.
            if lanes[i].candidates[lanes[i].next - 1].previous.is_none() {
                let has_redefines = match leaves[i].semantic.as_ref() {
                    Semantic::Binds(t) => t.redefines.is_some(),
                    _ => false,
                };
                if has_redefines {
                    streams.push((
                        j,
                        p.value.unwrap_or_default(),
                        0usize,
                        false,
                        false,
                        trials[j].env.clone(),
                    ));
                }
            }
        }
        loop {
            let mut run = Vec::new();
            for s in 0..streams.len() {
                let (j, others, next, done, unknown, _) = &mut streams[s];
                if *done {
                    continue;
                }
                let i = ids[*j];
                let every = lanes[i].binds.as_ref().unwrap().every();
                if *next == others.len() {
                    trials[*j].truth = if *unknown {
                        1
                    } else if every {
                        2
                    } else {
                        0
                    };
                    *done = true;
                    continue;
                }
                let other = &others[*next];
                *next += 1;
                match other.pre {
                    facts::Verdict::Pass => run.push(s),
                    facts::Verdict::Unknown => {
                        if every
                            || lanes[i].candidates[lanes[i].next - 1].ordinal
                                == super::super::binds::PRIVATE
                        {
                            trials[*j].truth = 1;
                            *done = true;
                        } else {
                            *unknown = true;
                        }
                    }
                    facts::Verdict::Fail => {
                        if every {
                            trials[*j].truth = 0;
                            *done = true;
                        }
                    }
                }
            }
            if run.is_empty() {
                if streams.iter().all(|s| s.3) {
                    break;
                }
                continue;
            }
            let answers = blocks(
                host,
                programs,
                run.iter()
                    .map(|&s| {
                        let (j, others, next, _, _, base) = &streams[s];
                        let i = ids[*j];
                        let every = lanes[i].binds.as_ref().unwrap().every();
                        (
                            &leaves[i],
                            &others[*next - 1].children,
                            if every {
                                trials[*j].env.clone()
                            } else {
                                base.clone()
                            },
                        )
                    })
                    .collect(),
            );
            let checks: Vec<_> = run
                .iter()
                .zip(&answers)
                .map(|(&s, r)| leaf(&leaves[ids[streams[s].0]], r.env.clone()))
                .collect();
            let excluded = host.project(&checks, |q| facts::excluded(&q.request.env));
            for ((s, r), e) in run.into_iter().zip(answers).zip(excluded) {
                let (j, _, _, done, unknown, _) = &mut streams[s];
                let i = ids[*j];
                let every = lanes[i].binds.as_ref().unwrap().every();
                let mut c = Cost::of(&trials[*j]);
                c.add(Cost::of(&r));
                c.add(e.cost);
                if r.failure.is_some() || e.cost.failure.is_some() {
                    *done = true;
                    trials[*j] = c.answer(0, trials[*j].env.clone());
                    continue;
                }
                if r.truth == 2 {
                    if !e.value.unwrap_or(true) {
                        trials[*j] = c.answer(2, r.env);
                        if !every {
                            *done = true;
                        }
                    } else {
                        trials[*j] = c.answer(trials[*j].truth, trials[*j].env.clone());
                    }
                } else {
                    trials[*j] = c.answer(r.truth, trials[*j].env.clone());
                    if r.truth == 1 {
                        *unknown = true;
                    }
                    if every {
                        *done = true;
                    }
                }
            }
        }
        for (i, r) in ids.into_iter().zip(trials) {
            let l = &mut lanes[i];
            l.cost.add(Cost::of(&r));
            if r.truth == 2 && r.failure.is_none() {
                l.env = r.env;
                l.truth = 2;
                l.done = true;
            } else {
                l.unknown |= r.truth == 1;
            }
        }
    }
    lanes
        .into_iter()
        .map(|l| l.cost.answer(l.truth, l.env))
        .collect()
}
