//! The prover's solves on the GPU, through UKL (decision 0654, step 5): a batch of formulas, each the clauses one solve
//! of a question would load, propagated on the process's device (`bitlex_prove_gpu.uglykl`). A formula whose
//! propagation meets a conflict is unsatisfiable as the CPU's solver finds it at level 0, with no conflict spent, so
//! the prover's answers are the same; every other formula is solved on the CPU.
//!
//! Memory, a launch: the formulas' literals end to end (4 bytes each), a word a clause (its start), 4 words a
//! formula, and two bit sets of each formula's variables. A launch holds at most `LAUNCH_LITERALS` literals; a batch
//! of more is launched in parts. Mapping: a threadgroup of `LANES` threads a formula, a thread every `LANES`-th clause.

use crate::bitlex_sat::Clauses;
use rerail_gpu::device::ComputeQueue;
use rerail_gpu::kernels::{alloc, buffer_of, launch, program, read};

const KERNEL: &str = include_str!("bitlex_prove_gpu.uglykl");

/// The sweeps over a formula's clauses a launch makes at most: past them, the formula is the CPU's.
const SWEEPS: u32 = 512;

/// The threads of a formula's threadgroup.
const LANES: u64 = 256;

/// The most literals one launch's formulas hold (64 MB of them): a launch's memory and GPU time bounded.
const LAUNCH_LITERALS: usize = 1 << 24;

#[repr(C)]
#[derive(Clone, Copy)]
struct UpArgs {
    n: u32,
    sweeps: u32,
}
// SAFETY: two u32s, no padding.
unsafe impl rerail_gpu::Pod for UpArgs {}

/// Whether the prover's batches propagate their solves on the GPU first (`RERAIL_PROVER_GPU=1`).
pub fn enabled() -> bool {
    std::env::var_os("RERAIL_PROVER_GPU").is_some_and(|v| v == "1")
}

