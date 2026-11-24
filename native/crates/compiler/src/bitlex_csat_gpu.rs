//! The prover's circuit solver on the device (decision 0670, step 3): CDCL over a batch of formulas, a SIMD group a
//! formula (`bitlex_csat_gpu.uglykl`), each read where it was built. A formula's gates, fanout and plain clauses are
//! in the compile's formula block (`bitlex_grown`), as the CPU's solver reads them; the kernel binds the block once
//! and finds each formula by its descriptor's offsets. Nothing is uploaded (decision 0353): what the device has of
//! its own is each formula's state (values, trail, watches, learned clauses), its working memory.
//!
//! The answers are the CPU's (`bitlex_csat::Solver`): satisfiable, unsatisfiable, or unknown past the conflicts'
//! budget or the learned clauses' room, sound either way. The search is its own (a round's implications in lane
//! order, the integer activities' heap), and `bitlex_csat_lanes` is the same search on the CPU: the same answer,
//! model and counts on every backend and without a device.
//!
//! Memory, a batch: a descriptor of `DESC` words a formula, in the block; the slots' states, each its largest
//! formula's `state_words`. A slot is a threadgroup of `LANES` (a SIMD group), which takes the batch's formulas from
//! a queue one after another: as many slots as fit in `STATE_BYTES`, so a batch's memory is bounded however many
//! formulas it holds, and the device has as many searches running at once as it keeps resident, each hiding the
//! others' memory reads. (With a state a formula, a batch of 768 of a 31,841-variable formula, 2.5 GB, took 28
//! times what 384 did.) Each launch runs a slot `STEPS` steps at most, the GPU's time a launch bounded.

pub use crate::bitlex_csat_lanes::{Answer, Formula, Solved};
use crate::bitlex_csat_lanes::{arena, learned};
use crate::bitlex_grown::{FormulaBlock, Grown};
use rerail_gpu::device::ComputeQueue;
use rerail_gpu::kernels::{alloc, alloc_working, launch, program, read};

const KERNEL: &str = include_str!("bitlex_csat_gpu.uglykl");

/// Whether the prover's batches solve on the GPU first (`RERAIL_PROVER_GPU=1`).
pub fn enabled() -> bool {
    std::env::var_os("RERAIL_PROVER_GPU").is_some_and(|v| v == "1")
}

/// A descriptor's words (the kernel's `DESC`).
const DESC: usize = 20;
/// A formula's lanes: one SIMD group.
const LANES: u64 = 32;
/// The steps a launch runs a slot at most (a round of propagation, a conflict, a decision).
const STEPS: u32 = 1 << 12;
/// The slots' states at most, in bytes.
pub const STATE_BYTES: usize = 1 << 30;
/// The slots at most.
const SLOTS: usize = 1 << 13;
/// The state's header (the kernel's `HEADER`) and the statuses it reports.
const HEADER: usize = 32;
const SAT: u32 = 1;
const UNSAT: u32 = 2;
const ROOMLESS: u32 = 4;
const BROKEN: u32 = 5;
/// The words a formula's report holds (`out`).
const OUT: usize = 16;

#[repr(C)]
#[derive(Clone, Copy)]
struct Args {
    n: u32,
    steps: u32,
    init: u32,
    slots: u32,
    slot_words: u32,
    pad: [u32; 3],
}
// SAFETY: eight u32s, no padding.
unsafe impl rerail_gpu::Pod for Args {}

/// A formula's state, in words (the kernel's layout, `cs_setup`), its widest gate `wide - 2` wide.
fn state_words(vars: usize, clauses: usize, wide: usize) -> usize {
    let (n, l, a) = (vars + 1, learned(vars) as usize, arena(vars) as usize);
    HEADER + 11 * n + 2 * wide + LANES as usize + l + (l + 1) + 4 * (clauses + l) + a
}

