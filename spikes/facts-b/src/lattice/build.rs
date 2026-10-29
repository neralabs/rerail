//! `lattice build`: Y2's dead-code product graph as CSR columns, from the inputs Y2 reads, and
//! Y2's own from-scratch reachability on the same inputs (timed, reachability apart from
//! findings; its reached states and levels kept as the exactness reference).
//!
//! The product graph's states are Y2's `(node, layer)` (`y2::dc`: flow, annotated, dispatch).
//! Its edges are Y2's `succ`, evaluated once per state of the closure of every state that could
//! be reached: the roots, every definition's flow state and its module's. That closure is a
//! superset of the reached set (whatever the roots), so the fixpoint from the roots over it is
//! Y2's reachability. Rows are sorted, repeats dropped (Y2's propagation ignores repeats).
//!
//! Ids: the first worktree's states sorted by `St` (so ids do not depend on the roots or on any
//! traversal order), then each later worktree's new states, sorted, appended. Layering (L10): the
//! first worktree's CSR is the base; another worktree stores only the rows that differ from the
//! base's (a bit per id, a rank per word, the replacement rows). A base state absent from a
//! worktree keeps the base's row there: no edge of that worktree reaches it (its states are closed
//! under its own `succ`) and it is not one of its roots.
//!
//! Output columns (`cols`): `base_off`, `base_tgt` (u32); `ov_bits` (u64), `ov_rank` (u32),
//! `ov_off`, `ov_tgt`, `seeds`, `y2_id`, `y2_lvl` (u32), concatenated over worktrees with their
//! ranges in `meta.json`.
use super::cols;
use crate::y2::dc::{self, Dc, St};
use crate::y2::facts::Facts;
use rustc_hash::{FxHashMap, FxHashSet};
use serde_json::json;
use std::path::Path;
use std::time::Instant;

/// One worktree's product graph: states in discovery order, their `succ` rows (as `St`, with
/// repeats), and the roots.
struct Enumerated {
    states: Vec<St>,
    off: Vec<usize>,
    tgt: Vec<St>,
    seeds: Vec<St>,
}

fn enumerate(f: &Facts) -> Enumerated {
    let mut seeds = Dc::root_states(f);
    seeds.sort_unstable();
    seeds.dedup();
    let mut seen: FxHashSet<St> = FxHashSet::default();
    let mut stack: Vec<St> = Vec::new();
    let push = |s: St, seen: &mut FxHashSet<St>, stack: &mut Vec<St>| {
        if seen.insert(s) {
            stack.push(s);
        }
    };
    for &s in &seeds {
        push(s, &mut seen, &mut stack);
    }
    for (&id, d) in f.defs.iter() {
        push(dc::st(id, dc::NONE), &mut seen, &mut stack);
        push(dc::st(d.module | dc::MOD, dc::NONE), &mut seen, &mut stack);
    }
    let mut e = Enumerated { states: Vec::new(), off: vec![0], tgt: Vec::new(), seeds };
    let mut buf = Vec::new();
    while let Some(s) = stack.pop() {
        buf.clear();
        dc::succ(f, s, &mut buf);
        e.states.push(s);
        e.tgt.extend_from_slice(&buf);
        e.off.push(e.tgt.len());
        for &t in &buf {
            push(t, &mut seen, &mut stack);
        }
    }
    e
}

fn layer_kind(s: St) -> usize {
    match dc::layer(s) {
        dc::NONE => 0,
        dc::ANN => 1,
        _ => 2,
    }
}

fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if v.is_empty() {
        f64::NAN
    } else if v.len() % 2 == 1 {
        v[v.len() / 2]
    } else {
        (v[v.len() / 2 - 1] + v[v.len() / 2]) / 2.0
    }
}

