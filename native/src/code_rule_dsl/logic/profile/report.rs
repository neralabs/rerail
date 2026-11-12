//! Moved from `profile.rs` by the crate split (tools/cratify/moves.tsv).

use super::*;

/// The engine stratum `key` ran on, and why (the first reason given; one loop per call).
/// Records where native stratum `key` ran (`plan::placement`), its shape, the work it did and the CPU it took.
pub(in crate::code_rule_dsl) fn placed(key: &str, p: &crate::code_rule_dsl::logic::plan::placement::Placement, shape: &crate::code_rule_dsl::logic::plan::placement::Shape, work: u64, cpu: f64, device: Option<&DeviceReport>) {
    if !on() {
        return;
    }
    let mut state = super::state();
    let t = state.placements.entry(key.to_owned()).or_default();
    t.runs += 1;
    let ran = match device {
        Some(d) if d.refused.is_none() => {
            t.gpu.add(&GpuTotal {
                runs: 1,
                rules: u64::from(d.rules),
                stages: u64::from(d.stages),
                submissions: u64::from(d.submissions),
                dispatches: u64::from(d.dispatches),
                reruns: u64::from(d.reruns),
                gpu: d.gpu_seconds,
                encode: d.encode_cpu,
                upload: d.upload_cpu,
                wait: d.wait_cpu,
                wait_wall: d.wait_wall,
                decode: d.decode_cpu,
                uploads: u64::from(d.uploads),
                upload_bytes: d.upload_bytes,
                resident: u64::from(d.resident_hits),
                sorts: u64::from(d.sorts),
                sort_hits: u64::from(d.sort_hits),
                rows: d.rows,
                distinct: d.distinct,
                bindings: d.bindings,
                made: d.made,
                waved: u64::from(d.waved),
                arena_reruns: u64::from(d.arena_reruns),
                sized: u64::from(d.sized),
            });
            true
        }
        Some(d) => {
            t.refused = d.refused.clone();
            false
        }
        None => false,
    };
    t.on_gpu += u64::from(ran);
    t.gpu_cheaper += u64::from(p.gpu_cheaper);
    t.device |= p.device;
    t.forms = shape.forms.iter().map(|f| f.name()).collect();
    t.missing = p.missing.iter().map(|f| f.name()).collect();
    t.planned_work += shape.work;
    t.work += work;
    t.cpu_estimate += p.cpu_s;
    t.gpu_estimate += p.gpu_s;
    t.cpu += cpu;
    for c in &p.joins {
        use crate::code_rule_dsl::logic::plan::placement::JoinForm;
        let chosen = match c.form {
            JoinForm::Index => c.index_s,
            JoinForm::Probe => c.probe_s.unwrap_or(c.index_s),
            JoinForm::Tile => c.tile_s.unwrap_or(c.index_s),
        };
        let e = &mut t.joins[c.form as usize];
        e.0 += 1;
        if c.index_s < JOIN_ESTIMATE_CAP {
            e.1 += c.index_s;
            e.2 += chosen;
        } else {
            e.3 += 1;
        }
    }
}

