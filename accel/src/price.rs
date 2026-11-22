//! What a launch costs on each lane: time, from the spikes' measurements, and energy, from published power figures.
//!
//! **Time.** Each (form, lane) has a cost per unit of the kernel's size ([`crate::Kernel::units`]), and each lane a
//! fixed cost per submission and per launch within one. They are the lanes' own measurements in a **release** build
//! of this crate (M5 Max, 18 cores, macOS 26.4.1, 2026-11-09, load 48–123 on a shared machine; the measurements are
//! `price::bench`, each the best of several runs). Each price is per unit at the largest workload a form has, where the
//! fixed costs are small next to the work. The relational lab replaces them with prices measured on captured strata
//! ([`Prices::set`]).
//!
//! What a lane's time is: the cores, CPU seconds (user + sys) of the reference kernel on one thread, so
//! core-seconds; SME, a submission's wall time; the GPU, device time; the Neural Engine, a submission's wall time,
//! the host's expansion of the inputs and summing of the outputs included (they run on the lane's thread).
//!
//! | Form | Workload | CPU (core-s) | SME (wall) | GPU (device) | ANE (wall) |
//! |---|---|---|---|---|---|
//! | bit product | W5, 5.5·10^10 | 228 ms (4.1e-12/unit) | 2.0 ms (3.7e-14) | 3.0 ms (5.4e-14) | 13.4 ms (2.4e-13) |
//! | bit product | W1, 4.5·10^9 | 27 ms (6.0e-12) | 1.6 ms (3.7e-13) | 0.40 ms (8.9e-14) | 4.9 ms (1.1e-12) |
//! | count table, counts | W2, 5.8·10^6 rows | 3.05 ms (5.3e-10) | — | 0.58 ms (9.9e-11) | — |
//! | count table, pairs | W2, 6.2·10^8 units | 77.6 ms (1.25e-10) | — | 5.87 ms (9.4e-12) | — |
//! | histogram | W4, 2.2·10^7 codes | 11.3 ms (5.1e-10) | — | 0.38 ms (1.7e-11) | — |
//! | closure | W3, 1.1·10^7 edge-words | 8.5 ms (7.5e-10) | — | 1.97 ms (1.7e-10) | — |
//!
//! W1 (13,088 rows against 444) costs more per unit than W5 (2,048 against 2,048) on every lane but the GPU: SME
//! packs 13,088 rows for a small product, and the Neural Engine expands and sums per row, so its host work grows with
//! the rows and not with the product. The prices take W5's.
//!
//! The debug-build prices these replace (load 35–140) were, per unit: bit product CPU 1.0e-12 (the spike's NEON form,
//! not this crate's reference), SME 8e-14, GPU 9e-14, ANE 2.5e-13 (prediction calls only); count table CPU 4.0e-10,
//! GPU 9.8e-11; pairs CPU 3e-9 (an estimate), GPU 9.4e-12; histogram CPU 2.5e-10, GPU 1.7e-11; closure CPU 1.8e-11
//! (the spike's bit-row DP), GPU 1.8e-10. The GPU's device prices are the same in both builds: its kernels are Metal.
//!
//! Fixed costs, measured with the smallest launch of each form:
//!
//! - the cores: about 2 µs a launch (a job on the executor's pool);
//! - SME: 1 µs a submission, 3 µs a launch;
//! - the GPU: 0.2 ms of wall a command buffer (80–100 µs of it host CPU, 8 µs on the device), 22 µs a launch;
//! - the Neural Engine: 0.1 ms a submission, and 0.25 ms a launch (a tile of 2,048 rows' prediction call).
//!
//! **Energy** is estimated, not measured (the user, 2026-11-09): power per busy lane times its busy time. The
//! figures are published ones for the M4 generation, the nearest to this machine that is reported:
//!
//! - a P-core under load, about 6 W (Notebookcheck's M4 Max review); the CPU lane's time is core-seconds;
//! - the SME unit: no published figure; taken as two P-cores' worth, 12 W, while it streams;
//! - the GPU of an M4 Max: 45 W (Notebookcheck's listed TDP; its game test drew about 113 W for the whole machine);
//! - the Neural Engine: about 2–3 W sustained in community `powermetrics` readings (2.8 W peak on an M4; 4.5 W in
//!   an M5 Pro study); taken as 3 W.
//!
//! **Host CPU** (decision 0100 §9). On a loaded machine the cores are the scarce lane, and a device's latency costs
//! the host nothing while the host works on. So the executor places by [`Prices::choose_host`]: of the lanes that
//! finish by the deadline (the band, or the caller's slack when it is later), the one that costs the host the least
//! CPU. Host CPU per unit and per job is its own table (`host_per_unit`, `host`); a lane's latency is measured by
//! the scoreboard as the run goes, and the cores' wall time is scaled by the machine's load.
//!
//! **Placement by energy** ([`Prices::choose`], kept for the price tests): the lane that finishes first, given what
//! is already queued on each lane, is the reference. Every lane that would finish within the **parity band** of it (`band`, 3× to start; the user,
//! 2026-11-09) is a candidate, and the candidate that spends the least energy is chosen. So near parity, the Neural
//! Engine takes the work: it frees the CPU and the GPU, and draws a fraction of their power.

