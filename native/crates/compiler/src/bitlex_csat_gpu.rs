//! The prover's circuit solver on the device (decision 0670, step 3): CDCL over a batch of formulas, a SIMD group a
//! formula (`bitlex_csat_gpu.uglykl`), each read where it was built. A formula's gates, fanout and plain clauses are
//! in the compile's formula block (`bitlex_grown`), as the CPU's solver reads them; the kernel binds the block once
//! and finds each formula by its descriptor's offsets. Nothing is uploaded (decision 0353): what the device has of
//! its own is each formula's state (values, trail, watches, learned clauses), its working memory.
//!
//! The answers are the CPU's (`bitlex_csat::Solver`): satisfiable, unsatisfiable, or unknown past the conflicts'
//! budget or the learned clauses' room, sound either way. The search is its own (decisions in VMTF order, a round's
//! implications in lane order), so a formula's conflicts are not the CPU's.
//!
//! Memory, a batch: a descriptor of `DESC` words a formula, in the block; the states, `state_words` each. Launches:
//! a threadgroup of `LANES` (a SIMD group) a formula; each launch runs a formula `STEPS` steps at most, the GPU's time
//! a launch bounded; at most `LAUNCHES` launches, past which a formula still running is unknown.

use crate::bitlex_csat::{Fanout, Gates};
use crate::bitlex_grown::{FormulaBlock, Grown};
use crate::bitlex_sat::Clauses;
use rerail_gpu::device::ComputeQueue;
use rerail_gpu::kernels::{alloc, launch, program, read};

const KERNEL: &str = include_str!("bitlex_csat_gpu.uglykl");

/// Whether the prover's batches solve on the GPU first (`RERAIL_PROVER_GPU=1`).
pub fn enabled() -> bool {
    std::env::var_os("RERAIL_PROVER_GPU").is_some_and(|v| v == "1")
}

/// A descriptor's words (the kernel's `DESC`).
const DESC: usize = 20;
/// A formula's lanes: one SIMD group.
const LANES: u64 = 32;
/// The steps a launch runs a formula at most (a round of propagation, a conflict, a decision).
const STEPS: u32 = 1 << 12;
/// The launches a batch takes at most.
const LAUNCHES: usize = 1 << 12;
/// A formula's room for learned clauses, by its size: their literals (16 words a variable, from 4,096 to 2^18), and
/// a clause for each 16 of those. The kernel reduces them as they fill it (`cs_reduce`).
fn arena(vars: usize) -> u32 {
    (16 * vars).clamp(1 << 12, 1 << 18) as u32
}
fn learned(vars: usize) -> u32 {
    arena(vars) / 16
}
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
    pad: u32,
}
// SAFETY: four u32s, no padding.
unsafe impl rerail_gpu::Pod for Args {}

/// A formula: its gates (the first `base` listed by `fan`, the rest a solve's own, listed by `own`), its plain
/// clauses, its variables (numbered from 1), and its budget of conflicts: with none, it stops at its first decision
/// (propagation at level 0 alone). All in one block.
pub struct Formula<'a> {
    pub vars: usize,
    pub gates: &'a Gates,
    pub base: usize,
    pub fan: &'a Fanout,
    pub own: Option<&'a Fanout>,
    pub clauses: &'a Clauses,
    pub conflicts: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// A model: each variable's value (`[0]` unused).
    Sat(Vec<bool>),
    Unsat,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct Solved {
    pub answer: Answer,
    /// Unknown because the learned clauses' room is full (else the budget, or the launches, ran out).
    pub roomless: bool,
    pub conflicts: u32,
    pub decisions: u32,
    pub rounds: u32,
    /// The clauses it learned and their literals.
    pub learned: u32,
    pub learned_literals: u32,
    pub visits: u32,
    pub items: u32,
    pub reductions: u32,
    /// The device's cycles by part, over 256 (0 where its backend has no clock): watch visits, gate items, analysis,
    /// jumps back, decisions, reductions.
    pub cycles: [u32; 6],
}