/// Where the native strata ran (decision 0095): how many on each backend, the strata the GPU would run cheaper and
/// the forms it lacks for them, by their CPU. What the device must take on next, and what it would save.
pub(in crate::code_rule_dsl) fn placements(r: &mut String, state: &State) {
    use crate::code_rule_dsl::logic::plan::placement::gpu_price;
    let all = &state.placements;
    if all.is_empty() {
        return;
    }
    let runs: u64 = all.values().map(|t| t.runs).sum();
    let gpu: u64 = all.values().map(|t| t.on_gpu).sum();
    let cpu: f64 = all.values().map(|t| t.cpu).sum();
    let device = all.values().any(|t| t.device);
    let _ = writeln!(
        r,
        "\n== Placement (decision 0095): {} strata, {runs} runs; {gpu} runs on the GPU, {} on the CPU ({cpu:.2} s CPU); binding {}",
        all.len(),
        runs - gpu,
        if device { "Metal" } else { "CPU" }
    );
    let _ = writeln!(r, "Prices: CPU measured (100 ns a unit of planned work, 0.13 ms a stratum); GPU: each join step on its device form (the relational lab's measures), each stage and submission at the host's measured cost.");
    device_strata(r, all);
    // The strata the GPU would run cheaper in at least one run, and did not.
    let cheaper: Vec<(&String, &PlacedTotal)> = all.iter().filter(|(_, t)| t.gpu_cheaper > t.on_gpu).collect();
    let cheaper_cpu: f64 = cheaper.iter().map(|(_, t)| t.cpu).sum();
    // The GPU priced at the work each stratum did, not the work planned: a planned estimate can be far off (a cross
    // product priced at 10^20 rows that met none).
    let at_work = |t: &PlacedTotal| gpu_price(t.runs, t.work as f64);
    let cheaper_gpu: f64 = cheaper.iter().map(|(_, t)| at_work(t)).sum();
    let _ = writeln!(r, "The GPU would be cheaper for {} strata: {cheaper_cpu:.2} s CPU taken, against {cheaper_gpu:.2} s on the GPU at the work they did.", cheaper.len());
    // Strata whose planned work is off by 10x or more, either way, by their CPU: the placement read a wrong price, and
    // the join order likely came from the same estimate.
    let off = |t: &PlacedTotal| {
        let (p, w) = (t.planned_work.max(1.0), (t.work as f64).max(1.0));
        w / p >= 10.0 || p / w >= 10.0
    };
    let (under, over) = all.values().filter(|t| off(t)).fold(((0u64, 0.0), (0u64, 0.0)), |(u, o), t| {
        if (t.work as f64) > t.planned_work { ((u.0 + 1, u.1 + t.cpu), o) } else { (u, (o.0 + 1, o.1 + t.cpu)) }
    });
    let _ = writeln!(r, "Planned work off by 10x or more: {} strata under (did more, {:.2} s CPU), {} over (did less, {:.2} s CPU); marked ! below.", under.0, under.1, over.0, over.1);
    // By the form the GPU lacks: a stratum counts under each form it lacks.
    let mut by: BTreeMap<&str, (u64, f64)> = BTreeMap::new();
    for (_, t) in &cheaper {
        for f in &t.missing {
            let e = by.entry(f).or_default();
            e.0 += 1;
            e.1 += t.cpu;
        }
    }
    let mut by: Vec<_> = by.into_iter().collect();
    by.sort_by(|a, b| b.1.1.total_cmp(&a.1.1));
    let _ = writeln!(r, "GPU support missing, by form (a stratum counts under each form it lacks):");
    let _ = writeln!(r, "{:>9} {:>7}  form", "cpu s", "strata");
    for (f, (n, c)) in &by {
        let _ = writeln!(r, "{c:>9.3} {n:>7}  {f}");
    }
    // The strata themselves, costliest first.
    let mut top = cheaper.clone();
    top.sort_by(|a, b| b.1.cpu.total_cmp(&a.1.cpu));
    // Each join step on the form its prices chose (the lab's measured device prices): what the device's joins would
    // take on, in strata the device runs once it has their other forms.
    let joins: Vec<(u64, f64, f64, u64)> = (0..3).map(|f| all.values().fold((0, 0.0, 0.0, 0), |a, t| (a.0 + t.joins[f].0, a.1 + t.joins[f].1, a.2 + t.joins[f].2, a.3 + t.joins[f].3))).collect();
    if joins.iter().any(|j| j.0 > 0) {
        use crate::code_rule_dsl::logic::plan::placement::JoinForm;
        let _ = writeln!(r, "Join steps by the form their prices choose (estimates; the device's prices measured by the relational lab; steps estimated over {JOIN_ESTIMATE_CAP:.0} s counted, not summed):");
        let _ = writeln!(r, "{:>9} {:>10} {:>10} {:>9}  form", "steps", "index s", "chosen s", "over cap");
        for (f, form) in [JoinForm::Index, JoinForm::Probe, JoinForm::Tile].into_iter().enumerate() {
            let _ = writeln!(r, "{:>9} {:>10.3} {:>10.3} {:>9}  {}", joins[f].0, joins[f].1, joins[f].2, joins[f].3, form.name());
        }
    }
    let _ = writeln!(r, "Costliest strata the GPU would run cheaper:");
    let _ = writeln!(r, "{:>8} {:>8} {:>12} {:>12} {:>5}  stratum: forms (missing on the GPU)", "cpu s", "gpu s", "planned", "work", "runs");
    for (key, t) in top.iter().take(20) {
        let mark = if off(t) { "!" } else { " " };
        let _ = writeln!(r, "{:>8.3} {:>8.4} {:>12.0} {:>11}{mark} {:>5}  {}: {} (missing {})", t.cpu, at_work(t), t.planned_work, t.work, t.runs, short(key, 60), t.forms.join(", "), t.missing.join(", "));
    }
}