use crate::kernel::{Form, Kernel};
use crate::lanes::Lane;

/// Watts per lane while busy (see the module's comment for the sources).
pub const WATTS: [f64; Lane::COUNT] = [6.0, 12.0, 45.0, 3.0, 6.0];

/// A lane's fixed costs, in seconds: per submission (shared by a batch's launches on the lane) and per launch.
#[derive(Clone, Copy, Debug)]
pub struct Fixed {
    pub submission: f64,
    pub launch: f64,
}

/// The prices of every (form, lane), and the lanes' fixed costs.
#[derive(Clone, Debug)]
pub struct Prices {
    /// Seconds per unit, by form then lane; `None` where the lane has no kernel for the form.
    per_unit: [[Option<f64>; Lane::COUNT]; FORMS],
    /// Per lane; for the CPU lane, wall seconds (the cores' time is `per_unit` divided by `cores`).
    pub fixed: [Fixed; Lane::COUNT],
    /// The cores the CPU lane spreads a launch over: the executor's pool (`RERAIL_THREADS`).
    pub cores: f64,
    /// The parity band: lanes finishing within `band` × the fastest are candidates.
    pub band: f64,
    /// Host CPU seconds per unit, by form then lane: what a launch costs the host's cores on that lane (the cores'
    /// own work; SME's stream on a core; a device lane's uploads and grouping).
    host_per_unit: [[Option<f64>; Lane::COUNT]; FORMS],
    /// Host CPU per job (a batch's share of the lane: its submission) and per launch in it.
    pub host: [Fixed; Lane::COUNT],
    /// Seconds from a job's submission to its outputs beyond its work (the GPU's commit, scheduling and wake): the
    /// start; the scoreboard's measurements replace it as the run goes.
    pub latency: [f64; Lane::COUNT],
}

/// What placement weighs a launch by on one lane ([`Prices::choose_host`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct Cost {
    /// When it would finish, counting what is queued, the lane's latency, and the load on the cores.
    pub finish: f64,
    /// Its wall seconds of work on the lane.
    pub wall: f64,
    /// The host CPU seconds it costs.
    pub host: f64,
}

/// What a lane's choice is weighed against: the lanes' queued seconds and measured latency, which lanes this batch
/// already pays a job on, how loaded the cores are, and how long the caller works before it waits.
#[derive(Clone, Copy, Debug, Default)]
pub struct Load {
    pub queued: [f64; Lane::COUNT],
    pub latency: [f64; Lane::COUNT],
    pub open: [bool; Lane::COUNT],
    /// The cores' slowdown: the machine's load over its cores, at least 1.
    pub cores: f64,
    /// Seconds the caller works before it waits for the batch (measured per kind of batch).
    pub slack: f64,
    /// Per lane, the measured host CPU over the price's estimate (1 until measured).
    pub host_scale: [f64; Lane::COUNT],
}

fn form_index(f: Form) -> usize {
    f as usize
}

/// The forms the table prices, and a program's row (always empty: a program prices itself).
const FORMS: usize = Form::ALL.len() + 1;