/// A formula's state, in words (the kernel's layout).
fn state_words(vars: usize, clauses: usize) -> usize {
    let (n, l, a) = (vars + 1, learned(vars) as usize, arena(vars) as usize);
    HEADER + 9 * n + 2 * n + 4 * (clauses + l) + (l + 1) + a + 2 * (n + 1) + n + l + LANES as usize
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
    if formulas.is_empty() {
        return Ok(Vec::new());
    }
    let shared = rerail_gpu::shared::get().ok_or("the prover's GPU: no device")?;
    let function = program("bitlex_csat_gpu.uglykl", KERNEL)?.function("csat_solve")?;
    let elsewhere = || "the prover's GPU: a formula not in the batch's block".to_string();
    let block: FormulaBlock = formulas[0].gates.placed().ok_or_else(elsewhere)?.0.clone();
    let mut table = Grown::<u32>::in_block(&block);
    let mut states = 0usize;
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
        let state = u32::try_from(states).map_err(|_| "the prover's GPU: a batch's states past 16 GiB".to_string())?;
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
            state,
            learned(f.vars),
            arena(f.vars),
            f.conflicts,
            f.fan.vars() as u32,
            0,
            0,
            0,
        ];
        table.extend_from_slice(&d);
        at.push(states);
        states += state_words(f.vars, f.clauses.len());
    }
    let n = formulas.len();
    let table_at = table.placed().expect("in the block").1;
    let blk = block.block().device_buffer(&shared.device)?;
    let state = alloc::<u32>(states)?;
    let out = alloc::<u32>(n * OUT)?;
    let mut q = ComputeQueue::new(&shared.queue);
    let run = |q: &mut ComputeQueue, init: u32| -> Result<(), String> {
        launch(q, function)
            .value("args", &Args { n: n as u32, steps: STEPS, init, pad: 0 })
            .buffer("blk", &blk)
            .buffer_at("table", &blk, table_at as u64)
            .buffer("state", &state)
            .buffer("out", &out)
            .run(&[n as u64], &[LANES])?;
        q.flush().await_()
    };
    run(&mut q, 1)?;
    let mut got = read::<u32>(&out, n * OUT);
    for _ in 0..LAUNCHES {
        if got.chunks(OUT).all(|o| o[0] != 0) {
            break;
        }
        run(&mut q, 0)?;
        got = read::<u32>(&out, n * OUT);
    }
    if let Some(i) = got.chunks(OUT).position(|o| o[0] == BROKEN) {
        return Err(format!("the device's solver broke on formula {i}: a loop past its bound"));
    }
    let any_sat = got.chunks(OUT).any(|o| o[0] == SAT);
    let words = if any_sat { read::<u32>(&state, states) } else { Vec::new() };
    drop(table);
    Ok(got
        .chunks(OUT)
        .zip(formulas)
        .zip(at)
        .map(|((o, f), s)| Solved {
            answer: match o[0] {
                SAT => Answer::Sat((0..=f.vars).map(|v| words[s + HEADER + v] & 1 != 0).collect()),
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
    use crate::bitlex_csat::Solver;
    use crate::bitlex_sat::{Answer as CpuAnswer, Lit};

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn lit(&mut self, vars: usize) -> Lit {
            let v = self.below(vars as u64) as Lit + 1;
            if self.next() & 1 == 0 { v } else { -v }
        }
    }

    /// Whether a model holds: every gate (as `Gates::cnf` writes its clauses) and every plain clause.
    fn holds(model: &[bool], gates: &Gates, clauses: &Clauses) -> bool {
        let mut all = Clauses::default();
        gates.cnf(&mut all);
        let t = |l: Lit| model[l.unsigned_abs() as usize] == (l > 0);
        all.iter().chain(clauses.iter()).all(|c| c.iter().any(|&l| t(l)))
    }

    /// A random circuit over `vars` inputs, built in `block`: `And`s, `Xor`s and `One`s over earlier variables (a
    /// third of them wide), plain clauses of two to four literals over all; the last `own` gates are a solve's own.
    fn circuit(rng: &mut Rng, block: &FormulaBlock, inputs: usize, gates_n: usize, clauses_n: usize, own: usize) -> (Gates, Clauses, usize, usize) {
        let (mut g, mut c) = (Gates::in_block(block), Clauses::in_block(block));
        let mut vars = inputs;
        for _ in 0..gates_n {
            match rng.below(5) {
                0 | 1 => {
                    let n = if rng.below(3) == 0 { 3 + rng.below(4) as usize } else { 1 + rng.below(2) as usize };
                    let args: Vec<Lit> = (0..n).map(|_| rng.lit(vars)).collect();
                    vars += 1;
                    g.and(vars as Lit, &args);
                }
                2 | 3 => {
                    let (a, b) = (rng.lit(vars), rng.lit(vars));
                    vars += 1;
                    g.xor(vars as Lit, a, b);
                }
                _ => {
                    let n = 1 + rng.below(4) as usize;
                    let args: Vec<Lit> = (0..n).map(|_| rng.lit(vars)).collect();
                    g.one(&args);
                }
            }
        }
        for _ in 0..clauses_n {
            let n = 2 + rng.below(3) as usize;
            let lits: Vec<Lit> = (0..n).map(|_| rng.lit(vars)).collect();
            c.push(&lits);
        }
        let base = g.len().saturating_sub(own);
        (g, c, vars, base)
    }

    /// Random circuits, some with a solve's own gates: the device's answers are the CPU solver's, and each model it
    /// gives holds.
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
        let got = solve(&formulas).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
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
        let got = solve(&formulas).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
        eprintln!("the device: {} formulas, {:.1} ms", formulas.len(), t.elapsed().as_secs_f64() * 1e3);
        let mut s = Solver::default();
        for ((p, (g, vars, base, c)), (x, (fan, _))) in paths.iter().zip(&made).zip(got.iter().zip(&fans)) {
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
