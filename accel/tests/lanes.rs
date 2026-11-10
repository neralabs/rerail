//! One wave across every lane: the executor places a batch's launches, submits each lane's share once, runs the
//! lanes together, and every output is the reference's.

use rerail_accel::{Batch, Bits, Executor, Kernel, Lane, Launch, cpu};
use std::sync::Arc;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn bits(rng: &mut Rng, rows: usize, width: usize, density: u64) -> Bits {
    let words = width.div_ceil(64);
    let mut data = vec![0u64; rows * words];
    for r in 0..rows {
        for c in 0..width {
            if rng.below(16) < density {
                data[r * words + c / 64] |= 1 << (c % 64);
            }
        }
    }
    Bits::new(rows, words, data)
}

fn parts(rng: &mut Rng, n: usize, max: u64, codes: u64) -> (Arc<[u32]>, Arc<[u16]>) {
    let (mut off, mut col) = (vec![0u32], Vec::new());
    for _ in 0..n {
        for _ in 0..rng.below(max + 1) {
            col.push(rng.below(codes) as u16);
        }
        off.push(col.len() as u32);
    }
    (off.into(), col.into())
}

fn wave(rng: &mut Rng) -> Vec<(Kernel, Lane)> {
    let (loff, left) = parts(rng, 50, 40, 300);
    let (roff, right) = parts(rng, 50, 30, 300);
    let (hoff, codes) = parts(rng, 80, 200, 115);
    let (mut off, mut dst) = (vec![0u32], Vec::new());
    for _ in 0..500 {
        for _ in 0..rng.below(3) {
            dst.push(rng.below(500) as u32);
        }
        off.push(dst.len() as u32);
    }
    // Large enough for the Neural Engine (Core ML keeps smaller products on the CPU), the constant fixed.
    let (a, b) = (bits(rng, 2048, 4096, 2), bits(rng, 256, 4096, 2).fixed());
    let min: Arc<[u32]> = (0..256).map(|j| 30 + (j % 5) as u32).collect();
    vec![
        (Kernel::BitProduct { a: a.clone(), b: b.clone(), min: None }, Lane::Ane),
        (Kernel::BitProduct { a: a.clone(), b: b.clone(), min: Some(min) }, Lane::Sme),
        (Kernel::CountTable { loff: loff.clone(), left: left.clone(), roff: roff.clone(), right: right.clone(), emit: false }, Lane::Gpu),
        (Kernel::CountTable { loff, left, roff, right, emit: true }, Lane::Cpu),
        (Kernel::Histogram { off: hoff, codes, width: 115 }, Lane::Gpu),
        (Kernel::Closure { n: 500, off: off.into(), dst: dst.into() }, Lane::Cpu),
    ]
}

#[test]
fn a_wave_runs_on_every_lane_at_once_and_matches_the_reference() {
    let ex = Executor::new();
    let lanes = ex.lanes();
    let mut rng = Rng(42);
    let ks = wave(&mut rng);
    // Pinned: every present lane gets work in the same batch; a lane this machine lacks is pinned to the CPU.
    let batch = Batch(ks.iter().map(|(k, l)| Launch::on(k.clone(), if lanes.contains(l) { *l } else { Lane::Cpu })).collect());
    let (done, report) = ex.run(batch).unwrap();
    for ((k, _), d) in ks.iter().zip(&done) {
        assert_eq!(d.out, cpu::run(k), "{} on {}", k.form().name(), d.lane.name());
    }
    for l in &lanes {
        assert!(report.launches[l.index()] > 0, "no launch on {}", l.name());
    }
    eprintln!("pinned: {report:?}");
    // Placed by price: the same outputs, wherever they ran.
    let (done, report) = ex.run(Batch(ks.iter().map(|(k, _)| Launch::new(k.clone())).collect())).unwrap();
    for ((k, _), d) in ks.iter().zip(&done) {
        assert_eq!(d.out, cpu::run(k), "{} on {}", k.form().name(), d.lane.name());
    }
    eprintln!("placed: {:?}; {report:?}", done.iter().map(|d| (d.lane.name(), d.out.clone()).0).collect::<Vec<_>>());
}

