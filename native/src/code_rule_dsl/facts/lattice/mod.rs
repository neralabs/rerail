//! Lattice facts (docs/facts/LATTICE.md): E1, the semiring-round fixpoint on the CPU; E3, its
//! maintenance per edit.
//!
//! - `round`: the operator (L5): CSR (flat or base + replacement rows), `set` and `min_level`; a
//!   sequential kernel and four parallel-within-a-round ones (push over frontier word ranges,
//!   push over edge ranges, owner-computes push, direction-optimizing push/pull); the naive
//!   reference. Depends on nothing else in the crate.
//! - `cols`: columns written once and mapped read-only (L6), with the copy counter; columns a
//!   worktree writes, mapped privately (copy-on-write pages).
//! - `build`: Y2's dead-code product graph (`y2::dc`) as columns, and Y2's own reachability timed
//!   on the same inputs, interleaved with the sequential fixpoint, as the exactness reference.
//! - `bench`: the timed, counted, checked runs (workloads A and B).
//! - `maint` (E3): the layered CSR updated in place; `min_level` and `set` maintained per edit
//!   in the L5 shape. Depends only on `round`.
//! - `e3`: Y3's replay with both lattices maintained after every event, checked against E1's
//!   fixpoint and Y2.
//!
//! `facts-b lattice build --dump F [--edges Y1_FILE] --out DIR [--y2-runs N] [--limit-worktrees N]
//!   [--order st|hash]`
//! `facts-b lattice run --graph DIR --threads T --out FILE [--runs N] [--lattice level|set]
//!   [--kernel seq|par|edge|own|dir] [--fleet within|across]`
//! `facts-b lattice maint --out DIR [--limit N] [--check-every N] [--check-load] [--workers W]`
pub mod bench;
pub mod build;
pub mod cols;
pub mod e3;
pub mod maint;
pub mod round;

use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

pub fn main(args: Vec<String>) {
    match args.get(2).map(String::as_str) {
        Some("build") => build::run(
            &PathBuf::from(arg(&args, "--dump").expect("--dump")),
            arg(&args, "--edges").map(PathBuf::from).as_deref(),
            &PathBuf::from(arg(&args, "--out").expect("--out")),
            arg(&args, "--y2-runs").map_or(5, |x| x.parse().unwrap()),
            arg(&args, "--limit-worktrees").map(|x| x.parse().unwrap()),
            match arg(&args, "--order").as_deref() {
                None | Some("st") => false,
                Some("hash") => true,
                Some(o) => panic!("--order {o}: st | hash"),
            },
        ),
        Some("run") => bench::run(bench::Opts {
            graph: PathBuf::from(arg(&args, "--graph").expect("--graph")),
            out: PathBuf::from(arg(&args, "--out").expect("--out")),
            threads: arg(&args, "--threads").map_or(1, |x| x.parse().unwrap()),
            runs: arg(&args, "--runs").map_or(5, |x| x.parse().unwrap()),
            lattice: match arg(&args, "--lattice").as_deref() {
                None | Some("level") => round::Lattice::MinLevel,
                Some("set") => round::Lattice::Set,
                Some(o) => panic!("--lattice {o}: level | set"),
            },
            kernel: arg(&args, "--kernel").unwrap_or_else(|| "par".into()),
            fleet: arg(&args, "--fleet"),
        }),
        Some("maint") => e3::run(&args),
        _ => {
            eprintln!("usage: facts-b lattice build --dump F [--edges Y1_FILE] --out DIR | run --graph DIR --threads T --out FILE [--runs N] [--lattice level|set] [--kernel seq|par] [--fleet within|across]");
            std::process::exit(2);
        }
    }
}
