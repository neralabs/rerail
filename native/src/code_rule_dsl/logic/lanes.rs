//! The engine's compute lanes (decision 0098 §2): one executor per binding, shared by every stratum of the process.
//!
//! - Under the **CPU binding**, the CPU's lanes: the cores and, where the CPU has one, its matrix unit (SME2). The
//!   CPU is a target in its own right (0095 §7), and SME is part of it.
//! - Under the **Metal binding**, every lane this machine has: the cores, SME, the GPU and the Neural Engine. Each
//!   launch goes where `rerail_accel::price` places it: the least energy among the lanes within the parity band of
//!   the first to finish.
//!
//! Which binding a thread's run has is the placement planner's (`plan::placement::set_device`).

use rerail_accel::Executor;
use rerail_accel::exec::{Backend, CpuBackend};
use std::sync::{Arc, LazyLock};

static CPU: LazyLock<Executor> = LazyLock::new(|| {
    let all = Executor::new();
    let mut lanes: Vec<Arc<dyn Backend>> = vec![Arc::new(CpuBackend)];
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    if all.lanes().contains(&rerail_accel::Lane::Sme) {
        lanes.extend(rerail_accel::sme::backend());
    }
    drop(all);
    Executor::with(lanes)
});
static ALL: LazyLock<Executor> = LazyLock::new(Executor::new);

/// The executor for this thread's run.
pub(in crate::code_rule_dsl) fn executor() -> &'static Executor {
    if super::plan::placement::device() { &ALL } else { &CPU }
}

/// The acceleration profile (decision 0098 §4): per form, where a launch's time goes from recognizing the form to
/// consuming its output. Each form's host phases are timed here (`host`), each batch's executor report is added
/// (`run`), and `report` renders the profile's section. Only under `RERAIL_PROFILE`.
pub(in crate::code_rule_dsl) mod prof {
    use super::super::profile;
    use rerail_accel::exec::{Done, Report, thread_cpu};
    use rerail_accel::{Batch, Lane};
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::sync::{LazyLock, Mutex};
    use std::time::Instant;

    #[derive(Default, Clone, Copy)]
    struct Time {
        n: u64,
        wall: f64,
        cpu: f64,
    }