impl Default for Prices {
    fn default() -> Self {
        let mut per_unit = [[None; Lane::COUNT]; FORMS];
        let mut set = |f: Form, l: Lane, s: f64| per_unit[form_index(f)][l.index()] = Some(s);
        // Release builds, per unit at the largest workload of each form (see the module's comment).
        set(Form::BitProduct, Lane::Cpu, 4.1e-12);
        set(Form::BitProduct, Lane::Sme, 3.7e-14);
        set(Form::BitProduct, Lane::Gpu, 5.4e-14);
        set(Form::BitProduct, Lane::Ane, 2.4e-13);
        set(Form::CountTable, Lane::Cpu, 5.3e-10);
        set(Form::CountTable, Lane::Gpu, 9.9e-11);
        set(Form::CountPairs, Lane::Cpu, 1.25e-10);
        set(Form::CountPairs, Lane::Gpu, 9.4e-12);
        set(Form::Histogram, Lane::Cpu, 5.1e-10);
        set(Form::Histogram, Lane::Gpu, 1.7e-11);
        set(Form::Closure, Lane::Cpu, 7.5e-10);
        set(Form::Closure, Lane::Gpu, 1.7e-10);
        // Joins, per row of both sides (the relational lab's replay of the Python pack's 232 joins, and the
        // sort's and probe's host costs, 2026-11-09; host CPU and device together). The cores: a radix sort and a
        // search, 5.7 ns a row of one core. The GPU's probe: the sort's 1.5 ns a row and the probe's 1.2 ns, about
        // 1.4 ns a row of both. Its tiles: the host's grouping and packing, 5 ns a row. The pairs (3 ns each on the
        // GPU) are not known ahead, so not priced.
        set(Form::Probe, Lane::Cpu, 5.7e-9);
        set(Form::Probe, Lane::Gpu, 1.4e-9);
        set(Form::Tile, Lane::Cpu, 5.7e-9);
        set(Form::Tile, Lane::Gpu, 5e-9);
        // Host CPU per unit. The cores and SME: their work is on the host's cores. The GPU: copying a host
        // column into a shared buffer (about 1 ns a row; a staged column is not copied), a tile side's grouping
        // (5 ns a row, measured with the join prices). The Neural Engine: its host expansion and sums are most of
        // its submission (the price's own comment), so its time counts as host time.
        let mut host_per_unit = [[None; Lane::COUNT]; FORMS];
        for f in Form::ALL {
            let i = form_index(f);
            host_per_unit[i][Lane::Cpu.index()] = per_unit[i][Lane::Cpu.index()];
            host_per_unit[i][Lane::Sme.index()] = per_unit[i][Lane::Sme.index()];
            host_per_unit[i][Lane::Ane.index()] = per_unit[i][Lane::Ane.index()];
            host_per_unit[i][Lane::Gpu.index()] = per_unit[i][Lane::Gpu.index()].map(|_| match f {
                Form::Tile => 5e-9,
                Form::BitProduct => 1e-15,
                _ => 1e-9,
            });
        }
        Prices {
            host_per_unit,
            // Host CPU a job and a launch (the scoreboard, 2026-11-09): the cores' task; SME's handoff; the GPU's
            // command buffer, encoder and commit, and each join's encoding and reads; the Neural Engine's surfaces.
            host: [
                Fixed { submission: 2e-6, launch: 0.0 },
                Fixed { submission: 1e-6, launch: 3e-6 },
                Fixed { submission: 40e-6, launch: 10e-6 },
                Fixed { submission: 0.1e-3, launch: 0.25e-3 },
                Fixed { submission: 0.0, launch: 2e-6 },
            ],
            latency: [0.0, 0.0, 0.2e-3, 0.1e-3, 0.0],
            per_unit,
            fixed: [
                // A launch's task on the rayon pool.
                Fixed { submission: 0.0, launch: 2e-6 },
                // Entering streaming mode, and a worker's handoff.
                Fixed { submission: 1e-6, launch: 3e-6 },
                // A command buffer's submit and wait, and a launch's encoding.
                Fixed { submission: 0.2e-3, launch: 22e-6 },
                // The surfaces and the grouping, and a tile's prediction call.
                Fixed { submission: 0.1e-3, launch: 0.25e-3 },
                Fixed { submission: 0.0, launch: 2e-6 },
            ],
            cores: crate::exec::threads() as f64,
            band: 3.0,
        }
    }
}

/// Rates learned from the cost statistics (decision 0100 §14): seconds a unit, by kernel name ([`Kernel::name`]) and
/// lane, with the samples each was fitted from. They take the place of the table's and a program's own.
/// The machine's learned rates (decision 0442: its price model's), their version (0 before any; else a number no other
/// machine's rates have, `VERSIONS`, so a thread's copy is never another machine's), and whether its stored statistics
/// are in place ([`first_price`]).
#[derive(Default)]
struct Rates {
    learned: std::sync::RwLock<Vec<Learned>>,
    version: std::sync::atomic::AtomicU64,
    priced: std::sync::atomic::AtomicBool,
}
static VERSIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn rates() -> std::sync::Arc<Rates> {
    rerail_context::machine::<Rates>()
}

impl Rates {
    fn changed(&self) {
        self.version.store(VERSIONS.fetch_add(1, std::sync::atomic::Ordering::Relaxed), std::sync::atomic::Ordering::Release);
    }
}

/// A rate learned from samples ([`learn_from`]).
#[derive(Clone, Debug)]
pub struct Learned {
    pub name: String,
    pub lane: Lane,
    /// Seconds a launch and a unit: a robust line through the samples ([`line`]), past the lane's own fixed costs.
    pub per_launch: f64,
    pub per_unit: f64,
    /// Host CPU seconds a launch and a unit (a device's encoding, sizing and reading back), the same way.
    pub host_per_launch: f64,
    pub host_per_unit: f64,
    pub samples: usize,
    /// The most units a sample had, and whether the samples' units span [`SPREAD`] times (else the line's slope is
    /// noise: [`Learned::at`]).
    pub largest: f64,
    pub spread: bool,
    /// Fitted from this run's own samples ([`observe`]), not the stored statistics.
    pub this_run: bool,
}