/// `lattice build --dump F [--edges Y1_FILE] --out DIR [--y2-runs N] [--limit-worktrees N]`.
pub fn run(dump: &Path, edges: Option<&Path>, out: &Path, y2_runs: usize, limit: Option<usize>) {
    std::fs::create_dir_all(out).unwrap();
    let t = Instant::now();
    let (d, y1) = crate::y2::read_dumps(dump, edges);
    let mut wts = crate::y2::worktrees(&d);
    // The reference worktree first: its graph is the base.
    if let Some(r) = d.reference_w {
        wts.retain(|w| *w != r);
        wts.insert(0, r);
    }
    if let Some(l) = limit {
        wts.truncate(l);
    }
    eprintln!("lattice build: dump read in {:.1}s, worktrees {:?}", t.elapsed().as_secs_f64(), wts);
    let mut ids: FxHashMap<St, u32> = FxHashMap::default();
    let (mut base_off, mut base_tgt): (Vec<u32>, Vec<u32>) = (vec![0], Vec::new());
    let mut layers: Vec<Vec<(u32, Vec<u32>)>> = Vec::new();
    let (mut seeds_col, mut y2_id, mut y2_lvl): (Vec<u32>, Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new(), Vec::new());
    let mut metas = Vec::new();
    for (wi, &wt) in wts.iter().enumerate() {
        let t0 = Instant::now();
        let f = crate::y2::load_facts(&d, y1.as_ref(), wt);
        let load_ms = t0.elapsed().as_secs_f64() * 1e3;
        // Y2 from scratch: reachability, then findings, `y2_runs` times.
        let (mut reach_ms, mut find_ms, mut la) = (Vec::new(), Vec::new(), Vec::new());
        let mut y2 = None;
        for _ in 0..y2_runs.max(1) {
            la.push(cols::load_average()[0]);
            let t1 = Instant::now();
            let mut dcs = Dc::scratch_reach(&f);
            reach_ms.push(t1.elapsed().as_secs_f64() * 1e3);
            let t2 = Instant::now();
            dcs.scratch_findings(&f);
            find_ms.push(t2.elapsed().as_secs_f64() * 1e3);
            y2 = Some(dcs);
        }
        let y2 = y2.unwrap();
        let levels = y2.levels();
        let succ_calls = y2.succ_calls();
        let findings = y2.finding_rows().len();
        drop(y2);
        // The part of Y2's reachability that evaluates edges: `succ` at every reached state, alone
        // (the rest is its traversal: the level heap, the reached map and the dispatch indexes).
        let mut succ_ms = Vec::new();
        let mut buf = Vec::new();
        let mut succ_out = 0usize;
        for _ in 0..y2_runs.max(1) {
            succ_out = 0;
            let t1 = Instant::now();
            for (s, _) in &levels {
                buf.clear();
                dc::succ(&f, *s, &mut buf);
                succ_out += buf.len();
            }
            succ_ms.push(t1.elapsed().as_secs_f64() * 1e3);
        }
        // The product graph.
        let t3 = Instant::now();
        let e = enumerate(&f);
        let enum_ms = t3.elapsed().as_secs_f64() * 1e3;
        let t4 = Instant::now();
        let mut fresh: Vec<St> = e.states.iter().copied().filter(|s| !ids.contains_key(s)).collect();
        fresh.sort_unstable();
        let first_new = ids.len() as u32;
        for s in &fresh {
            let n = ids.len() as u32;
            ids.insert(*s, n);
        }
        let mut rows: Vec<(u32, Vec<u32>)> = Vec::with_capacity(e.states.len());
        let mut raw_edges = 0usize;
        for (i, s) in e.states.iter().enumerate() {
            let r = &e.tgt[e.off[i]..e.off[i + 1]];
            raw_edges += r.len();
            let mut row: Vec<u32> = r.iter().map(|t| ids[t]).collect();
            row.sort_unstable();
            row.dedup();
            rows.push((ids[s], row));
        }
        rows.sort_unstable_by_key(|r| r.0);
        let mut overrides = Vec::new();
        let mut kinds = [0usize; 3];
        for s in &e.states {
            kinds[layer_kind(*s)] += 1;
        }
        let edges: usize = rows.iter().map(|r| r.1.len()).sum();
        if wi == 0 {
            // The base: every id of this worktree, in id order (ids 0..n here).
            assert_eq!(rows.len(), ids.len());
            for (v, row) in &rows {
                assert_eq!(*v as usize + 1, base_off.len());
                base_tgt.extend_from_slice(row);
                base_off.push(base_tgt.len() as u32);
            }
        } else {
            let nb = base_off.len() - 1;
            for (v, row) in rows {
                let b: &[u32] = if (v as usize) < nb { &base_tgt[base_off[v as usize] as usize..base_off[v as usize + 1] as usize] } else { &[] };
                if row != b {
                    overrides.push((v, row));
                }
            }
        }
        let assemble_ms = t4.elapsed().as_secs_f64() * 1e3;
        // Roots and Y2's reached states with their levels, by id.
        let s0 = seeds_col.len();
        seeds_col.extend(e.seeds.iter().map(|s| ids[s]));
        let mut lv: Vec<(u32, u32)> = levels.iter().map(|(s, l)| (*ids.get(s).unwrap_or_else(|| panic!("worktree {wt}: Y2 reached a state outside the closure")), *l)).collect();
        lv.sort_unstable();
        let y0 = y2_id.len();
        y2_id.extend(lv.iter().map(|x| x.0));
        y2_lvl.extend(lv.iter().map(|x| x.1));
        let mut reached_kinds = [0usize; 3];
        for (s, _) in &levels {
            reached_kinds[layer_kind(*s)] += 1;
        }
        // Y2's propagation examines the rows of the states it reaches (repeats included).
        let y2_edges = succ_out;
        let ov_edges: usize = overrides.iter().map(|r: &(u32, Vec<u32>)| r.1.len()).sum();
        let m = json!({
            "worktree": wt, "load_ms": load_ms,
            "y2": {"runs": y2_runs.max(1), "reach_ms": reach_ms, "reach_ms_median": median(&reach_ms), "findings_ms": find_ms, "findings_ms_median": median(&find_ms), "load_average_1m": la,
                   "reached": levels.len(), "reached_flow_annotated_dispatch": reached_kinds, "succ_calls": succ_calls, "succ_edges_examined": y2_edges, "findings": findings,
                   "succ_at_reached_ms": succ_ms, "succ_at_reached_ms_median": median(&succ_ms)},
            "graph": {"states": e.states.len(), "states_flow_annotated_dispatch": kinds, "edges_with_repeats": raw_edges, "edges": edges, "new_ids": fresh.len(), "first_new_id": first_new,
                      "enumerate_ms": enum_ms, "assemble_ms": assemble_ms, "replaced_rows": overrides.len(), "replaced_row_edges": ov_edges},
            "seeds": [s0, e.seeds.len()], "y2_levels": [y0, lv.len()],
        });
        eprintln!("lattice build: worktree {wt}: {}", m);
        metas.push(m);
        layers.push(overrides);
    }
    // Pad the base to the union of ids, then each worktree's replacement rows over it.
    let n = ids.len();
    while base_off.len() < n + 1 {
        base_off.push(base_tgt.len() as u32);
    }
    let words = n.div_ceil(64);
    let (mut ov_bits, mut ov_rank, mut ov_off, mut ov_tgt) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (wi, rows) in layers.iter().enumerate() {
        let (b, r, o, t) = crate::lattice::round::layer_columns(n, rows);
        metas[wi]["layer"] = json!({"bits": [ov_bits.len(), words], "off": [ov_off.len(), o.len()], "tgt": [ov_tgt.len(), t.len()]});
        ov_bits.extend(b);
        ov_rank.extend(r);
        ov_off.extend(o);
        ov_tgt.extend(t);
    }
    cols::write(&out.join("base_off.u32"), &base_off);
    cols::write(&out.join("base_tgt.u32"), &base_tgt);
    // The base's transpose, for pull rounds (`round::parallel_dir`; the base worktree only).
    let (rev_off, rev_tgt) = crate::lattice::round::transpose(&crate::lattice::round::Csr::new(&base_off, &base_tgt, n));
    cols::write(&out.join("rev_off.u32"), &rev_off);
    cols::write(&out.join("rev_tgt.u32"), &rev_tgt);
    cols::write(&out.join("ov_bits.u64"), &ov_bits);
    cols::write(&out.join("ov_rank.u32"), &ov_rank);
    cols::write(&out.join("ov_off.u32"), &ov_off);
    cols::write(&out.join("ov_tgt.u32"), &ov_tgt);
    cols::write(&out.join("seeds.u32"), &seeds_col);
    cols::write(&out.join("y2_id.u32"), &y2_id);
    cols::write(&out.join("y2_lvl.u32"), &y2_lvl);
    let meta = json!({"dump": dump.display().to_string(), "edges": edges.map(|p| p.display().to_string()), "n": n, "base_edges": base_tgt.len(), "worktrees": metas,
        "bytes": {"base": (base_off.len() + base_tgt.len()) * 4, "layers": ov_bits.len() * 8 + (ov_rank.len() + ov_off.len() + ov_tgt.len()) * 4}});
    std::fs::write(out.join("meta.json"), serde_json::to_string_pretty(&meta).unwrap()).unwrap();
    eprintln!("lattice build: {n} ids, {} base edges, {} worktrees, {:.1}s", base_tgt.len(), wts.len(), t.elapsed().as_secs_f64());
}