/// Begun batches do not wait (decision 0100 §3): two batches go out before either is collected, and each still
/// gives the reference's outputs. A lane's worker runs them in the order they were sent.
#[test]
fn begun_batches_run_while_the_caller_works_and_match_the_reference() {
    let ex = Executor::new();
    let lanes = ex.lanes();
    let mut rng = Rng(7);
    let (a, b) = (wave(&mut rng), wave(&mut rng));
    let pin = |ks: &[(Kernel, Lane)]| Batch(ks.iter().map(|(k, l)| Launch::on(k.clone(), if lanes.contains(l) { *l } else { Lane::Cpu })).collect());
    let first = ex.begin(pin(&a)).unwrap();
    let second = ex.begin(pin(&b)).unwrap();
    // The caller's own work meanwhile: the references. Then the second batch is collected before the first.
    let want: Vec<_> = a.iter().chain(&b).map(|(k, _)| cpu::run(k)).collect();
    let (db, _) = second.wait().unwrap();
    let (da, _) = first.wait().unwrap();
    assert_eq!(da.len() + db.len(), want.len());
    for (d, w) in da.iter().chain(&db).zip(&want) {
        assert_eq!(&d.out, w, "on {}", d.lane.name());
    }
}

/// A relation of `n` rows: key 0 a node (a document `d` in the high bits, its node below 2^16), key 1 small.
fn nodes(rng: &mut Rng, n: usize, docs: u64, per_doc: u64, small: u64) -> rerail_accel::kernel::Keys {
    let k0: Vec<i64> = (0..n).map(|_| ((rng.below(docs) as i64) << 16) | rng.below(per_doc) as i64).collect();
    let k1: Vec<i64> = (0..n).map(|_| rng.below(small) as i64 - 2).collect();
    vec![Arc::from(k0), Arc::from(k1)].into()
}

/// The joins of a level on every lane that has them, by sort and probe and by tiles, the right sides kept by id
/// across batches: each lane's pairs are the reference's, and the second batch reads what the first kept.
#[test]
fn joins_keep_their_right_sides_across_batches_and_match_the_reference() {
    use rerail_accel::kernel::JoinBy;
    let ex = Executor::new();
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let rights: Vec<_> = (0..3).map(|i| nodes(&mut rng, 3000 + 1500 * i, 5, 400, 3)).collect();
    // Many pairs a row: past the space a probe expects at first, so its second scatter runs.
    let dense = nodes(&mut rng, 4000, 1, 40, 1);
    for batch in 0..2 {
        let mut launches = Vec::new();
        let mut want = Vec::new();
        for lane in ex.lanes() {
            if !matches!(lane, Lane::Cpu | Lane::Gpu) {
                continue;
            }
            for (i, r) in rights.iter().enumerate() {
                for by in [JoinBy::Probe, JoinBy::Tile] {
                    let left = nodes(&mut rng, 2000 + 700 * i + batch * 300, 5, 400, 3);
                    let k = Kernel::Join { left: left.clone(), right: r.clone(), right_id: Some(i as u64), by };
                    want.push(cpu::run(&k));
                    launches.push(Launch::on(k, lane));
                }
            }
            let left = nodes(&mut rng, 3000, 1, 40, 1);
            let k = Kernel::Join { left, right: dense.clone(), right_id: Some(99), by: JoinBy::Probe };
            want.push(cpu::run(&k));
            launches.push(Launch::on(k, lane));
        }
        let (done, _) = ex.run(Batch(launches)).unwrap();
        for (i, (d, w)) in done.iter().zip(&want).enumerate() {
            let rerail_accel::Output::Pairs(p) = &d.out else { panic!("pairs") };
            assert!(!p.is_empty(), "batch {batch} launch {i}: no pairs");
            assert_eq!(&d.out, w, "batch {batch} launch {i} on {}", d.lane.name());
        }
    }
    let space = ex.space();
    for lane in [Lane::Cpu, Lane::Gpu] {
        if ex.lanes().contains(&lane) {
            let s = space[lane.index()];
            // The CPU keeps one sort an id; the GPU one sort and one grouping (the dense side's sort alone).
            assert!(s.hits >= s.misses, "{}: {s:?}", lane.name());
        }
    }
    ex.clear();
    assert_eq!(ex.space()[Lane::Cpu.index()].entries, 0);
}