/// How far the samples' units must spread (largest over smallest) for a line's slope to be fitted from them, and how
/// far past the largest sample a line prices. A line of small launches says nothing of large ones:
/// - the cores' matcher waves at s1000 ran six small launches first (6.7e4–8.3e4 units), whose line's slope was about
///   nothing: a wave 80 times larger was priced at 6.5 ms and took 200;
/// - the GPU's first four (7e4–1.3e5) gave a slope of 2.7e-8 s a unit against its 6e-9 on large waves, and 14 of 18
///   waves went to the cores (3.2 s of wall against 0.6 on the GPU; 2026-11-17).
pub const SPREAD: f64 = 2.0;

impl Learned {
    /// The busy wall and host CPU of a launch of `units`, given the priors' per-unit rates (`rate` as [`learned`]
    /// returns it). Within the samples' reach, the line. Past it, or where the samples do not spread, a line from the
    /// fitted line's value at the largest sample, whose slope is the prior's or that point's own rate a unit, the
    /// less: what was measured holds at the samples' size, and past it a price grows no faster than either says.
    fn at(rate: &[f64; 6], units: f64, prior: f64, host_prior: f64) -> (f64, f64) {
        let [per_launch, per_unit, host_per_launch, host_per_unit, largest, spread] = *rate;
        let line = (per_launch + per_unit * units, host_per_launch + host_per_unit * units);
        if (spread != 0.0 && units <= SPREAD * largest) || largest <= 0.0 {
            return line;
        }
        let from = |a: f64, b: f64, prior: f64| {
            let at = a + b * largest;
            let slope = prior.min(at / largest);
            (at - slope * largest).max(0.0) + slope * units
        };
        (from(per_launch, per_unit, prior), from(host_per_launch, host_per_unit, host_prior))
    }
}

/// The fewest warm samples a rate is fitted from.
pub const MIN_SAMPLES: usize = 8;

// A price reads its thread's copy of the rates (`LEARNED_COPY`), taken again only when their version moves, so pricing
// (on every thread, for every launch) touches no shared lock.
/// The learned rates' generation and, per kernel name and lane, its line's six coefficients.
type LearnedCopy = (u64, Vec<(String, Lane, [f64; 6])>);
thread_local! {
    static LEARNED_COPY: std::cell::RefCell<LearnedCopy> = const { std::cell::RefCell::new((0, Vec::new())) };
}

/// What a price waits for first: the stored statistics, which the engine reads once a run has work to price
/// ([`before_first_price`]). `true` once they are in place (or there are none): later prices skip it.
static FIRST_PRICE: std::sync::OnceLock<fn() -> bool> = std::sync::OnceLock::new();

/// Sets what the first price waits for (the engine's `costs::ready`): a process that prices nothing reads nothing.
pub fn before_first_price(ready: fn() -> bool) {
    let _ = FIRST_PRICE.set(ready);
}

/// Waits, before the first price, for what [`before_first_price`] names.
pub fn first_price() {
    let rates = rates();
    if rates.priced.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    if FIRST_PRICE.get().is_none_or(|ready| ready()) {
        rates.priced.store(true, std::sync::atomic::Ordering::Release);
    }
}

/// The learned seconds of `name` on `lane`: a launch's and a unit's wall, then a launch's and a unit's host CPU; then
/// the largest sample's units, and 1 when the samples spread ([`Learned::at`]).
fn learned(name: &str, lane: Lane) -> Option<[f64; 6]> {
    first_price();
    let rates = rates();
    let version = rates.version.load(std::sync::atomic::Ordering::Acquire);
    if version == 0 {
        return None;
    }
    LEARNED_COPY.with_borrow_mut(|(seen, copy)| {
        if *seen != version {
            *copy = rates
                .learned
                .read()
                .map(|l| {
                    l.iter()
                        .map(|x| (x.name.clone(), x.lane, [x.per_launch, x.per_unit, x.host_per_launch, x.host_per_unit, x.largest, f64::from(u8::from(x.spread))]))
                        .collect()
                })
                .unwrap_or_default();
            *seen = version;
        }
        copy.iter().find(|x| x.1 == lane && x.0 == name).map(|x| x.2)
    })
}

/// Whether a launch named `name` on `lane` has its host CPU priced from a learned line: a program's, once fitted
/// (a table form's host CPU stays the table's). Such a price is not scaled by the lane's measured host ratio, and
/// does not feed it: the ratio corrects priors, and a learned line scaled by it was counted twice (a GPU program's
/// prior host CPU of nothing set the ratio to its bound, 50, and the learned line then priced a wave's host at 0.8 s).
pub fn host_learned(name: &str, lane: Lane) -> bool {
    Form::ALL.iter().all(|f| f.name() != name) && learned(name, lane).is_some()
}

