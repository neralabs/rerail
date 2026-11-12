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
    // This wave contains array kernels; SQL programs have separate coverage.
    for l in lanes.iter().filter(|&&l|l!=Lane::Sql) {
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
    vec![rerail_accel::Col::from(k0), rerail_accel::Col::from(k1)].into()
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
            let p = d.out.pairs().expect("pairs");
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

/// The small path on every lane: right sides of at most 2,048 rows sorted in one dispatch, with one to three keys,
/// negative and repeated keys, empty sides, and columns staged behind a gate the host opens after the batch was
/// begun. Every lane's pairs are the reference's.
#[test]
fn small_joins_and_staged_columns_match_the_reference() {
    use rerail_accel::kernel::{JoinBy, Keys};
    let ex = Executor::new();
    let mut rng = Rng(0x5eed);
    let mut col = |n: usize, m: u64| -> Vec<i64> { (0..n).map(|_| rng.below(m) as i64 - (m as i64) / 2).collect() };
    let mut cases: Vec<(Vec<Vec<i64>>, Vec<Vec<i64>>)> = Vec::new();
    for (nl, nr, k, m) in [(1500, 2048, 1, 700), (3000, 1000, 2, 40), (10, 1, 3, 3), (0, 50, 1, 9), (40, 0, 2, 9), (5000, 1999, 3, 6), (6000, 5000, 1, 50), (4000, 9000, 2, 30)] {
        cases.push(((0..k).map(|_| col(nl, m)).collect(), (0..k).map(|_| col(nr, m)).collect()));
    }
    let host = |cols: &[Vec<i64>]| -> Keys { cols.iter().map(|c| rerail_accel::Col::from(c.clone())).collect::<Vec<_>>().into() };
    for lane in ex.lanes() {
        if !matches!(lane, Lane::Cpu | Lane::Gpu) {
            continue;
        }
        // Host columns.
        let launches: Vec<Launch> = cases.iter().map(|(l, r)| Launch::on(Kernel::Join { left: host(l), right: host(r), right_id: None, by: JoinBy::Probe }, lane)).collect();
        let want: Vec<_> = launches.iter().map(|l| cpu::run(&l.kernel)).collect();
        let (done, _) = ex.run(Batch(launches)).unwrap();
        for (i, (d, w)) in done.iter().zip(&want).enumerate() {
            assert_eq!(&d.out, w, "host case {i} on {}", lane.name());
        }
        // Staged: begun before the values are written; the gate opened after.
        let (gate, opener) = rerail_accel::Gate::new();
        let mut writers = Vec::new();
        let mut stage = |cols: &[Vec<i64>]| -> Keys {
            cols.iter()
                .map(|c| {
                    let (col, w) = gate.column(c.len()).unwrap();
                    writers.push((w, c.clone()));
                    col
                })
                .collect::<Vec<_>>()
                .into()
        };
        let launches: Vec<Launch> = cases.iter().map(|(l, r)| Launch::on(Kernel::Join { left: stage(l), right: stage(r), right_id: None, by: JoinBy::Probe }, lane)).collect();
        let running = ex.begin(Batch(launches)).unwrap();
        for (mut w, c) in writers {
            w.values().copy_from_slice(&c);
        }
        opener.open();
        let (done, _) = running.wait().unwrap();
        for (i, (d, w)) in done.iter().zip(&want).enumerate() {
            assert_eq!(&d.out, w, "staged case {i} on {}", lane.name());
        }
    }
    eprintln!("{}", ex.score());
}

/// A warm loop of one small join a batch on the gpu lane, for a profiler (`RERAIL_WARM_LOOP=<batches>`): prints the
/// host CPU per job and the phases.
#[test]
#[ignore = "a measurement: run by name with RERAIL_WARM_LOOP"]
fn warm_small_gpu_jobs() {
    use rerail_accel::kernel::{JoinBy, Keys};
    let n: usize = std::env::var("RERAIL_WARM_LOOP").ok().and_then(|v| v.parse().ok()).unwrap_or(2000);
    let ex = Executor::new();
    if !ex.lanes().contains(&Lane::Gpu) {
        return;
    }
    let mut rng = Rng(7);
    let keys = |rng: &mut Rng, n: usize| -> Keys { vec![rerail_accel::Col::from((0..n).map(|_| rng.below(300) as i64).collect::<Vec<_>>())].into() };
    let (l, r) = (keys(&mut rng, 600), keys(&mut rng, 400));
    // `RERAIL_WARM_STAGED=1`: the columns staged behind a gate, opened after the batch is begun (the engine's way).
    let staged = std::env::var("RERAIL_WARM_STAGED").is_ok_and(|v| v == "1");
    let run = || {
        if !staged {
            ex.run(Batch(vec![Launch::on(Kernel::Join { left: l.clone(), right: r.clone(), right_id: None, by: JoinBy::Probe }, Lane::Gpu)])).unwrap();
            return;
        }
        let (gate, opener) = rerail_accel::Gate::new();
        let (lc, mut lw) = gate.column(l[0].len()).unwrap();
        let (rc, mut rw) = gate.column(r[0].len()).unwrap();
        let k = Kernel::Join { left: vec![lc].into(), right: vec![rc].into(), right_id: None, by: JoinBy::Probe };
        let running = ex.begin(Batch(vec![Launch::on(k, Lane::Gpu)])).unwrap();
        lw.values().copy_from_slice(&l[0]);
        rw.values().copy_from_slice(&r[0]);
        opener.open();
        running.wait().unwrap();
    };
    for _ in 0..50 {
        run();
    }
    let before = ex.score().units[Lane::Gpu.index()].cpu;
    for _ in 0..n {
        run();
    }
    let s = ex.score();
    eprintln!("warm gpu job: {:.1} us host cpu", (s.units[Lane::Gpu.index()].cpu - before) * 1e6 / n as f64);
    eprintln!("{s}");
}

/// A device lane behind its plan: its worker is busy with an earlier batch, so the cores and the waiting caller
/// claim the launches planned on it (decision 0100 §11), and the outputs are the reference's.
#[test]
fn launches_planned_on_a_lane_behind_its_plan_are_claimed_by_the_others() {
    use rerail_accel::exec::{Backend, CpuBackend};
    use rerail_accel::kernel::{JoinBy, Output};
    /// A "device" that runs the reference after `delay`.
    struct Slow(std::time::Duration);
    impl Backend for Slow {
        fn lane(&self) -> Lane {
            Lane::Gpu
        }
        fn can(&self, _: &Kernel) -> bool {
            true
        }
        fn submit(&self, ks: &[&Kernel]) -> Result<Vec<Output>, String> {
            std::thread::sleep(self.0);
            Ok(ks.iter().map(|k| cpu::run(k)).collect())
        }
    }
    let delay = std::time::Duration::from_millis(300);
    let mut ex = Executor::with(vec![Arc::new(CpuBackend::default()), Arc::new(Slow(delay))]);
    // The device priced far cheaper, so the plan puts every launch there.
    for f in [rerail_accel::kernel::Form::Probe] {
        ex.prices.set(f, Lane::Gpu, Some(1e-15));
    }
    ex.prices.latency = [0.0; Lane::COUNT];
    ex.prices.fixed[Lane::Gpu.index()] = rerail_accel::price::Fixed { submission: 0.0, launch: 0.0 };
    ex.prices.host[Lane::Gpu.index()] = rerail_accel::price::Fixed { submission: 0.0, launch: 0.0 };
    let mut rng = Rng(0xc1a1);
    let mut join = |n: usize| {
        let col = |rng: &mut Rng| -> rerail_accel::Col { (0..n).map(|_| rng.below(50) as i64).collect::<Vec<_>>().into() };
        let (l, r) = (col(&mut rng), col(&mut rng));
        Kernel::Join { left: vec![l].into(), right: vec![r].into(), right_id: None, by: JoinBy::Probe }
    };
    // The device's worker takes this one first, pinned, and is busy for `delay`.
    let busy = ex.begin(Batch(vec![Launch::on(join(100), Lane::Gpu)])).unwrap();
    let ks: Vec<Kernel> = (0..12).map(|i| join(200 + i)).collect();
    let want: Vec<Output> = ks.iter().map(cpu::run).collect();
    let t0 = std::time::Instant::now();
    let running = ex.begin_as("claims test", Batch(ks.into_iter().map(Launch::new).collect())).unwrap();
    let (done, _) = running.wait().unwrap();
    let took = t0.elapsed();
    for (i, (d, w)) in done.iter().zip(&want).enumerate() {
        assert_eq!(&d.out, w, "launch {i} on {}", d.lane.name());
    }
    let tag = ex.score().tags["claims test"].clone();
    assert_eq!(tag.launches[Lane::Gpu.index()], 12, "the plan put every launch on the device: {tag:?}");
    assert!(tag.moved > 0 && done.iter().any(|d| d.lane == Lane::Cpu), "the cores took the late device's launches: {tag:?}");
    assert!(took < delay, "the batch did not wait for the busy device: {took:?}");
    busy.wait().unwrap();
    eprintln!("{}", ex.score());
}

/// A "device" that runs the reference: at once, or begun and finished after `delay` (`start`), logging when each
/// submission began and ended.
struct Fake {
    delay: std::time::Duration,
    log: Arc<std::sync::Mutex<Vec<(&'static str, std::time::Instant)>>>,
}

struct FakePending(Vec<rerail_accel::kernel::Output>, std::time::Duration, Arc<std::sync::Mutex<Vec<(&'static str, std::time::Instant)>>>);

impl rerail_accel::exec::Pending for FakePending {
    fn finish(self: Box<Self>) -> Result<Vec<rerail_accel::kernel::Output>, String> {
        std::thread::sleep(self.1);
        self.2.lock().unwrap().push(("end", std::time::Instant::now()));
        Ok(self.0)
    }
}

impl rerail_accel::exec::Backend for Fake {
    fn lane(&self) -> Lane {
        Lane::Gpu
    }
    fn can(&self, _: &Kernel) -> bool {
        true
    }
    fn submit(&self, ks: &[&Kernel]) -> Result<Vec<rerail_accel::kernel::Output>, String> {
        Ok(ks.iter().map(|k| cpu::run(k)).collect())
    }
    fn start<'a>(&'a self, ks: &[&Kernel]) -> Result<Box<dyn rerail_accel::exec::Pending + 'a>, String> {
        self.log.lock().unwrap().push(("start", std::time::Instant::now()));
        Ok(Box::new(FakePending(self.submit(ks)?, self.delay, Arc::clone(&self.log))))
    }
}

fn probe(rng: &mut Rng, n: usize) -> Kernel {
    let col = |rng: &mut Rng| -> rerail_accel::Col { (0..n).map(|_| rng.below(50) as i64).collect::<Vec<_>>().into() };
    let (l, r) = (col(rng), col(rng));
    Kernel::Join { left: vec![l].into(), right: vec![r].into(), right_id: None, by: rerail_accel::kernel::JoinBy::Probe }
}

/// The cores behind their plan, every pool thread busy and the caller not yet waiting: a device with nothing
/// planned in the batch takes its launches (decision 0100 §11), and the outputs are the reference's.
#[test]
fn a_device_with_nothing_planned_takes_the_cores_late_launches() {
    use rerail_accel::exec::CpuBackend;
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut ex = Executor::with(vec![Arc::new(CpuBackend::default()), Arc::new(Fake { delay: std::time::Duration::ZERO, log })]);
    // The device fast, but dear in host CPU, so the plan puts every launch on the cores.
    ex.prices.set(rerail_accel::kernel::Form::Probe, Lane::Gpu, Some(1e-9));
    ex.prices.latency = [0.0; Lane::COUNT];
    ex.prices.fixed[Lane::Gpu.index()] = rerail_accel::price::Fixed { submission: 0.0, launch: 0.0 };
    ex.prices.host[Lane::Gpu.index()] = rerail_accel::price::Fixed { submission: 0.05, launch: 0.05 };
    let hold = std::time::Duration::from_millis(300);
    // Every pool thread busy for `hold`, its work unknown to the plan (estimate 0).
    let busy: Vec<_> = (0..rerail_accel::exec::threads()).map(|_| ex.spawn("hold", 0.0, move || std::thread::sleep(hold))).collect();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut rng = Rng(0xd1);
    let ks: Vec<Kernel> = (0..8).map(|i| probe(&mut rng, 200 + i)).collect();
    let want: Vec<rerail_accel::kernel::Output> = ks.iter().map(cpu::run).collect();
    let running = ex.begin_as("idle device", Batch(ks.into_iter().map(Launch::new).collect())).unwrap();
    // The caller works before it waits, so only the device can reach the launches meanwhile.
    std::thread::sleep(std::time::Duration::from_millis(100));
    let t0 = std::time::Instant::now();
    let (done, _) = running.wait().unwrap();
    for (i, (d, w)) in done.iter().zip(&want).enumerate() {
        assert_eq!(&d.out, w, "launch {i} on {}", d.lane.name());
    }
    let tag = ex.score().tags["idle device"].clone();
    assert!(done.iter().all(|d| d.lane == Lane::Gpu), "the device took every late launch: {tag:?}");
    assert!(t0.elapsed() < std::time::Duration::from_millis(50), "the outputs were ready when the caller waited");
    for b in busy {
        b.wait().unwrap();
    }
}

/// Two batches on a device two submissions deep: the second begins before the first is read.
#[test]
fn a_device_begins_its_next_submission_before_it_reads_the_last() {
    use rerail_accel::exec::CpuBackend;
    let log = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ex = Executor::with(vec![Arc::new(CpuBackend::default()), Arc::new(Fake { delay: std::time::Duration::from_millis(100), log: Arc::clone(&log) })]);
    let mut rng = Rng(0xd2);
    let (a, b) = (probe(&mut rng, 100), probe(&mut rng, 120));
    let want = [cpu::run(&a), cpu::run(&b)];
    let ra = ex.begin(Batch(vec![Launch::on(a, Lane::Gpu)])).unwrap();
    let rb = ex.begin(Batch(vec![Launch::on(b, Lane::Gpu)])).unwrap();
    let (da, _) = ra.wait().unwrap();
    let (db, _) = rb.wait().unwrap();
    assert_eq!((&da[0].out, &db[0].out), (&want[0], &want[1]));
    let log = log.lock().unwrap();
    let kinds: Vec<&str> = log.iter().map(|e| e.0).collect();
    assert_eq!(kinds, ["start", "start", "end", "end"], "the second began while the first was in flight");
}

/// A program of the caller's (`Kernel::Program`): its result says which lane's form ran it.
struct Where {
    units: f64,
    cpu: Option<f64>,
    gpu: Option<f64>,
}

impl rerail_accel::kernel::Program for Where {
    fn name(&self) -> &'static str {
        "a test's program"
    }
    fn units(&self) -> f64 {
        self.units
    }
    fn per_unit(&self, lane: Lane) -> Option<f64> {
        match lane {
            Lane::Cpu => self.cpu,
            Lane::Gpu => self.gpu,
            _ => None,
        }
    }
    fn run(&self, lane: Lane) -> Result<rerail_accel::kernel::ProgramOutput, String> {
        Ok(Arc::new(lane))
    }
}

fn program(units: f64, cpu: Option<f64>, gpu: Option<f64>) -> Kernel {
    Kernel::Program(rerail_accel::kernel::ProgramKernel(Arc::new(Where { units, cpu, gpu })))
}

fn ran_on(out: &rerail_accel::kernel::Output) -> Lane {
    let rerail_accel::kernel::Output::Program(r) = out else { panic!("not a program's result: {out:?}") };
    *r.0.downcast_ref::<Lane>().expect("the test's program returns its lane")
}

/// Programs are launches like the others: placed by their own prices, run by the lane they were placed on, each
/// lane's form returning its result, and scored.
#[test]
fn a_program_is_placed_by_its_own_prices_and_runs_the_lanes_form() {
    let ex = Executor::new();
    let gpu = ex.lanes().contains(&Lane::Gpu);
    // Only a cores' form; only a device's (where there is one); both, the device's far cheaper on a large one.
    let mut ks = vec![program(1e3, Some(1e-9), None)];
    if gpu {
        ks.push(program(1e3, None, Some(1e-9)));
        ks.push(program(1e9, Some(1e-8), Some(1e-12)));
    }
    let (done, _) = ex.run(Batch(ks.into_iter().map(Launch::new).collect())).unwrap();
    for d in &done {
        assert_eq!(ran_on(&d.out), d.lane, "its result is the form of the lane it ran on");
    }
    assert_eq!(done[0].lane, Lane::Cpu);
    if gpu {
        assert_eq!(done[1].lane, Lane::Gpu, "a program with only a device's form runs there");
        assert_eq!(done[2].lane, Lane::Gpu, "and one far cheaper there goes there");
    }
    assert!(format!("{}", ex.score()).contains("program"), "{}", ex.score());
}

#[test]
fn polling_a_batch_preserves_its_outputs_and_scoreboard_completion() {
    use rerail_accel::exec::CpuBackend;
    let executor=Executor::with(vec![Arc::new(CpuBackend::default())]);
    let kernel=Kernel::Histogram {off:vec![0,3].into(),codes:vec![0,1,1].into(),width:2};
    let expected=cpu::run(&kernel);
    let mut running=executor.begin(Batch(vec![Launch::on(kernel.clone(),Lane::Cpu),Launch::on(kernel,Lane::Cpu)])).unwrap();
    let started=std::time::Instant::now();
    while !running.ready() {
        assert!(started.elapsed()<std::time::Duration::from_secs(5));
        if !rerail_accel::exec::on_pool() || !rerail_accel::exec::help_once() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    assert!(running.ready(),"Polling a completed batch must retain every reply");
    let (done,report)=running.wait().unwrap();
    assert_eq!(done.len(),2);
    for item in done {assert_eq!(item.out,expected);}
    assert_eq!(report.launches[Lane::Cpu.index()],2);
    let score=executor.score();let cpu=&score.units[Lane::Cpu.index()];
    assert_eq!((cpu.jobs,cpu.failed,cpu.queued,cpu.running),(2,0,0,0));
}

/// The cost statistics fit a device lane's rate from warm samples of one kernel alone, its fixed costs taken out;
/// cold, failed and mixed jobs, and a table form on the cores, are not fitted.
#[test]
fn learned_rates_come_from_warm_samples_of_one_kernel() {
    use rerail_accel::price::{MIN_SAMPLES, Prices, learn_from};
    use rerail_accel::score::Sample;
    let fixed = Prices::default().fixed[Lane::Gpu.index()];
    let sample = |lane, forms: &str, launches: u32, units: f64, per: f64| Sample {
        lane,
        forms: forms.to_owned(),
        launches,
        units,
        estimate: 0.0,
        host_estimate: 0.0,
        wall: fixed.submission + fixed.launch * f64::from(launches) + per * units,
        cpu: 0.0,
        waited: 0.0,
        phases: String::new(),
        cold: false,
        ok: true,
        load: 1.0,
        threads: 1,
    };
    let mut samples: Vec<Sample> = (0..MIN_SAMPLES).map(|i| sample(Lane::Gpu, "test-learned-span", 1, 1e5 + i as f64, 2e-9)).collect();
    samples.push(Sample { cold: true, ..sample(Lane::Gpu, "test-learned-span", 1, 1e5, 1.0) });
    samples.push(Sample { ok: false, ..sample(Lane::Gpu, "test-learned-span", 1, 1e5, 1.0) });
    samples.extend((0..MIN_SAMPLES).map(|_| sample(Lane::Gpu, "test-learned-span,closure", 2, 1e5, 1.0)));
    samples.extend((0..MIN_SAMPLES).map(|_| sample(Lane::Cpu, "closure", 1, 1e5, 1.0)));
    let fitted = learn_from(&samples);
    assert_eq!(fitted.len(), 1, "{fitted:?}");
    assert_eq!((fitted[0].name.as_str(), fitted[0].lane, fitted[0].samples), ("test-learned-span", Lane::Gpu, MIN_SAMPLES));
    assert!((fitted[0].per_unit - 2e-9).abs() < 1e-12, "{}", fitted[0].per_unit);
}