/// For each formula (its variables, numbered from 1, and its clauses): whether unit propagation from no decision
/// meets a conflict, on the process's device. An error where there is no device or the kernel does not compile: a
/// caller that asked for the GPU stops on it (no quiet fallback).
pub fn conflicts(formulas: &[(u32, &Clauses)]) -> Result<Vec<bool>, String> {
    let shared = rerail_gpu::shared::get().ok_or("the prover's GPU: no device")?;
    let function = program("bitlex_prove_gpu.uglykl", KERNEL)?.function("unit_propagate")?;
    let mut q = ComputeQueue::new(&shared.queue);
    let mut out = Vec::with_capacity(formulas.len());
    let mut i = 0;
    while i < formulas.len() {
        let mut j = i;
        let mut literals = 0;
        while j < formulas.len() && (j == i || literals + formulas[j].1.lits().len() <= LAUNCH_LITERALS) {
            literals += formulas[j].1.lits().len();
            j += 1;
        }
        let part = &formulas[i..j];
        let (mut info, mut starts, mut lits) = (Vec::with_capacity(part.len() * 4), Vec::new(), Vec::with_capacity(literals));
        let mut words = 0u32;
        for &(vars, clauses) in part {
            let base = lits.len() as u32;
            info.extend([starts.len() as u32, clauses.len() as u32, words]);
            starts.push(base);
            starts.extend(clauses.ends().iter().take(clauses.len().saturating_sub(1)).map(|&e| base + e));
            if clauses.is_empty() {
                starts.pop();
            }
            lits.extend_from_slice(clauses.lits());
            let w = (vars + 1).div_ceil(32);
            info.push(w);
            words += 2 * w;
        }
        starts.push(lits.len() as u32);
        let n = part.len() as u32;
        let vals = buffer_of(&vec![0u32; words.max(1) as usize])?;
        let done = alloc::<u32>(part.len())?;
        launch(&q, function)
            .value("args", &UpArgs { n, sweeps: SWEEPS })
            .buffer("info", &buffer_of(&info)?)
            .buffer("starts", &buffer_of(&starts)?)
            .buffer("lits", &buffer_of(&lits)?)
            .buffer("vals", &vals)
            .buffer("out", &done)
            .run(&[u64::from(n)], &[LANES])?;
        q.flush().await_()?;
        let got = read::<u32>(&done, part.len());
        if std::env::var_os("RERAIL_PROVING_TRACE").is_some() {
            let n = |k: u32| got.iter().filter(|&&x| x == k).count();
            eprintln!("gpu launch: {} formulas: {} conflicts, {} fixed points, {} past the sweeps", part.len(), n(1), n(0), n(2));
        }
        out.extend(got.into_iter().map(|x| x == 1));
        i = j;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitlex_sat::{Answer, Solver};

    /// Unit propagation from no decision, on the CPU: whether it meets a conflict.
    fn propagates_to_conflict(vars: u32, clauses: &Clauses) -> bool {
        let mut val = vec![0i8; vars as usize + 1];
        loop {
            let mut changed = false;
            for c in clauses.iter() {
                let mut free = None;
                let mut n = 0;
                let mut sat = false;
                for &l in c {
                    let v = val[l.unsigned_abs() as usize];
                    if v == 0 {
                        n += 1;
                        free = Some(l);
                    } else if (v > 0) == (l > 0) {
                        sat = true;
                        break;
                    }
                }
                if sat {
                    continue;
                }
                match (n, free) {
                    (0, _) => return true,
                    (1, Some(l)) => {
                        val[l.unsigned_abs() as usize] = if l > 0 { 1 } else { -1 };
                        changed = true;
                    }
                    _ => {}
                }
            }
            if !changed {
                return false;
            }
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    /// Formulas of chains of implications from a unit, some closed by a clause their chain falsifies, some with
    /// random clauses of three: the GPU's conflicts are the CPU's propagation's, and each is unsatisfiable to the
    /// solver with no conflict spent.
    #[test]
    fn the_gpus_conflicts_are_the_solvers_at_level_0() {
        if rerail_gpu::shared::get().is_none() {
            eprintln!("no device: the prover's GPU test did not run");
            return;
        }
        let mut rng = Rng(0x9e3779b97f4a7c15);
        let mut formulas: Vec<(u32, Clauses)> = Vec::new();
        for f in 0..300u32 {
            let vars = 50 + (rng.next() % 400) as u32;
            let mut c = Clauses::default();
            c.push(&[1]);
            // A chain 1 -> 2 -> ... in a random order of its links.
            let mut links: Vec<u32> = (1..vars).collect();
            for k in (1..links.len()).rev() {
                links.swap(k, (rng.next() % (k as u64 + 1)) as usize);
            }
            for &a in &links {
                c.push(&[-(a as i32), a as i32 + 1]);
            }
            for _ in 0..rng.next() % 200 {
                let l = |r: &mut Rng| ((r.next() % u64::from(vars)) as i32 + 1) * if r.next() & 1 == 0 { 1 } else { -1 };
                let x = [l(&mut rng), l(&mut rng), l(&mut rng)];
                c.push(&x);
            }
            if f % 2 == 0 {
                c.push(&[-(vars as i32)]);
            }
            formulas.push((vars, c));
        }
        let asked: Vec<(u32, &Clauses)> = formulas.iter().map(|(v, c)| (*v, c)).collect();
        let got = conflicts(&asked).unwrap_or_else(|e| panic!("{}: {e}", rerail_gpu::device::backend()));
        let mut conflicted = 0;
        for (i, ((vars, c), &g)) in formulas.iter().zip(&got).enumerate() {
            assert_eq!(g, propagates_to_conflict(*vars, c), "formula {i}: the GPU's propagation and the CPU's differ");
            if g {
                conflicted += 1;
                let mut s = Solver::new(*vars as usize);
                s.add_all(c);
                assert_eq!(s.solve(0), Answer::Unsat, "formula {i}: a conflict the solver does not find at level 0");
                assert_eq!(s.conflicts, 0, "formula {i}");
            }
        }
        assert!(conflicted >= 150, "{conflicted} of 300 met a conflict");
    }
}
