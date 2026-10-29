//! Lattice facts, experiment E1 (docs/facts/LATTICE.md): the semiring-round fixpoint on the CPU.
//!
//! - `round`: the operator (L5): CSR (flat or base + replacement rows), `set` and `min_level`,
//!   sequential and parallel-within-a-round kernels, the naive reference. Depends on nothing else
//!   in the crate.
//! - `cols`: columns written once and mapped read-only (L6), with the copy counter.
//! - `build`: Y2's dead-code product graph (`y2::dc`) as columns, and Y2's own reachability timed
//!   on the same inputs as the exactness reference.
//! - `bench`: the timed, counted, checked runs (workloads A and B).
//!
//! `facts-b lattice build --dump F [--edges Y1_FILE] --out DIR [--y2-runs N] [--limit-worktrees N]`
//! `facts-b lattice run --graph DIR --threads T --out FILE [--runs N] [--lattice level|set]
//!   [--kernel seq|par] [--fleet within|across]`
pub mod bench;
pub mod build;
pub mod cols;
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
        _ => {
            eprintln!("usage: facts-b lattice build --dump F [--edges Y1_FILE] --out DIR | run --graph DIR --threads T --out FILE [--runs N] [--lattice level|set] [--kernel seq|par] [--fleet within|across]");
            std::process::exit(2);
        }
    }
}
