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
static LEARNED: std::sync::RwLock<Vec<Learned>> = std::sync::RwLock::new(Vec::new());

/// A rate learned from samples ([`learn_from`]).
#[derive(Clone, Debug)]
pub struct Learned {
    pub name: String,
    pub lane: Lane,
    /// Seconds a unit, the samples' median.
    pub per_unit: f64,
    pub samples: usize,
}

/// The fewest warm samples a rate is fitted from.
pub const MIN_SAMPLES: usize = 8;

/// Bumped at each change of [`LEARNED`]: a price reads its thread's copy, taken again only when this moves, so pricing
/// (on every thread, for every launch) touches no shared lock.
static LEARNED_VERSION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

thread_local! {
    static LEARNED_COPY: std::cell::RefCell<(u64, Vec<(String, Lane, f64)>)> = const { std::cell::RefCell::new((0, Vec::new())) };
}

fn learned(name: &str, lane: Lane) -> Option<f64> {
    let version = LEARNED_VERSION.load(std::sync::atomic::Ordering::Acquire);
    if version == 0 {
        return None;
    }
    LEARNED_COPY.with_borrow_mut(|(seen, copy)| {
        if *seen != version {
            *copy = LEARNED.read().map(|l| l.iter().map(|x| (x.name.clone(), x.lane, x.per_unit)).collect()).unwrap_or_default();
            *seen = version;
        }
        copy.iter().find(|x| x.1 == lane && x.0 == name).map(|x| x.2)
    })
}

/// The rates learned so far.
pub fn learned_rates() -> Vec<Learned> {
    LEARNED.read().map(|l| l.clone()).unwrap_or_default()
}

/// Fits per-unit rates from `samples` and puts them in place of the priors; the rates fitted.
///
/// Only what one sample measures directly is fitted: a device lane's wall (its submission's fixed costs taken out)
/// and a program's on the cores (one job on one thread). A table form on the cores splits over threads, so its
/// sample's wall is not one unit's cost; it keeps its prior. Each rate is the median of its warm, successful samples
/// of one kernel name alone, with at least [`MIN_SAMPLES`] of them.
pub fn learn_from(samples: &[crate::score::Sample]) -> Vec<Learned> {
    let p = Prices::default();
    let mut by: std::collections::BTreeMap<(String, usize), Vec<f64>> = Default::default();
    for s in samples {
        let name = s.forms.split(',').next().unwrap_or("");
        // One kernel name alone: a mixed job's wall is not one name's.
        if s.cold || !s.ok || s.units <= 0.0 || s.forms.split(',').any(|f| f != name) {
            continue;
        }
        let name = name.to_owned();
        let program = Form::ALL.iter().all(|f| f.name() != name);
        if s.lane == Lane::Cpu && !program {
            continue;
        }
        let fixed = p.fixed[s.lane.index()];
        let busy = s.wall - fixed.submission - fixed.launch * f64::from(s.launches);
        if busy > 0.0 {
            by.entry((name, s.lane.index())).or_default().push(busy / s.units);
        }
    }
    let fitted: Vec<Learned> = by
        .into_iter()
        .filter(|(_, v)| v.len() >= MIN_SAMPLES)
        .map(|((name, lane), mut v)| {
            v.sort_by(f64::total_cmp);
            Learned { name, lane: Lane::ALL[lane], per_unit: v[v.len() / 2], samples: v.len() }
        })
        .collect();
    if let Ok(mut l) = LEARNED.write() {
        for f in &fitted {
            l.retain(|x| !(x.name == f.name && x.lane == f.lane));
            l.push(f.clone());
        }
        if !fitted.is_empty() {
            LEARNED_VERSION.fetch_add(1, std::sync::atomic::Ordering::Release);
        }
    }
    fitted
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
        let per = learned(k.name(), lane).unwrap_or(prior);
        let busy = per * k.units();
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
        let per = match k {
            Kernel::Program(p) => p.0.host_per_unit(lane)?,
            _ => self.host_per_unit[form_index(k.form())][lane.index()]?,
        };
        let fixed = self.host[lane.index()];
        Some(per * k.units() + fixed.launch + if open { 0.0 } else { fixed.submission })
    }

    /// `k`'s cost on `lane` under `load`.
    pub fn cost(&self, k: &Kernel, lane: Lane, load: &Load) -> Option<Cost> {
        let i = lane.index();
        let slow = if matches!(lane, Lane::Cpu | Lane::Sme | Lane::Sql) { load.cores.max(1.0) } else { 1.0 };
        let wall = self.wall(k, lane)? * slow;
        let queued = if lane.is_pool() { load.queued[Lane::Cpu.index()] + load.queued[Lane::Sql.index()] } else { load.queued[i] };
        let finish = queued + if load.open[i] { 0.0 } else { load.latency[i] } + wall;
        let scale = if load.host_scale[i] > 0.0 { load.host_scale[i] } else { 1.0 };
        Some(Cost { finish, wall, host: self.host(k, lane, load.open[i])? * scale })
    }

    /// Chooses a lane for `k` among `able` by host CPU (decision 0100 §9). The lanes that finish by the deadline are
    /// candidates: the later of the band times the first to finish, and the caller's slack (it works that long
    /// before it waits, so a lane done by then costs it no wall time). Among them, the least host CPU wins, then the
    /// least energy. The chosen lane's cost, and the fastest's.
    pub fn choose_host(&self, k: &Kernel, able: &[Lane], load: &Load) -> Option<(Lane, Cost, Cost)> {
        let costs: Vec<(Lane, Cost)> = able.iter().filter_map(|&l| Some((l, self.cost(k, l, load)?))).collect();
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
        let keys = |n: usize| -> crate::kernel::Keys { vec![Col::from((0..n as i64).collect::<Vec<_>>())].into() };
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