/// A formula's widest gate's width and 2 (at least 4): the room of a conflict's clause or an explanation.
fn wide(gates: &crate::bitlex_csat::Gates) -> usize {
    (0..gates.len()).map(|g| gates.args(g).len()).max().unwrap_or(0).max(2) + 2
}

/// A byte offset in the block as the kernel reads it: in words.
fn words(at: usize) -> Result<u32, String> {
    if at % 4 != 0 {
        return Err(format!("the prover's GPU: an offset {at} not of a word"));
    }
    u32::try_from(at / 4).map_err(|_| format!("the prover's GPU: an offset {at} past 16 GiB"))
}

/// Each formula solved on the process's device. An error where there is no device, the kernel does not compile, or
/// a formula is not in the block the others are.
pub fn solve(formulas: &[Formula]) -> Result<Vec<Solved>, String> {
    solve_in(formulas, STATE_BYTES)
}

/// `solve`, with the slots' states in `state_bytes` at most (one slot at least).
pub fn solve_in(formulas: &[Formula], state_bytes: usize) -> Result<Vec<Solved>, String> {
    if formulas.is_empty() {
        return Ok(Vec::new());
    }
    let shared = rerail_gpu::shared::get().ok_or("the prover's GPU: no device")?;
    let function = program("bitlex_csat_gpu.uglykl", KERNEL)?.function("csat_solve")?;
    let elsewhere = || "the prover's GPU: a formula not in the batch's block".to_string();
    let block: FormulaBlock = formulas[0].gates.placed().ok_or_else(elsewhere)?.0.clone();
    let mut table = Grown::<u32>::in_block(&block);
    let (mut slot_words, mut model_words) = (0usize, 0usize);
    let mut at = Vec::with_capacity(formulas.len());
    for f in formulas {
        let (b, rec, args) = f.gates.placed().ok_or_else(elsewhere)?;
        let (b2, lits, ends) = f.clauses.placed().ok_or_else(elsewhere)?;
        let (fs, fg) = f.fan.placed().ok_or_else(elsewhere)?;
        if !b.same(&block) || !b2.same(&block) {
            return Err(elsewhere());
        }
        let (os, og) = match f.own {
            Some(o) => {
                let (s, g) = o.placed().ok_or_else(elsewhere)?;
                (words(s)?, words(g)?)
            }
            None => (u32::MAX, u32::MAX),
        };
        let wide = wide(f.gates);
        let model = u32::try_from(model_words).map_err(|_| "the prover's GPU: a batch's models past 16 GiB".to_string())?;
        let d: [u32; DESC] = [
            f.vars as u32,
            f.base as u32,
            f.gates.len() as u32,
            words(rec)?,
            words(args)?,
            words(fs)?,
            words(fg)?,
            os,
            og,
            words(lits)?,
            words(ends)?,
            f.clauses.len() as u32,
            model,
            learned(f.vars),
            arena(f.vars),
            f.conflicts,
            f.fan.vars() as u32,
            wide as u32,
            0,
            0,
        ];
        table.extend_from_slice(&d);
        at.push(model_words);
        model_words += (f.vars + 1).div_ceil(32);
        slot_words = slot_words.max(state_words(f.vars, f.clauses.len(), wide));
    }
    let n = formulas.len();
    let slots = (state_bytes / (4 * slot_words)).clamp(1, SLOTS).min(n);
    if slots * slot_words > u32::MAX as usize {
        return Err("the prover's GPU: the slots' states past 16 GiB".to_string());
    }
    let table_at = table.placed().expect("in the block").1;
    let blk = block.block().device_buffer(&shared.device)?;
    // The device's own memory: the host never reads a state.
    let state = alloc_working::<u32>(slots * slot_words)?;
    let out = alloc::<u32>(n * OUT)?;
    let models = alloc::<u32>(model_words.max(1))?;
    let queue = alloc::<u32>(1 + slots)?;
    let mut q = ComputeQueue::new(&shared.queue);
    let run = |q: &mut ComputeQueue, init: u32| -> Result<(), String> {
        let a = Args { n: n as u32, steps: STEPS, init, slots: slots as u32, slot_words: slot_words as u32, pad: [0; 3] };
        launch(q, function)
            .value("args", &a)
            .buffer("blk", &blk)
            .buffer_at("table", &blk, table_at as u64)
            .buffer("state", &state)
            .buffer("out", &out)
            .buffer("models", &models)
            .buffer("queue", &queue)
            .run(&[slots as u64], &[LANES])?;
        q.flush().await_()
    };
    run(&mut q, 1)?;
    // Each formula ends within `MAX_STEPS` of its own: past the launches that takes, the device is at fault.
    let most = n.div_ceil(slots) * ((crate::bitlex_csat_lanes::STEPS as usize).div_ceil(STEPS as usize) + 1) + 1;
    let mut got = Vec::new();
    for launches in 0.. {
        if launches > most {
            return Err(format!("the prover's GPU: a batch past {most} launches"));
        }
        run(&mut q, 0)?;
        got = read::<u32>(&out, n * OUT);
        if got.chunks(OUT).all(|o| o[0] != 0) {
            break;
        }
    }
    if let Some(i) = got.chunks(OUT).position(|o| o[0] == BROKEN) {
        return Err(format!("the device's solver broke on formula {i}: a loop past its bound"));
    }
    let any_sat = got.chunks(OUT).any(|o| o[0] == SAT);
    let words = if any_sat { read::<u32>(&models, model_words) } else { Vec::new() };
    drop(table);
    Ok(got
        .chunks(OUT)
        .zip(formulas)
        .zip(at)
        .map(|((o, f), m)| Solved {
            answer: match o[0] {
                SAT => Answer::Sat((0..=f.vars).map(|v| words[m + v / 32] >> (v % 32) & 1 != 0).collect()),
                UNSAT => Answer::Unsat,
                _ => Answer::Unknown,
            },
            roomless: o[0] == ROOMLESS,
            conflicts: o[1],
            decisions: o[2],
            rounds: o[3],
            learned: o[4],
            learned_literals: o[5],
            visits: o[6],
            items: o[7],
            reductions: o[8],
            cycles: [o[9], o[10], o[11], o[12], o[13], o[14]],
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitlex_csat::{Fanout, Gates, Solver};
    use crate::bitlex_csat_lanes::tests::{circuit, holds, same_search, Rng};
    use crate::bitlex_sat::{Answer as CpuAnswer, Clauses};

    /// Random circuits, some with a solve's own gates, through fewer slots than formulas: the device's answers are the
    /// CPU solver's, each model it gives holds, and its search is `bitlex_csat_lanes`'s.
    #[test]
    fn the_devices_answers_are_the_cpus() {
        if rerail_gpu::shared::get().is_none() {
            eprintln!("no device: the prover's GPU solver test did not run");
            return;
        }
        let block = FormulaBlock::new();
        let mut rng = Rng(0x2545f4914f6cdd1d);
        let mut made = Vec::new();
        for i in 0..400usize {
            let inputs = 4 + rng.below(40) as usize;
            let gates_n = rng.below(120) as usize;
            let clauses_n = rng.below(2 * inputs as u64 + 4) as usize;
            let own = if i % 3 == 0 { rng.below(10) as usize } else { 0 };
            made.push(circuit(&mut rng, &block, inputs, gates_n, clauses_n, own));
        }
        let fans: Vec<(Fanout, Fanout)> = made.iter().map(|(g, _, vars, base)| (Fanout::of(g, *base, *vars), Fanout::of_range(g, *base..g.len(), *vars))).collect();
        let formulas: Vec<Formula> = made
            .iter()
            .zip(&fans)
            .map(|((g, c, vars, base), (fan, own))| Formula { vars: *vars, gates: g, base: *base, fan, own: (*base < g.len()).then_some(own), clauses: c, conflicts: 100_000 })
            .collect();
        // A megabyte of states: a few dozen slots, each taking formulas from the queue one after another.
        let got = solve_in(&formulas, 1 << 20).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
        for (i, (f, x)) in formulas.iter().zip(&got).enumerate() {
            same_search(i, x, &crate::bitlex_csat_lanes::solve(f));
        }
        let (mut sat, mut unsat, mut searched) = (0, 0, 0);
        let mut s = Solver::default();
        for (i, (((g, c, vars, base), (fan, _)), x)) in made.iter().zip(&fans).zip(&got).enumerate() {
            s.reset(*vars);
            for cl in c.iter() {
                s.add(cl);
            }
            let cpu = s.solve(g, fan, *base, 100_000);
            match (&x.answer, cpu) {
                (Answer::Sat(m), CpuAnswer::Sat) => {
                    assert!(holds(m, g, c), "formula {i}: the device's model does not hold");
                    sat += 1;
                }
                (Answer::Unsat, CpuAnswer::Unsat) => unsat += 1,
                (a, b) => panic!("formula {i} ({vars} vars, {} gates, {} clauses): the device {a:?}, the CPU {b:?}", g.len(), c.len()),
            }
            if x.conflicts > 0 {
                searched += 1;
            }
        }
        eprintln!("{sat} satisfiable, {unsat} unsatisfiable, {searched} with conflicts");
        assert!(sat >= 40 && unsat >= 40 && searched >= 40, "{sat} sat, {unsat} unsat, {searched} searched");
    }

    /// With no conflict to spend, the device is unsatisfiable exactly where the CPU's solver is with none spent (a
    /// conflict at level 0): the prover's rounds rest on it (decision 0654, its kernel's test ported).
    #[test]
    fn with_no_conflicts_the_device_is_level_0s_propagation() {
        if rerail_gpu::shared::get().is_none() {
            eprintln!("no device: the prover's GPU solver test did not run");
            return;
        }
        let block = FormulaBlock::new();
        let mut rng = Rng(0x9e3779b97f4a7c15);
        let made: Vec<_> = (0..400usize).map(|i| {
            let inputs = 4 + rng.below(40) as usize;
            let (g, c) = (rng.below(120) as usize, rng.below(2 * inputs as u64 + 4) as usize);
            let own = if i % 3 == 0 { rng.below(10) as usize } else { 0 };
            circuit(&mut rng, &block, inputs, g, c, own)
        }).collect();
        let fans: Vec<(Fanout, Fanout)> = made.iter().map(|(g, _, vars, base)| (Fanout::of(g, *base, *vars), Fanout::of_range(g, *base..g.len(), *vars))).collect();
        let formulas: Vec<Formula> = made
            .iter()
            .zip(&fans)
            .map(|((g, c, vars, base), (fan, own))| Formula { vars: *vars, gates: g, base: *base, fan, own: (*base < g.len()).then_some(own), clauses: c, conflicts: 0 })
            .collect();
        let got = solve(&formulas).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
        for (i, (f, x)) in formulas.iter().zip(&got).enumerate() {
            same_search(i, x, &crate::bitlex_csat_lanes::solve(f));
        }
        let (mut at_0, mut searched) = (0, 0);
        let mut s = Solver::default();
        for (i, (((g, c, vars, base), (fan, _)), x)) in made.iter().zip(&fans).zip(&got).enumerate() {
            s.reset(*vars);
            for cl in c.iter() {
                s.add(cl);
            }
            let before = s.conflicts;
            let cpu = s.solve(g, fan, *base, 100_000);
            let cpu_at_0 = cpu == CpuAnswer::Unsat && s.conflicts == before;
            assert_eq!(x.answer == Answer::Unsat, cpu_at_0, "formula {i}: the device {:?}, the CPU {cpu:?} after {} conflicts", x.answer, s.conflicts - before);
            assert_eq!(x.conflicts, 0, "formula {i}");
            at_0 += usize::from(cpu_at_0);
            searched += usize::from(s.conflicts > before);
        }
        assert!(at_0 >= 100 && searched >= 40, "{at_0} unsatisfiable at level 0, {searched} searched");
    }

    /// The formulas in `RERAIL_CSAT_DUMPS` (`Gates::dump`s), solved on the device together and on the CPU: each one's
    /// answers, conflicts and time (decision 0670; a tool, so ignored).
    #[test]
    #[ignore]
    fn the_dumped_formulas_on_the_device() {
        let Some(dir) = std::env::var_os("RERAIL_CSAT_DUMPS") else { return };
        let mut paths: Vec<_> = std::fs::read_dir(dir).expect("the dumps").map(|e| e.expect("a dump").path()).collect();
        paths.sort();
        // `RERAIL_CSAT_ONLY`: one dump, by name; `RERAIL_CSAT_COPIES`: each dump that many times in the batch (what
        // a batch of searches costs, not one's latency).
        if let Some(only) = std::env::var_os("RERAIL_CSAT_ONLY") {
            paths.retain(|p| p.file_name() == Some(only.as_os_str()));
        }
        let copies: usize = std::env::var("RERAIL_CSAT_COPIES").ok().and_then(|c| c.parse().ok()).unwrap_or(1);
        let paths: Vec<_> = paths.iter().flat_map(|p| std::iter::repeat_n(p.clone(), copies)).collect();
        let block = FormulaBlock::new();
        let made: Vec<(Gates, usize, usize, Clauses)> = paths
            .iter()
            .map(|p| Gates::undump_into(&std::fs::read_to_string(p).expect("a dump"), Gates::in_block(&block), Clauses::in_block(&block)))
            .collect();
        let fans: Vec<(Fanout, Fanout)> = made.iter().map(|(g, vars, base, _)| (Fanout::of(g, *base, *vars), Fanout::of_range(g, *base..g.len(), *vars))).collect();
        let formulas: Vec<Formula> = made
            .iter()
            .zip(&fans)
            .map(|((g, vars, base, c), (fan, own))| Formula { vars: *vars, gates: g, base: *base, fan, own: (*base < g.len()).then_some(own), clauses: c, conflicts: 100_000 })
            .collect();
        let t = std::time::Instant::now();
        let bytes = std::env::var("RERAIL_CSAT_STATE_MB").ok().and_then(|m| m.parse::<usize>().ok()).map_or(STATE_BYTES, |m| m << 20);
        let got = solve_in(&formulas, bytes).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
        eprintln!("the device: {} formulas, {:.1} ms", formulas.len(), t.elapsed().as_secs_f64() * 1e3);
        let mut s = Solver::default();
        for ((p, (g, vars, base, c)), ((x, (fan, _)), f)) in paths.iter().zip(&made).zip(got.iter().zip(&fans).zip(&formulas)).step_by(copies) {
            let t = std::time::Instant::now();
            let twin = crate::bitlex_csat_lanes::solve(f);
            eprintln!("the device's search on the CPU: {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
            same_search(0, x, &twin);
            let t = std::time::Instant::now();
            s.reset(*vars);
            for cl in c.iter() {
                s.add(cl);
            }
            let c0 = s.conflicts;
            let cpu = s.solve(g, fan, *base, 100_000);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            let answer = match &x.answer {
                Answer::Sat(_) => "Sat",
                Answer::Unsat => "Unsat",
                Answer::Unknown => "Unknown",
            };
            eprintln!(
                "{}: the device {answer}{}, {} conflicts, {} decisions, {} rounds, {} learned ({} literals), {} watch visits, {} gate items, {} reductions, Mcycles visit/items/analyze/back/decide/reduce {:?}; the CPU {cpu:?}, {} conflicts, {ms:.1} ms",
                p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
                if x.roomless { " (room full)" } else { "" },
                x.conflicts,
                x.decisions,
                x.rounds,
                x.learned,
                x.learned_literals,
                x.visits,
                x.items,
                x.reductions,
                x.cycles.map(|c| u64::from(c) * 256 / 1_000_000),
                s.conflicts - c0
            );
        }
    }
}