/// One finished job as a price learns from it: its lane, its launches' names (comma-joined), how many launches and
/// units, its wall and host CPU, the cores' slowdown when it ended, and whether it was cold and succeeded
/// ([`crate::score::Sample::measured`], [`observe`]).
#[derive(Clone, Copy, Debug)]
pub struct Measured<'a> {
    pub lane: Lane,
    pub forms: &'a str,
    pub launches: u32,
    pub units: f64,
    pub wall: f64,
    pub cpu: f64,
    pub load: f64,
    pub cold: bool,
    pub ok: bool,
}

/// A sample as a fit reads it: its kernel name, and per launch its units, busy wall (the lane's fixed costs taken
/// out) and host CPU; `None` where it fits no rate: cold, failed, no units, more than one kernel name, or a table form
/// on the cores (it splits over the threads, so its wall is not one unit's cost).
///
/// A pool lane's wall is divided by the cores' slowdown when it ran (`load`, at least 1): its price multiplies the
/// rate by the slowdown when it prices (`Prices::cost`), so a rate fitted from loaded walls counted the load twice (the
/// cores' matcher wave at 7.8e-8 s a unit under a load of 80, priced at 4 times that again).
fn point_of(m: &Measured) -> Option<(String, [f64; 3])> {
    let Measured { lane, forms, launches, units, wall, cpu, load, cold, ok } = *m;
    let name = forms.split(',').next().unwrap_or("");
    if cold || !ok || units <= 0.0 || launches == 0 || forms.split(',').any(|f| f != name) {
        return None;
    }
    let program = Form::ALL.iter().all(|f| f.name() != name);
    if lane == Lane::Cpu && !program {
        return None;
    }
    let fixed = Prices::default().fixed[lane.index()];
    let busy = wall - fixed.submission - fixed.launch * f64::from(launches);
    let busy = if matches!(lane, Lane::Cpu | Lane::Sme | Lane::Sql) { busy / load.max(1.0) } else { busy };
    let n = f64::from(launches);
    (busy > 0.0).then(|| (name.to_owned(), [units / n, busy / n, cpu.max(0.0) / n]))
}

/// A robust line `y = a + b x` through `points` (Theil–Sen: `b` the median of the pairs' slopes, `a` the median of
/// what is left), both at least 0. A launch of a program has a fixed part (sizing, buffers, pipelines) and a part
/// that grows with its units: a rate a unit alone, from the median of `y / x`, priced the GPU's matcher wave from its
/// small launches (1e5 units, 10 ms) at 9 times its large ones' rate (s1000, 2026-11-17). Without spread in `x`, the
/// line is through the origin.
pub fn line(points: &[(f64, f64)]) -> (f64, f64) {
    // The middle value by selection, not a sort: the pairs' slopes are n² / 2 of them.
    let median = |mut v: Vec<f64>| -> Option<f64> {
        if v.is_empty() {
            return None;
        }
        let mid = v.len() / 2;
        Some(*v.select_nth_unstable_by(mid, f64::total_cmp).1)
    };
    let mut slopes = Vec::with_capacity(points.len() * points.len() / 2);
    for (i, &(x0, y0)) in points.iter().enumerate() {
        for &(x1, y1) in &points[i + 1..] {
            // Pairs whose units differ by under a tenth say nothing of the slope.
            if (x1 - x0).abs() > 0.1 * x0.max(x1) {
                slopes.push((y1 - y0) / (x1 - x0));
            }
        }
    }
    let Some(b) = median(slopes) else {
        return (0.0, median(points.iter().map(|&(x, y)| y / x).collect()).unwrap_or(0.0).max(0.0));
    };
    let b = b.max(0.0);
    let a = median(points.iter().map(|&(x, y)| y - b * x).collect()).unwrap_or(0.0).max(0.0);
    (a, b)
}