/// Where the process's CPU went, by thread: the spans measure their own thread's CPU, so a worker's is under a span
/// only where it opens one (`span_at`). Threads by name, digits folded (a pool's threads are one line).
fn cpu_by_thread(r: &mut String, threads: &super::Threads) {
    let covered: f64 = threads.rows.iter().map(|t| t.covered).sum();
    let _ = writeln!(
        r,
        "\n== CPU by thread: process {:.2} s; under spans {:.2} s ({:.0}%), outside every span {:.2} s",
        threads.process,
        covered,
        100.0 * covered / threads.process.max(1e-9),
        (threads.process - covered).max(0.0)
    );
    let mut groups: BTreeMap<String, (usize, f64, f64)> = BTreeMap::new();
    for t in &threads.rows {
        let name: String = t.name.chars().map(|c| if c.is_ascii_digit() { '#' } else { c }).collect();
        let g = groups.entry(name).or_default();
        g.0 += 1;
        g.1 += t.cpu;
        g.2 += t.covered;
    }
    let mut by: Vec<_> = groups.into_iter().filter(|(_, g)| g.1 >= 0.005).collect();
    by.sort_by(|a, b| (b.1.1 - b.1.2).total_cmp(&(a.1.1 - a.1.2)));
    let _ = writeln!(r, "{:>9} {:>9} {:>9} {:>8}  thread", "cpu s", "spans s", "outside s", "threads");
    for (name, (n, cpu, cov)) in by.iter().take(20) {
        let _ = writeln!(r, "{cpu:>9.2} {cov:>9.2} {:>9.2} {n:>8}  {name}", (cpu - cov).max(0.0));
    }
    // By the spans' last steps: a pool's items share them wherever they were started (gaps.tsv has the paths).
    let mut by: BTreeMap<(&str, &str), (f64, usize)> = BTreeMap::new();
    let gaps = super::gaps();
    for g in &gaps {
        let e = by.entry((last_step(&g.after), last_step(&g.before))).or_default();
        e.0 += g.cpu;
        e.1 += 1;
    }
    let mut by: Vec<_> = by.into_iter().filter(|(_, (cpu, _))| *cpu >= 0.01).collect();
    by.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
    if !by.is_empty() {
        let total: f64 = gaps.iter().map(|g| g.cpu).sum();
        let _ = writeln!(r, "Outside every span, between a thread's spans: {total:.2} s (the outermost span that closed -> the next that opened, by");
        let _ = writeln!(r, "their last steps; on a pool's threads, its own work between items: stealing, splitting, spinning; gaps.tsv):");
        let _ = writeln!(r, "{:>9} {:>7}  closed -> opened", "cpu s", "paths");
        for ((after, before), (cpu, n)) in by.iter().take(12) {
            let _ = writeln!(r, "{cpu:>9.2} {n:>7}  {after} -> {before}");
        }
    }
}

