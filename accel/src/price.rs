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
//! - the cores: about 2 µs a launch within a submission (one parallel pass on the rayon pool);
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
//! **Placement** ([`Prices::choose`]): the lane that finishes first, given what is already queued on each lane, is
//! the reference. Every lane that would finish within the **parity band** of it (`band`, 3× to start; the user,
//! 2026-11-09) is a candidate, and the candidate that spends the least energy is chosen. So near parity, the Neural
//! Engine takes the work: it frees the CPU and the GPU, and draws a fraction of their power.

use crate::kernel::{Form, Kernel};
use crate::lanes::Lane;

/// Watts per lane while busy (see the module's comment for the sources).
pub const WATTS: [f64; 4] = [6.0, 12.0, 45.0, 3.0];

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
    per_unit: [[Option<f64>; 4]; 5],
    /// Per lane; for the CPU lane, wall seconds (the cores' time is `per_unit` divided by `cores`).
    pub fixed: [Fixed; 4],
    /// The cores the CPU lane spreads a launch over.
    pub cores: f64,
    /// The parity band: lanes finishing within `band` × the fastest are candidates, the least energy wins.
    pub band: f64,
}

fn form_index(f: Form) -> usize {
    f as usize
}

impl Default for Prices {
    fn default() -> Self {
        let mut per_unit = [[None; 4]; 5];
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
        Prices {
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
            ],
            cores: std::thread::available_parallelism().map_or(8, |n| n.get()) as f64,
            band: 3.0,
        }
    }
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
        let busy = self.per_unit(k.form(), lane)? * k.units();
        let busy = if lane == Lane::Cpu { busy / self.cores } else { busy };
        Some(self.fixed[lane.index()].launch + busy)
    }

    /// The energy, in joules, of `wall` seconds on `lane` (the CPU lane busy on `cores`).
    pub fn joules(&self, lane: Lane, wall: f64) -> f64 {
        let busy = if lane == Lane::Cpu { wall * self.cores } else { wall };
        WATTS[lane.index()] * busy
    }

    /// Chooses a lane for `k` among `able` lanes, given each lane's `queued` seconds: the least energy among the
    /// lanes that finish within the band of the first to finish. A lane with nothing queued yet pays its submission.
    pub fn choose(&self, k: &Kernel, able: &[Lane], queued: &[f64; 4]) -> Option<(Lane, f64)> {
        let finish = |l: Lane| -> Option<f64> {
            let q = queued[l.index()];
            let sub = if q == 0.0 { self.fixed[l.index()].submission } else { 0.0 };
            Some(q + sub + self.wall(k, l)?)
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
