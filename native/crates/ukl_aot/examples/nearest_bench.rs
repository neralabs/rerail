//! `nearest_jump`'s three versions timed on one forest: the scalar reference, the hand-vectorized NEON kernel and
//! rerail-ukl-vec's Rust of the UKL kernel at 8 lanes. Each run starts from the same states; the fastest of `REPS`
//! is printed, with the CPU time of all of them.
//!
//! `cargo run -p rerail-ukl-aot --profile profiling --example nearest_bench [NODES SETS SPARSE deep|shallow]`

use rerail_ukl_aot::{Forest, NearestArgs};
use std::time::{Duration, Instant};

const REPS: usize = 7;

fn time(name: &str, f: &Forest, mut run: impl FnMut(&mut [u32]) -> u32) {
    let mut best = Duration::MAX;
    let mut state = f.start.clone();
    for _ in 0..REPS {
        state.copy_from_slice(&f.start);
        let t = Instant::now();
        let most = run(&mut state);
        best = best.min(t.elapsed());
        std::hint::black_box(most);
    }
    assert!(state == f.want, "{name}: wrong answers");
    let per = best.as_secs_f64() * 1e9 / f64::from(f.cells());
    println!("{name:>10}: {:>9.3} ms, {per:.2} ns a cell", best.as_secs_f64() * 1e3);
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let num = |i: usize, d: u32| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    let (nodes, sets, sparse) = (num(0, 1 << 16), num(1, 16), num(2, 8));
    let deep = a.get(3).is_some_and(|s| s == "deep");
    let f = Forest::random(nodes, sets, sparse, deep, 7);
    println!("{nodes} nodes, {sets} sets ({} cells), one node in {sparse} a member, {}", f.cells(), if deep { "deep" } else { "shallow" });
    time("scalar", &f, |s| rerail_ukl_aot::scalar::nearest_jump(s, nodes, f.cells()));
    #[cfg(target_arch = "aarch64")]
    {
        time("hand", &f, |s| unsafe { rerail_ukl_aot::hand::nearest_jump(s.as_mut_ptr(), nodes, f.cells()) });
        let args = NearestArgs { nodes, cells: f.cells(), pad0: 0, pad1: 0 };
        time("ukl w8", &f, |s| unsafe { rerail_ukl_aot::nearest_run(rerail_ukl_aot::nearest_jump_w8::nearest_jump_w8, s, args, 256) });
    }
}