/// A span path's last step.
fn last_step(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The report, and the tables, into `dir`. `header`: what was run (lines put first).
pub fn write(dir: &Path, header: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let threads = super::threads();
    let mut threads_tsv = String::from("thread\tcpu_s\tunder_spans_s\toutside_spans_s\n");
    for t in &threads.rows {
        let _ = writeln!(threads_tsv, "{}\t{:.4}\t{:.4}\t{:.4}", t.name, t.cpu, t.covered, (t.cpu - t.covered).max(0.0));
    }
    std::fs::write(dir.join("threads.tsv"), threads_tsv)?;
    let mut gaps_tsv = String::from("after\tbefore\tcpu_s\n");
    for g in super::gaps() {
        let _ = writeln!(gaps_tsv, "{}\t{}\t{:.4}", g.after, g.before, g.cpu);
    }
    std::fs::write(dir.join("gaps.tsv"), gaps_tsv)?;
    let state = super::state();
    let mut spans_tsv = String::from("path\tcalls\tcpu_s\town_s\talloc_bytes\town_alloc_bytes\tfreed_bytes\tpeak_rise_bytes\town_peak_rise_bytes\n");
    for (p, t) in &state.spans {
        let _ = writeln!(spans_tsv, "{p}\t{}\t{:.4}\t{:.4}\t{}\t{}\t{}\t{}\t{}", t.calls, t.total, t.own, t.alloc, t.own_alloc, t.freed, t.rise, t.own_rise);
    }
    std::fs::write(dir.join("spans.tsv"), spans_tsv)?;
    std::fs::write(dir.join("memory.tsv"), memory::sampled_tsv())?;
    let mut sql: Vec<_> = state.sql.iter().collect();
    sql.sort_by_key(|(_, s)| std::cmp::Reverse(s.ns));
    let mut sql_tsv = String::from("span\truns\tms\tvm_steps\tfullscan_steps\tsorts\tautoindex_rows\tsql\n");
    for ((p, text), s) in &sql {
        let _ = writeln!(sql_tsv, "{p}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{text}", s.runs, s.ns as f64 / 1e6, s.vm, s.scan, s.sort, s.autoindex);
    }
    std::fs::write(dir.join("sql.tsv"), sql_tsv)?;
    let work = |steps: &Vec<(u64, u64)>| steps.iter().map(|(a, _)| a).sum::<u64>();
    let mut native: Vec<_> = state.native.iter().collect();
    native.sort_by_key(|(_, s)| std::cmp::Reverse(work(s)));
    let mut native_tsv = String::from("span\twork\tsteps_in_out\tplan\tcpu_s\n");
    for ((p, plan), steps) in &native {
        let s: Vec<String> = steps.iter().map(|(a, b)| format!("{a}:{b}")).collect();
        let cpu = state.native_cpu.get(&(p.clone(), plan.clone())).copied().unwrap_or(0.0);
        let _ = writeln!(native_tsv, "{p}\t{}\t{}\t{plan}\t{cpu:.4}", work(steps), s.join(","));
    }
    std::fs::write(dir.join("native.tsv"), native_tsv)?;
    let mut placement_tsv = String::from("stratum\truns\ton_gpu\tgpu_cheaper\tplanned_work\twork\tcpu_s\tcpu_estimate_s\tgpu_estimate_s\tforms\tmissing\n");
    for (key, t) in &state.placements {
        let _ = writeln!(
            placement_tsv,
            "{key}\t{}\t{}\t{}\t{:.0}\t{}\t{:.4}\t{:.4}\t{:.4}\t{}\t{}",
            t.runs, t.on_gpu, t.gpu_cheaper, t.planned_work, t.work, t.cpu, t.cpu_estimate, t.gpu_estimate, t.forms.join(","), t.missing.join(",")
        );
    }
    std::fs::write(dir.join("placement.tsv"), placement_tsv)?;

    // The report.
    let mut r = String::new();
    r.push_str(header);
    cpu_by_thread(&mut r, &threads);
    let (analysis_text, analysis_json) = analysis_report::report(&state);
    std::fs::write(dir.join("analysis.txt"), &analysis_text)?;
    std::fs::write(dir.join("analysis.json"), serde_json::to_vec_pretty(&analysis_json)?)?;
    r.push('\n');
    r.push_str(&analysis_text);
    fallbacks(&mut r, &state);
    executors(&mut r, &state);
    hazards(&mut r, &state);
    placements(&mut r, &state);
    explain(&mut r, &state);
    // A root is a span with no parent recorded: a top span, or one whose parent was still open (a dump).
    let root = |p: &str| p.rsplit_once('/').is_none_or(|(parent, _)| !state.spans.contains_key(parent));
    let roots: f64 = state.spans.iter().filter(|(p, _)| root(p)).map(|(_, t)| t.total).sum();
    let _ = writeln!(r, "\n== Spans: CPU (user + system), summed over calls; own = less the spans inside it ({roots:.2} s in all)");
    let _ = writeln!(r, "{:>9} {:>9} {:>6} {:>9}  span", "cpu s", "own s", "%", "calls");
    // A tree: each span under its parent, children by CPU.
    fn walk(r: &mut String, spans: &BTreeMap<String, SpanTotal>, parent: Option<&str>, depth: usize, all: f64) {
        let mut kids: Vec<(&String, &SpanTotal)> = spans
            .iter()
            .filter(|(p, _)| match parent {
                None => p.rsplit_once('/').is_none_or(|(parent, _)| !spans.contains_key(parent)),
                Some(par) => p.strip_prefix(par).and_then(|rest| rest.strip_prefix('/')).is_some_and(|rest| !rest.contains('/')),
            })
            .collect();
        kids.sort_by(|a, b| b.1.total.total_cmp(&a.1.total));
        for (p, t) in kids {
            // Spans under 0.1% of the run are left out of the tree (the tables keep them).
            if t.total < all * 0.001 && depth > 0 {
                continue;
            }
            let name = p.rsplit('/').next().unwrap_or(p);
            let _ = writeln!(r, "{:>9.3} {:>9.3} {:>5.1}% {:>9}  {}{}", t.total, t.own, 100.0 * t.total / all.max(1e-9), t.calls, "  ".repeat(depth), short(name, 100));
            walk(r, spans, Some(p), depth + 1, all);
        }
    }
    walk(&mut r, &state.spans, None, 0, roots);
    let kinds_tsv = kinds::by_kind(&mut r, &state.spans);
    std::fs::write(dir.join("kinds.tsv"), kinds_tsv)?;
    kinds::unexplained(&mut r, &state.spans);
    kinds::searches(&mut r, &state.spans);
    let heads_tsv = kinds::by_head(&mut r, &native, &state.native_cpu);
    std::fs::write(dir.join("heads.tsv"), heads_tsv)?;
    memory_report(&mut r, &state.spans);

    let sql_ns: u64 = sql.iter().map(|(_, s)| s.ns).sum();
    let sql_vm: u64 = sql.iter().map(|(_, s)| s.vm).sum();
    let _ = writeln!(
        r,
        "\n== SQL statements by SQLite's time ({} distinct, {:.2} s, {sql_vm} VM steps in all); plans for the first 15",
        sql.len(),
        sql_ns as f64 / 1e9
    );
    let _ = writeln!(r, "{:>9} {:>7} {:>12} {:>11} {:>6} {:>9}  span / statement", "ms", "runs", "vm steps", "full scan", "sorts", "autoidx");
    for (i, ((p, text), s)) in sql.iter().take(40).enumerate() {
        let _ = writeln!(r, "{:>9.1} {:>7} {:>12} {:>11} {:>6} {:>9}  {p}", s.ns as f64 / 1e6, s.runs, s.vm, s.scan, s.sort, s.autoindex);
        let _ = writeln!(r, "{:>58}{}", "", short(text, 400));
        if i < 15
            && let Some(plan) = state.notes.get(&format!("plan:{text}"))
        {
            for line in plan.lines() {
                let _ = writeln!(r, "{:>60}{line}", "");
            }
        }
    }

    let _ = writeln!(r, "\n== Native joins by work (rows into every step, summed over chunks), with their probes' CPU; steps in:out");
    for ((p, plan), steps) in native.iter().take(30) {
        let s: Vec<String> = steps.iter().map(|(a, b)| format!("{a}:{b}")).collect();
        let cpu = state.native_cpu.get(&((*p).clone(), (*plan).clone())).copied().unwrap_or(0.0);
        let _ = writeln!(r, "{:>12}  {cpu:.2} s  {p}\n{:>14}steps {}\n{:>14}{}", work(steps), "", s.join(" "), "", short(plan, 600));
    }

    // The roofline: SQL's VM steps and the native joins' rows are charged here, the rest by `work`. A native
    // component's step counts are read after its run, so its rows go to the run's span (`…/engine run (joins)`).
    {
        let own: BTreeMap<String, f64> = state.spans.iter().map(|(p, t)| (p.clone(), t.own)).collect();
        let mut charged = state.work.clone();
        for ((p, _), s) in &sql {
            *charged.entry((p.clone(), Unit::VmStep)).or_default() += s.vm;
        }
        let mut joins: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        for ((p, _), steps) in &native {
            let run = format!("{p}/engine run (joins)");
            let at = if own.contains_key(&run) { run } else { p.clone() };
            // A join's work: the rows into each step, and the rows its last step makes.
            let made = steps.last().map_or(0, |s| s.1);
            *charged.entry((at.clone(), Unit::JoinRow)).or_default() += work(steps) + made;
            let j = joins.entry(at).or_default();
            j.0 += work(steps) + made;
            j.1 += made;
        }
        roofline::report(&mut r, &own, &charged, &joins);
    }

    let _ = writeln!(r, "\n== Waits: wall time each thread spent blocked, on what, and what that was doing when the wait began");
    if state.waits.is_empty() {
        r.push_str("(no waits)\n");
    } else {
        let _ = writeln!(r, "{:>9} {:>9} {:>7}  waiter: waited on (which was doing)", "wall s", "cpu s", "waits");
        let mut waits: Vec<_> = state.waits.iter().collect();
        waits.sort_by(|a, b| b.1.wall.total_cmp(&a.1.wall));
        for ((path, on, doing), w) in waits {
            let _ = writeln!(r, "{:>9.3} {:>9.3} {:>7}  {path}: {on} ({doing})", w.wall, w.cpu, w.count);
        }
    }

    r.push_str(&crate::code_rule_dsl::logic::lanes::prof::report());
    stages(&mut r);
    let _ = writeln!(r, "\n== Device (Metal): per span and kernel; GPU = command buffers' GPU time, wait = host wall time");
    if state.devices.is_empty() {
        r.push_str("(no device work)\n");
    }
    for ((p, k), d) in &state.devices {
        let _ = writeln!(
            r,
            "{:>9.3} ms GPU {:>9.3} ms wait {:>5} runs {:>5} launches {:>11} cells  {k}  ({p})",
            d.gpu * 1e3,
            d.wall * 1e3,
            d.runs,
            d.launches,
            d.cells
        );
        if d.phases.iter().any(|p| *p > 0.0) {
            for (name, s) in PHASES.iter().zip(d.phases) {
                let _ = writeln!(r, "{:>14.3} ms  {name}", s * 1e3);
            }
            let _ = writeln!(r, "{:>14} MB made, {} MB reused", d.made >> 20, d.reused >> 20);
        }
    }

    // The executor's scoreboard (rerail-accel): every job on every lane, its thread CPU and its phases. On a device
    // lane, `cpu s` is the host CPU the device cost; `wall` phases with no CPU are the device's own timeline.
    let _ = writeln!(r, "\n== Executor (rerail-accel): jobs per lane, host CPU per phase");
    let scores = crate::code_rule_dsl::logic::lanes::scores();
    if scores.is_empty() {
        r.push_str("(no executor)\n");
    }
    for (binding, score) in scores {
        let _ = writeln!(r, "-- {binding}");
        let _ = writeln!(r, "{score}");
    }

    // The cost statistics (decision 0100 §14): the store, the samples read, and the rates learned in place of priors.
    let _ = writeln!(r, "\n== Cost statistics (RERAIL_COSTS)");
    r.push_str(&crate::code_rule_dsl::logic::costs::summary());

    // Waits that ran others' jobs (decision 0100 §12): a thread waiting on the pool runs queued jobs meanwhile, and a
    // long one makes its own wait end late. Per site: its waits, the jobs they ran, and how late those made them.
    let (waits, late_by) = rerail_accel::exec::waits();
    let _ = writeln!(r, "\n== Waits that ran others' jobs (rerail-accel)");
    if waits.is_empty() {
        r.push_str("(no waits)\n");
    } else {
        let _ = writeln!(
            r,
            "{:<40} {:>7} {:>7} {:>7} {:>10} {:>10} {:>10} {:>6} {:>10} {:>10} {:>10}  longest job run",
            "wait", "waits", "helped", "jobs", "wait ms", "jobs ms", "jobs cpu", "late", "late ms<=", "late max", "late exact"
        );
        for (site, w) in &waits {
            let _ = writeln!(
                r,
                "{:<40} {:>7} {:>7} {:>7} {:>10.1} {:>10.1} {:>10.1} {:>6} {:>10.2} {:>10.2} {:>10.2}  {:.2} ms {}",
                short(site, 40),
                w.waits,
                w.helping,
                w.jobs,
                w.wall * 1e3,
                w.jobs_wall * 1e3,
                w.jobs_cpu * 1e3,
                w.late,
                w.late_wall * 1e3,
                w.late_max * 1e3,
                w.late_exact * 1e3,
                w.longest * 1e3,
                w.longest_job
            );
        }
        // `late ms<=`: a wait that found its work ready right after a job it ran was late by at most that job's
        // wall; `late exact` where the waited work said when it ended (a batch's launches, a host job).
        let mut late_by = late_by;
        late_by.sort_by(|a, b| b.1.late_wall.total_cmp(&a.1.late_wall));
        let _ = writeln!(r, "jobs that made waits late: {:<34} {:>7} {:>10} {:>10}", "job", "waits", "late ms<=", "late max");
        for (job, l) in late_by.iter().take(12) {
            let _ = writeln!(r, "  {:<58} {:>7} {:>10.2} {:>10.2}", short(job, 58), l.waits, l.late_wall * 1e3, l.late_max * 1e3);
        }
    }

    // Warnings: the shapes that usually explain a slow rule.
    let _ = writeln!(r, "\n== Warnings");
    let mut warned = 0;
    // A device path that failed and ran on the CPU instead.
    for ((topic, why), n) in &state.decisions {
        if topic.contains("fell back to the CPU") {
            let _ = writeln!(r, "- {topic}: {n} times: {why}");
            warned += 1;
        }
    }
    if !state.hazards.is_empty() {
        let runs: u64 = state.hazards.values().map(|v| v.0).sum();
        let _ = writeln!(r, "- {} SQL statements ({runs} runs) force waits inside the waves: see == Forces SQL waits.", state.hazards.len());
        warned += 1;
    }
    for ((p, text), s) in &sql {
        if s.autoindex > 0 {
            let _ = writeln!(r, "- automatic index ({} rows) built for a statement in {p}: inspect demand selectivity and join order before adding an index.\n    {}", s.autoindex, short(text, 300));
            warned += 1;
        }
        if s.vm > 0 && s.scan * 2 > s.vm && s.vm > 1_000_000 {
            let _ = writeln!(r, "- full scans are {:.0}% of {} VM steps in {p}: a join reads a whole table.\n    {}", 100.0 * s.scan as f64 / s.vm as f64, s.vm, short(text, 300));
            warned += 1;
        }
        if s.runs > 1000 && s.ns / s.runs.max(1) < 20_000 {
            let _ = writeln!(r, "- {} runs of one statement in {p}: per-row or per-chunk statement overhead.\n    {}", s.runs, short(text, 200));
            warned += 1;
        }
    }
    for ((p, plan), steps) in &native {
        // Step 0 is the seed (one binding a chunk), so its step is the first atom's scan, not a fan-out.
        for (i, w) in steps.windows(2).enumerate().skip(1) {
            if w[1].0 > 0 && w[0].0 > 0 && w[1].0 > 20 * w[0].0 && w[1].0 > 1_000_000 {
                let _ = writeln!(r, "- native step {} fans out {}x ({} -> {} rows) in {p}:\n    {}", i + 1, w[1].0 / w[0].0, w[0].0, w[1].0, short(plan, 300));
                warned += 1;
            }
        }
    }
    // Planning that is a visible share of the run: one warning, its costliest plans named.
    let mut slow: Vec<(&(String, String, String), &PlanTotal)> = state.plans.iter().filter(|(_, t)| t.planning > 0.01 * roots).collect();
    slow.sort_by(|a, b| b.1.planning.total_cmp(&a.1.planning));
    if !slow.is_empty() {
        let total: f64 = slow.iter().map(|(_, t)| t.planning).sum();
        let _ = writeln!(
            r,
            "- planning takes {total:.2} s CPU ({:.0}% of the run) in {} plans, each over 1% of the run; each is planned again per loop (chunk). A plan reused across chunks or a bounded search would remove it. The costliest:",
            100.0 * total / roots.max(1e-9),
            slow.len()
        );
        for ((key, kind, _), t) in slow.iter().take(5) {
            let _ = writeln!(
                r,
                "    {} in {key}: {:.2} s over {} loops, {} literals, {} subsets and {} candidates a plan",
                rule_name(t.rule, kind),
                t.planning,
                t.loops,
                t.plan.steps.len(),
                t.plan.subsets,
                t.plan.candidates
            );
        }
        warned += 1;
    }
    for ((key, kind, _), t) in &state.plans {
        for (i, (st, a)) in t.plan.steps.iter().zip(&t.actual).enumerate() {
            let est = st.rows * t.loops as f64;
            let got = a.1 as f64;
            if t.measured && (got > 100.0 * est.max(1.0) && got > 100_000.0) {
                let _ = writeln!(r, "- {} in {key}: step {} ({}) estimated {est:.0} rows, produced {got:.0}: the estimate is off by over 100x.", rule_name(t.rule, kind), i + 1, st.literal);
                warned += 1;
            }
        }
    }
    for ((p, k), d) in &state.devices {
        // Over a second waiting on the device.
        if d.wall > 1.0 {
            let _ = writeln!(
                r,
                "- {k} in {p}: {:.2} s of GPU time for {} cells ({:.0} ns a cell); on a shared GPU the time includes other work queued ahead.",
                d.gpu,
                d.cells,
                1e9 * d.gpu / (d.cells.max(1) as f64)
            );
            warned += 1;
        }
    }
    for ((topic, text), _) in &state.decisions {
        if topic == "fallback" {
            let _ = writeln!(r, "- fallback: {text}");
            warned += 1;
        }
    }
    if warned == 0 {
        r.push_str("(none)\n");
    }
    let _ = writeln!(r, "\n== Notes");
    for (k, v) in state.notes.iter().filter(|(k, _)| !k.starts_with("plan:")) {
        let _ = writeln!(r, "{k}: {}", short(v, 600));
    }
    std::fs::write(dir.join("profile.txt"), r)
}

/// The stage counters (`rerail_prof`): the device runs' host stages (CPU of the thread that ran
/// them), their GPU time (`gpu.device_us`), memory readings and counts. With
/// `RERAIL_GPU_KERNEL_TIMES` set, `kernel_us.<kernel>` splits the GPU time by kernel (each a
/// command buffer of its own: slower, for profiling only).
fn stages(r: &mut String) {
    use std::fmt::Write as _;
    let stages = rerail_prof::stages();
    let _ = writeln!(r, "\n== Stages (rerail-prof): CPU ms of the threads that ran each, its calls (or its count); kernel_us = GPU µs by kernel");
    if stages.is_empty() {
        r.push_str("(no stages)\n");
        return;
    }
    // Timed stages by CPU, then the counts by name.
    let mut timed: Vec<_> = stages.iter().filter(|(_, (d, _))| !d.is_zero()).collect();
    timed.sort_by(|a, b| b.1.0.cmp(&a.1.0));
    for (name, (d, calls)) in timed {
        let _ = writeln!(r, "{:>12.1} ms {calls:>9}  {name}", d.as_secs_f64() * 1e3);
    }
    let mut kernels: Vec<_> = stages.iter().filter(|(n, _)| n.starts_with("kernel_us.")).collect();
    kernels.sort_by(|a, b| b.1.1.cmp(&a.1.1));
    for (name, (_, us)) in &kernels {
        let k = &name["kernel_us.".len()..];
        let n = stages.get(&format!("kernel_n.{k}")).map_or(0, |e| e.1);
        let _ = writeln!(r, "{:>12.1} ms {n:>9}  GPU {k}", *us as f64 / 1e3);
    }
    // `RERAIL_GPU_WALK_STEPS`: the micro walks by their steps (a GPU thread's serial work), with
    // their walks and longest walk: a long walk is a dispatch's critical path.
    let mut walks: Vec<_> = stages.iter().filter_map(|(n, e)| n.strip_prefix("walk_steps.").map(|w| (w, e.1))).collect();
    if !walks.is_empty() {
        walks.sort_by(|a, b| b.1.cmp(&a.1));
        let total: usize = walks.iter().map(|w| w.1).sum();
        let _ = writeln!(r, "\n  micro walks by steps ({total} steps in all; t0 = 8 frames, t2 = the loop tier):");
        let _ = writeln!(r, "{:>14} {:>10} {:>10} {:>10}  walk words", "steps", "walks", "per walk", "longest");
        for (w, steps) in walks.iter().take(30) {
            let items = stages.get(&format!("walk_items.{w}")).map_or(0, |e| e.1);
            let max = stages.get(&format!("walk_max.{w}")).map_or(0, |e| e.1);
            let _ = writeln!(r, "{steps:>14} {items:>10} {:>10.1} {max:>10}  {w}", *steps as f64 / items.max(1) as f64);
        }
    }
    for (name, (d, count)) in stages.iter() {
        if d.is_zero() && !name.starts_with("kernel_") && !name.starts_with("walk_") {
            let _ = writeln!(r, "{:>12} {count:>12}  {name}", "count");
        }
    }
}