/// Jobs on every lane at once: batches begun from several threads, each launch on the cores a job of its own on the
/// pool, each device lane's share one job beside them. Every output is the reference's, and the scoreboard counts
/// every launch: as many jobs on the cores as launches there, more than one of them at once, nothing left queued.
#[test]
fn jobs_run_on_every_lane_at_once_and_the_scoreboard_counts_them() {
    let ex = Executor::new();
    let lanes = ex.lanes();
    let mut rng = Rng(11);
    let waves: Vec<_> = (0..4).map(|_| wave(&mut rng)).collect();
    // Each wave on its pinned lanes, and again on the cores.
    let batches: Vec<Vec<(Kernel, Lane)>> = waves
        .iter()
        .map(|w| w.iter().map(|(k, l)| (k.clone(), if lanes.contains(l) { *l } else { Lane::Cpu })).chain(w.iter().map(|(k, _)| (k.clone(), Lane::Cpu))).collect())
        .collect();
    std::thread::scope(|s| {
        for b in &batches {
            let ex = &ex;
            s.spawn(move || {
                let (done, _) = ex.run(Batch(b.iter().map(|(k, l)| Launch::on(k.clone(), *l)).collect())).unwrap();
                for ((k, l), d) in b.iter().zip(&done) {
                    assert_eq!(d.lane, *l);
                    assert_eq!(d.out, cpu::run(k), "{} on {}", k.form().name(), d.lane.name());
                }
            });
        }
    });
    let score = ex.score();
    eprintln!("{score}");
    let on = |l: Lane| batches.iter().flatten().filter(|(_, x)| *x == l).count() as u64;
    let cpu = &score.units[Lane::Cpu.index()];
    assert_eq!(cpu.jobs, on(Lane::Cpu));
    for l in &lanes {
        let u = &score.units[l.index()];
        assert_eq!(u.launches, on(*l), "{}", l.name());
        assert_eq!((u.queued, u.running, u.failed), (0, 0, 0), "{}", l.name());
        assert!(u.outstanding < 1e-9, "{}: {}", l.name(), u.outstanding);
    }
    if rerail_accel::exec::threads() > 1 {
        assert!(cpu.peak > 1, "the cores ran one job at a time: {cpu:?}");
    }
}

/// Host jobs on the cores' pool: jobs that wait for jobs, more of them than the pool has threads, all finish (a wait
/// on the pool runs the pool's other jobs); a panic is an error; the scoreboard counts each job by its name.
#[test]
fn host_jobs_wait_for_each_other_on_the_pool_and_a_panic_is_an_error() {
    let ex = Arc::new(Executor::with(vec![Arc::new(rerail_accel::exec::CpuBackend::default())]));
    let n = rerail_accel::exec::threads() * 3;
    let parents: Vec<_> = (0..n)
        .map(|i| {
            let inner = Arc::clone(&ex);
            ex.spawn("parent", 0.0, move || {
                let a = inner.spawn("child", 0.0, move || 2 * i);
                let b = inner.spawn("child", 0.0, move || 2 * i + 1);
                a.wait().unwrap() + b.wait().unwrap()
            })
        })
        .collect();
    for (i, p) in parents.into_iter().enumerate() {
        assert_eq!(p.wait().unwrap(), 4 * i + 1);
    }
    let bad = ex.spawn("bad", 0.0, || -> usize { panic!("a host job's panic, on purpose") });
    assert!(bad.wait().unwrap_err().contains("panicked"));
    let s = ex.score();
    let cpu = &s.units[Lane::Cpu.index()];
    assert_eq!((cpu.jobs, cpu.failed), (3 * n as u64 + 1, 1));
    assert_eq!(s.forms[&(Lane::Cpu, "child")].launches, 2 * n as u64);
    assert_eq!(s.forms[&(Lane::Cpu, "parent")].launches, n as u64);
}