/// The rate fitted from points `[units, busy wall, host CPU]` a launch.
fn fitted(name: String, lane: Lane, points: &[[f64; 3]], this_run: bool) -> Learned {
    let (per_launch, per_unit) = line(&points.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    let (host_per_launch, host_per_unit) = line(&points.iter().map(|p| (p[0], p[2])).collect::<Vec<_>>());
    let (lo, hi) = points.iter().fold((f64::INFINITY, 0.0f64), |(lo, hi), p| (lo.min(p[0]), hi.max(p[0])));
    Learned { name, lane, per_launch, per_unit, host_per_launch, host_per_unit, samples: points.len(), largest: hi, spread: hi >= SPREAD * lo, this_run }
}

/// This run's warm samples' points, by (kernel name, lane): the latest [`RUN_HELD`] of each (the run's own, decision
/// 0442).
#[derive(Default)]
struct RunPoints(std::sync::Mutex<std::collections::BTreeMap<(String, usize), PointsHeld>>);
/// One kernel name's latest points on one lane, oldest first.
type PointsHeld = std::collections::VecDeque<[f64; 3]>;
/// The most recent samples a rate of this run is fitted from.
const RUN_HELD: usize = 64;
/// The fewest of this run's own samples a rate is fitted from: fewer than the stored statistics' ([`MIN_SAMPLES`]),
/// since a run has few launches of a program (s1000: 18 matcher waves) and they are what the machine does now.
pub const RUN_MIN_SAMPLES: usize = 4;

/// A finished job, priced from as the run goes (the scoreboard's `finish`): once a kernel name on a lane has
/// [`RUN_MIN_SAMPLES`] warm samples of this run, the line through its latest replaces its prior and the stored
/// statistics' rate. A run then prices its later launches from what its earlier ones measured, not from a constant
/// (a cold run with no stored statistics priced the GPU's matcher wave at 2.5e-8 s a unit, 1.65 times what it
/// measured, and its host CPU at nothing). Refitted at each sample up to 16, then at every 8th.
pub fn observe(m: Measured) {
    let lane = m.lane;
    let Some((name, point)) = point_of(&m) else { return };
    let fit = {
        let points = rerail_context::local::<RunPoints>();
        let Ok(mut run) = points.0.lock() else { return };
        let held = run.entry((name.clone(), lane.index())).or_default();
        if held.len() == RUN_HELD {
            held.pop_front();
        }
        held.push_back(point);
        let n = held.len();
        if n < RUN_MIN_SAMPLES || (n > 16 && n % 8 != 0 && n != RUN_HELD) {
            return;
        }
        fitted(name, lane, &held.iter().copied().collect::<Vec<_>>(), true)
    };
    let rates = rates();
    if let Ok(mut l) = rates.learned.write() {
        l.retain(|x| !(x.name == fit.name && x.lane == fit.lane));
        l.push(fit);
        rates.changed();
    }
}

/// The rates learned so far.
pub fn learned_rates() -> Vec<Learned> {
    rates().learned.read().map(|l| l.clone()).unwrap_or_default()
}

/// Fits per-unit rates from `samples` and puts them in place of the priors; the rates fitted.
///
/// Only what one sample measures directly is fitted: a device lane's wall (its submission's fixed costs taken out)
/// and a program's on the cores (one job on one thread). A table form on the cores splits over threads, so its
/// sample's wall is not one unit's cost; it keeps its prior. Each rate is the median of its warm, successful samples
/// of one kernel name alone, with at least [`MIN_SAMPLES`] of them.
pub fn learn_from(samples: &[crate::score::Sample]) -> Vec<Learned> {
    let fitted = fit_rates(samples);
    let rates = rates();
    if let Ok(mut l) = rates.learned.write() {
        for f in &fitted {
            // A rate of this run's own samples stays: it is what the machine does now.
            if l.iter().any(|x| x.this_run && x.name == f.name && x.lane == f.lane) {
                continue;
            }
            l.retain(|x| !(x.name == f.name && x.lane == f.lane));
            l.push(f.clone());
        }
        if !fitted.is_empty() {
            rates.changed();
        }
    }
    fitted
}

/// The most points a stored rate is fitted from: its kernel's newest. The line through them is Theil–Sen's, over every
/// pair, so the fit's cost grows with their square: over a store's 2,000 samples of a kernel (`READ_PER_FORM`) it took
/// 1.3 s of a process's start, and the first launch that priced a kernel waited for it (2026-11-18).
pub const FIT_HELD: usize = 256;

/// [`learn_from`]'s rates, not put in place: from each kernel's newest [`FIT_HELD`] points, `samples` being newest
/// first (as the store reads them).
pub fn fit_rates(samples: &[crate::score::Sample]) -> Vec<Learned> {
    let mut by: std::collections::BTreeMap<(String, usize), Vec<[f64; 3]>> = Default::default();
    for s in samples {
        if let Some((name, point)) = point_of(&s.measured()) {
            let held = by.entry((name, s.lane.index())).or_default();
            if held.len() < FIT_HELD {
                held.push(point);
            }
        }
    }
    by.into_iter().filter(|(_, v)| v.len() >= MIN_SAMPLES).map(|((name, lane), v)| fitted(name, Lane::ALL[lane], &v, false)).collect()
}

impl Prices {
    /// Replaces the price of `form` on `lane` (seconds per unit; `None`: no kernel).
    pub fn set(&mut self, form: Form, lane: Lane, per_unit: Option<f64>) {
        self.per_unit[form_index(form)][lane.index()] = per_unit;
    }

    pub fn per_unit(&self, form: Form, lane: Lane) -> Option<f64> {
        self.per_unit[form_index(form)][lane.index()]
    }

    /// `k`'s wall time on `lane`, without the submission's cost; `None` when the lane has no kernel for it.
    pub fn wall(&self, k: &Kernel, lane: Lane) -> Option<f64> {
        // A lane with no form of it stays without one; a learned rate replaces its prior.
        let prior = match k {
            Kernel::Program(p) => p.0.per_unit(lane)?,
            _ => self.per_unit(k.form(), lane)?,
        };
        let busy = match learned(k.name(), lane) {
            Some(rate) => Learned::at(&rate, k.units(), prior, 0.0).0,
            None => prior * k.units(),
        };
        // A table form splits its work over the cores; a program's cores' form is one job, its price one thread's.
        let busy = if lane == Lane::Cpu && !matches!(k, Kernel::Program(_)) { busy / self.cores } else { busy };
        Some(self.fixed[lane.index()].launch + busy)
    }

    /// The energy, in joules, of `wall` seconds on `lane` (the CPU lane busy on `cores`).
    pub fn joules(&self, lane: Lane, wall: f64) -> f64 {
        let busy = if lane == Lane::Cpu { wall * self.cores } else { wall };
        WATTS[lane.index()] * busy
    }

    /// Chooses a lane for `k` among `able` lanes, given each lane's `queued` seconds: the least energy among the
    /// lanes that finish within the band of the first to finish. A lane with nothing queued yet pays its submission.
    pub fn choose(&self, k: &Kernel, able: &[Lane], queued: &[f64; Lane::COUNT]) -> Option<(Lane, f64)> {
        self.choose_open(k, able, queued, &std::array::from_fn(|i| queued[i] > 0.0))
    }

    /// `k`'s host CPU seconds on `lane`, its job's share counted when the lane is not `open` for this batch yet.
    pub fn host(&self, k: &Kernel, lane: Lane, open: bool) -> Option<f64> {
        // A program's host CPU is learned with its wall where it has samples (its prior on a device is a guess: the
        // encoding and reading back of a program the price cannot see).
        let work = match k {
            Kernel::Program(p) => {
                let prior = p.0.host_per_unit(lane)?;
                learned(k.name(), lane).map_or(prior * k.units(), |rate| Learned::at(&rate, k.units(), 0.0, prior).1)
            }
            _ => self.host_per_unit[form_index(k.form())][lane.index()]? * k.units(),
        };
        let fixed = self.host[lane.index()];
        Some(work + fixed.launch + if open { 0.0 } else { fixed.submission })
    }

    /// `k`'s cost on `lane` under `load`: the learned model's when it predicted one ([`crate::cost_model`]), else
    /// the rates'. The model predicts a one-launch job under this load: its wall less the lane's submission is the
    /// work (at least a launch's), and its CPU is the host's, less the job's share when the lane is open.
    pub fn cost_predicted(&self, k: &Kernel, lane: Lane, load: &Load, predicted: Option<crate::cost_model::Predicted>) -> Option<Cost> {
        let Some(p) = predicted else { return self.cost(k, lane, load) };
        // The lane must have a kernel for it, as the rates say.
        self.wall(k, lane)?;
        let i = lane.index();
        let wall = (p.wall - self.fixed[i].submission).max(self.fixed[i].launch);
        let host = if load.open[i] { (p.cpu - self.host[i].submission).max(self.host[i].launch) } else { p.cpu };
        let finish = load.queued[i] + if load.open[i] { 0.0 } else { load.latency[i] } + wall;
        Some(Cost { finish, wall, host })
    }

    /// [`Prices::choose_host`], each lane's cost the model's where it predicted one (`predicted`, by lane).
    pub fn choose_host_predicted(&self, k: &Kernel, able: &[Lane], load: &Load, predicted: &[Option<crate::cost_model::Predicted>; Lane::COUNT]) -> Option<(Lane, Cost, Cost)> {
        let costs: Vec<(Lane, Cost)> = able.iter().filter_map(|&l| Some((l, self.cost_predicted(k, l, load, predicted[l.index()])?))).collect();
        self.choose_among(&costs, load)
    }

    /// `k`'s cost on `lane` under `load`.
    pub fn cost(&self, k: &Kernel, lane: Lane, load: &Load) -> Option<Cost> {
        let i = lane.index();
        let slow = if matches!(lane, Lane::Cpu | Lane::Sme | Lane::Sql) { load.cores.max(1.0) } else { 1.0 };
        let wall = self.wall(k, lane)? * slow;
        let queued = if lane.is_pool() { load.queued[Lane::Cpu.index()] + load.queued[Lane::Sql.index()] } else { load.queued[i] };
        let finish = queued + if load.open[i] { 0.0 } else { load.latency[i] } + wall;
        let scale = if load.host_scale[i] > 0.0 && !host_learned(k.name(), lane) { load.host_scale[i] } else { 1.0 };
        Some(Cost { finish, wall, host: self.host(k, lane, load.open[i])? * scale })
    }

    /// Chooses a lane for `k` among `able` by host CPU (decision 0100 §9). The lanes that finish by the deadline are
    /// candidates: the later of the band times the first to finish, and the caller's slack (it works that long
    /// before it waits, so a lane done by then costs it no wall time). Among them, the least host CPU wins, then the
    /// least energy. The chosen lane's cost, and the fastest's.
    pub fn choose_host(&self, k: &Kernel, able: &[Lane], load: &Load) -> Option<(Lane, Cost, Cost)> {
        let costs: Vec<(Lane, Cost)> = able.iter().filter_map(|&l| Some((l, self.cost(k, l, load)?))).collect();
        self.choose_among(&costs, load)
    }

    /// The choice among lanes' costs ([`Prices::choose_host`]).
    fn choose_among(&self, costs: &[(Lane, Cost)], load: &Load) -> Option<(Lane, Cost, Cost)> {
        let fastest = costs.iter().min_by(|a, b| a.1.finish.total_cmp(&b.1.finish))?.1;
        let deadline = (fastest.finish * self.band).max(load.slack);
        let (lane, cost) = costs
            .iter()
            .filter(|(_, c)| c.finish <= deadline)
            .min_by(|a, b| a.1.host.total_cmp(&b.1.host).then(self.joules(a.0, a.1.wall).total_cmp(&self.joules(b.0, b.1.wall))).then(a.1.finish.total_cmp(&b.1.finish)))
            .copied()?;
        Some((lane, cost, fastest))
    }

    /// [`Prices::choose`], where `open` says which lanes this batch already pays a submission on: a lane not open
    /// pays its submission, whatever other batches have queued on it.
    pub fn choose_open(&self, k: &Kernel, able: &[Lane], queued: &[f64; Lane::COUNT], open: &[bool; Lane::COUNT]) -> Option<(Lane, f64)> {
        let finish = |l: Lane| -> Option<f64> {
            let sub = if open[l.index()] { 0.0 } else { self.fixed[l.index()].submission };
            Some(queued[l.index()] + sub + self.wall(k, l)?)
        };
        let best = able.iter().filter_map(|&l| finish(l)).fold(f64::INFINITY, f64::min);
        if !best.is_finite() {
            return None;
        }
        able.iter()
            .filter_map(|&l| Some((l, finish(l)?, self.wall(k, l)?)))
            .filter(|&(_, f, _)| f <= best * self.band)
            .min_by(|a, b| self.joules(a.0, a.2).total_cmp(&self.joules(b.0, b.2)).then(a.1.total_cmp(&b.1)))
            .map(|(l, _, w)| (l, w))
    }
}

#[cfg(test)]
#[path = "price_bench.rs"]
mod bench;

#[cfg(test)]
mod host_tests {
    use super::*;
    use crate::col::Col;
    use crate::kernel::JoinBy;

    fn join(n: usize) -> Kernel {
        let keys = |n: usize| -> crate::kernel::Keys { vec![Col::from((0..n as i64).collect::<rerail_blocks::vector::Vector<_>>())].into() };
        Kernel::Join { left: keys(n), right: keys(n), right_id: None, by: JoinBy::Probe }
    }

    /// Within the caller's slack the lane of least host CPU wins, though it finishes later; with no slack, the band
    /// decides, and a lane far slower is not a candidate. A loaded machine stretches the cores' wall time.
    #[test]
    fn the_least_host_cpu_wins_within_the_slack() {
        let p = Prices::default();
        let k = join(100_000);
        let able = [Lane::Cpu, Lane::Gpu];
        let (cpu, gpu) = (p.cost(&k, Lane::Cpu, &Load::default()).unwrap(), p.cost(&k, Lane::Gpu, &Load { latency: [0.0, 0.0, 2e-3, 0.0, 0.0], ..Load::default() }).unwrap());
        assert!(gpu.host < cpu.host, "the gpu costs the host less: {gpu:?} {cpu:?}");
        let mut load = Load { latency: [0.0, 0.0, 2e-3, 0.0, 0.0], cores: 1.0, ..Load::default() };
        load.slack = 1.0;
        assert_eq!(p.choose_host(&k, &able, &load).unwrap().0, Lane::Gpu);
        // No slack, and the gpu's latency far past the cores' time: the cores.
        load.slack = 0.0;
        load.latency[2] = 1.0;
        assert_eq!(p.choose_host(&k, &able, &load).unwrap().0, Lane::Cpu);
        // The cores' wall time stretches under load.
        let loaded = p.cost(&k, Lane::Cpu, &Load { cores: 4.0, ..Load::default() }).unwrap();
        assert!((loaded.wall / cpu.wall - 4.0).abs() < 1e-9);
    }
}