    #[derive(Default)]
    struct Form {
        /// Host phases by name, in first-seen order: calls, wall seconds, this thread's CPU seconds.
        host: Vec<(&'static str, Time)>,
        batches: u64,
        /// Per lane: launches, submissions, wall, the lane thread's CPU, the backend's phases.
        launches: [u64; 4],
        submissions: [u64; 4],
        wall: [f64; 4],
        lane_cpu: [f64; 4],
        encode: [f64; 4],
        device: [f64; 4],
        device_known: [bool; 4],
        decode: [f64; 4],
        worker_cpu: [f64; 4],
        /// The executor's own: placement, and spawning/joining the lanes' threads.
        total: f64,
        place: f64,
        overhead: f64,
        /// Counted events by name (probes answered, rows emitted, rows walked before a table paid).
        counts: Vec<(&'static str, u64)>,
    }

    static FORMS: LazyLock<Mutex<BTreeMap<&'static str, Form>>> = LazyLock::new(Default::default);

    /// A host phase of `form`, timed until dropped.
    pub(in crate::code_rule_dsl) struct Host(Option<(&'static str, &'static str, Instant, f64)>);

    pub(in crate::code_rule_dsl) fn host(form: &'static str, phase: &'static str) -> Host {
        Host(profile::on().then(|| (form, phase, Instant::now(), thread_cpu())))
    }

    impl Drop for Host {
        fn drop(&mut self) {
            let Some((form, phase, t, c)) = self.0.take() else { return };
            let (wall, cpu) = (t.elapsed().as_secs_f64(), thread_cpu() - c);
            let mut forms = FORMS.lock().unwrap();
            let f = forms.entry(form).or_default();
            match f.host.iter_mut().find(|h| h.0 == phase) {
                Some(h) => {
                    h.1.n += 1;
                    h.1.wall += wall;
                    h.1.cpu += cpu;
                }
                None => f.host.push((phase, Time { n: 1, wall, cpu })),
            }
        }
    }

    /// Counts `n` events of `what` for `form`.
    pub(in crate::code_rule_dsl) fn count(form: &'static str, what: &'static str, n: u64) {
        if !profile::on() || n == 0 {
            return;
        }
        let mut forms = FORMS.lock().unwrap();
        let f = forms.entry(form).or_default();
        match f.counts.iter_mut().find(|c| c.0 == what) {
            Some(c) => c.1 += n,
            None => f.counts.push((what, n)),
        }
    }

    /// Runs `batch` on this thread's executor, its report added to `form`'s.
    pub(in crate::code_rule_dsl) fn run(form: &'static str, batch: Batch) -> Result<(Vec<Done>, Report), String> {
        let r = super::executor().run(batch)?;
        if profile::on() {
            let rep = &r.1;
            let mut forms = FORMS.lock().unwrap();
            let f = forms.entry(form).or_default();
            f.batches += 1;
            f.total += rep.total;
            f.place += rep.place;
            f.overhead += rep.overhead();
            for l in Lane::ALL {
                let i = l.index();
                if rep.launches[i] == 0 {
                    continue;
                }
                f.launches[i] += rep.launches[i] as u64;
                f.submissions[i] += 1;
                f.wall[i] += rep.wall[i];
                f.lane_cpu[i] += rep.lane_cpu[i];
                let p = &rep.phases[i];
                f.encode[i] += p.encode;
                f.decode[i] += p.decode;
                f.worker_cpu[i] += p.worker_cpu;
                if let Some(d) = p.device {
                    f.device[i] += d;
                    f.device_known[i] = true;
                }
            }
        }
        Ok(r)
    }

    /// The section: per form, the host phases, then per lane the launches and where the submission's time went, then
    /// the executor's own time.
    pub(in crate::code_rule_dsl) fn report() -> String {
        let forms = FORMS.lock().unwrap();
        let mut r = String::new();
        if forms.is_empty() {
            return r;
        }
        let ms = |s: f64| s * 1e3;
        let _ = writeln!(r, "\n== Acceleration (decision 0098): planning, launching and execution per form");
        let _ = writeln!(r, "Host phases are timed on the engine's thread (wall, and that thread's CPU). Per lane: the submission's");
        let _ = writeln!(r, "wall, the lane thread's CPU, the backend's encode / device / decode split and its workers' CPU. The");
        let _ = writeln!(r, "executor's own: placement, and spawning and joining lane threads (its wall less the slowest lane).");
        let mut grand = (0.0f64, 0.0f64);
        for (name, f) in forms.iter() {
            let host_cpu: f64 = f.host.iter().map(|h| h.1.cpu).sum();
            let kernel: f64 = (0..4).map(|i| if f.device_known[i] { f.device[i] } else { f.wall[i] - f.encode[i] - f.decode[i] }).sum();
            let _ = writeln!(r, "\n{name}: {} batches; host {:.1} ms CPU; executor {:.1} ms wall ({:.1} placing, {:.1} threads); kernels {:.1} ms",
                f.batches, ms(host_cpu), ms(f.total), ms(f.place), ms(f.overhead), ms(kernel));
            grand.0 += host_cpu + f.place + f.overhead + f.encode.iter().sum::<f64>() + f.decode.iter().sum::<f64>();
            grand.1 += kernel;
            for (phase, t) in &f.host {
                let _ = writeln!(r, "  host  {phase:<44} {:>7} calls {:>10.2} ms wall {:>10.2} ms CPU", t.n, ms(t.wall), ms(t.cpu));
            }
            for l in Lane::ALL {
                let i = l.index();
                if f.launches[i] == 0 {
                    continue;
                }
                let dev = if f.device_known[i] { format!("{:.2}", ms(f.device[i])) } else { "-".into() };
                let _ = writeln!(r, "  lane  {:<4} {:>5} launches in {:>5} submissions: {:>9.2} ms wall, {:>9.2} ms lane CPU; encode {:.2}, device {dev}, decode {:.2}, workers {:.2} ms CPU",
                    l.name(), f.launches[i], f.submissions[i], ms(f.wall[i]), ms(f.lane_cpu[i]), ms(f.encode[i]), ms(f.decode[i]), ms(f.worker_cpu[i]));
            }
            for (what, n) in &f.counts {
                let _ = writeln!(r, "  count {what:<44} {n:>12}");
            }
        }
        let _ = writeln!(r, "\nAll forms: {:.1} ms around the kernels (host phases' CPU, placing, threads, lanes' encode and decode) against {:.1} ms in them", ms(grand.0), ms(grand.1));
        r
    }
}
