//! What a launch costs on each lane: time, from the spikes' measurements, and energy, from published power figures.
//!
//! **Time.** Each (form, lane) has a cost per unit of the kernel's size ([`crate::Kernel::units`]), and each lane a
//! fixed cost per submission and per launch within one. They are the lanes' own measurements (M5 Max, 2026-11-09,
//! debug builds, load 35–140), each a workload's best time over its size in units; the CPU's are the matrix-offload
//! spikes' (`tools/matrix-offload/RESULTS.md`). The relational lab replaces them with prices measured on captured
//! strata ([`Prices::set`]).
//!
//! | Form | Workload | CPU (core-s) | SME | GPU (device) | ANE |
//! |---|---|---|---|---|---|
//! | bit product | W5, 5.5·10^10 bit-ops | 57 ms (NEON) | 4.4 ms (8e-14/unit) | 0.38–0.49 ms (W1: 8.9e-14/unit) | 7.5 ms (W5 1.4e-13, W1 5.0e-13) |
//! | count table, counts | W2, 5.8·10^6 rows | 2.3 ms | — | 0.57 ms | — |
//! | count table, pairs | W2, 6.2·10^8 units | (est. 3e-9/unit) | — | 5.85 ms | — |
//! | histogram | W4, 22.3·10^6 codes | 5.5 ms | — | 0.38 ms | — |
//! | closure | W3, 1.1–2.2·10^7 edge-words | 0.4 ms | — | 1.98 ms | — |
//!
//! Fixed costs: SME 3 µs a submission; the GPU 0.3–0.7 ms of wall a command buffer (0.2 ms of it host CPU, a few
//! µs on the device); the Neural Engine 0.13 ms a tile of 2,048 rows within one batched prediction, and a few ms of
//! host conversion a submission in a debug build.
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
        // Bit product: the lanes' own measurements (see the module's comment).
        set(Form::BitProduct, Lane::Cpu, 57e-3 / 5.5e10);
        set(Form::BitProduct, Lane::Sme, 8e-14);
        set(Form::BitProduct, Lane::Gpu, 9e-14);
        set(Form::BitProduct, Lane::Ane, 2.5e-13);
        set(Form::CountTable, Lane::Cpu, 2.3e-3 / 5.8e6);
        set(Form::CountTable, Lane::Gpu, 0.57e-3 / 5.8e6);
        set(Form::CountPairs, Lane::Cpu, 3e-9);
        set(Form::CountPairs, Lane::Gpu, 9.4e-12);
        set(Form::Histogram, Lane::Cpu, 5.5e-3 / 22.3e6);
        set(Form::Histogram, Lane::Gpu, 1.7e-11);
        set(Form::Closure, Lane::Cpu, 0.4e-3 / 2.17e7);
        set(Form::Closure, Lane::Gpu, 1.8e-10);
        Prices {
            per_unit,
            fixed: [
                Fixed { submission: 0.0, launch: 5e-6 },
                // Entering streaming mode, and a worker's handoff.
                Fixed { submission: 3e-6, launch: 1e-6 },
                // A command buffer's submit and wait, and an encoder's dispatch.
                Fixed { submission: 0.5e-3, launch: 5e-6 },
                // The batched prediction's host conversions, and a tile of 2,048 rows.
                Fixed { submission: 2e-3, launch: 0.13e-3 },
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
