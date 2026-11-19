//! The profile's roofline: for each span, the work it did in units of one kind (SQLite VM steps, native join rows,
//! source bytes parsed, nodes built, values measured, cells solved, plan candidates priced), and the best time this
//! machine gives one unit of that kind (its roof), measured when the profile is written by a short loop doing only
//! that unit's work. A span's roof time is its units times the roof; the rest of its own CPU is its excess, which
//! ranks where time goes beyond what the work needs. Beside it, a join's amplification (rows into its steps per row
//! it produced) shows work the plan makes beyond its output: a span can be near its roof and still do far more
//! work than its result needs.
//!
//! The roofs are operator roofs, not hardware ones: the loop for a VM step runs SQLite's cheapest statements, the
//! one for a join row probes a hash table of a million keys, the one for a node writes 32 bytes per node at memory
//! bandwidth. Each is the speed the engine could reach doing that work with no overhead around it.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::time::Instant;

/// A kind of work a span is charged with.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Unit {
    /// A SQLite VM step (charged from each statement's `SQLITE_STMTSTATUS_VM_STEP`).
    VmStep,
    /// A row into a step of a native join (`native::setfix` step counts).
    JoinRow,
    /// A byte of source parsed by tree-sitter.
    SourceByte,
    /// A node of a tape or batch built (encoded, projected, interned).
    Node,
    /// A value read to measure a relation's statistics.
    Value,
    /// A cell of a nearest-ancestor set solved.
    Cell,
    /// A candidate order the planner priced.
    Candidate,
    /// A byte of a source or tape read, hashed, compressed or written (content addressing).
    Byte,
    /// A row entered in an engine index or set when a component opens: a row loaded from SQLite into a component
    /// relation, or a row of an input or derived relation entered in a hash index for a join's lookup.
    IndexEntry,
    /// A literal of a rule (a body literal or a head) read by a transformation of the whole program: the demand
    /// split, the strata.
    Literal,
    /// A file's metadata read from the file system (the stat index's check).
    FileStat,
}

impl Unit {
    pub(super) const ALL: [Unit; 11] =
        [Unit::VmStep, Unit::JoinRow, Unit::SourceByte, Unit::Node, Unit::Value, Unit::Cell, Unit::Byte, Unit::IndexEntry, Unit::Candidate, Unit::Literal, Unit::FileStat];

    pub(super) fn name(self) -> &'static str {
        match self {
            Unit::VmStep => "VM step",
            Unit::JoinRow => "join row",
            Unit::SourceByte => "source byte",
            Unit::Node => "node",
            Unit::Value => "value",
            Unit::Cell => "cell",
            Unit::Candidate => "candidate",
            Unit::Byte => "byte",
            Unit::IndexEntry => "index entry",
            Unit::Literal => "literal",
            Unit::FileStat => "file stat",
        }
    }

    /// What the roof's loop does.
    fn roof_loop(self) -> &'static str {
        match self {
            Unit::VmStep => "SQLite, in memory: a filtered scan and an indexed join over 200k rows, the faster per step",
            Unit::JoinRow => "a probe of a hash table of 1M keys (foldhash), random keys",
            Unit::SourceByte => "none: tree-sitter is the roof (only a memo hit avoids it); its rate is shown",
            Unit::Node => "writing 32 bytes per node and interning one value of it in a hash table (1M nodes)",
            Unit::Value => "inserting values into a hash set of distinct values (1M, half repeated)",
            Unit::Cell => "reading and writing 8 bytes per cell, sequentially (memory bandwidth)",
            Unit::Candidate => "none: the planner's own rate is shown, with no roof",
            Unit::Byte => "SHA-256 of 64 MB in memory (the content key every stored byte needs)",
            Unit::IndexEntry => "entering 1M row ids of 2-column rows in a hash table (foldhash), not presized",
            Unit::Literal => "rewriting a program's body literals (atoms of 3 variables) node by node into a new image, as a pass that rebuilds it does",
            Unit::FileStat => "reading one file's metadata 20k times (the file system's cached path)",
        }
    }
}

/// Nanoseconds per unit at the roof, for the units a run charged work to (`None`: no roof). On a thread of the
/// highest quality of service, so that on a machine with performance and efficiency cores it runs where the engine's
/// busy threads do. Each unit is measured once per process (a run that makes runs, `corpus::world`'s rounds, writes a
/// profile per run), and once per engine binary and day across processes: the loops take seconds in a debug build,
/// which every profiled run paid before its report (O139). Returns where the roofs came from.
pub(super) fn calibrate(units: &std::collections::BTreeSet<Unit>) -> (BTreeMap<Unit, Option<f64>>, String) {
    static ROOFS: std::sync::LazyLock<std::sync::Mutex<BTreeMap<Unit, Option<f64>>>> = std::sync::LazyLock::new(Default::default);
    let mut known = ROOFS.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache_file();
    let mut source = "measured now";
    if units.iter().any(|u| !known.contains_key(u))
        && let Some(stored) = cache.as_deref().and_then(read_cache)
    {
        for (u, ns) in stored {
            known.entry(u).or_insert(ns);
        }
        source = "read from the roof cache";
    }
    let missing: Vec<Unit> = units.iter().copied().filter(|u| !known.contains_key(u)).collect();
    // Phase 0's run (`bootstrap.rr`, inside a run) measures none: its profile is a part of the run's, whose own report
    // measures them after every phase, so the loops (seconds in a debug build) are not charged to the run's loading.
    if !missing.is_empty() && crate::code_rule_dsl::phase_clock::active() {
        source = "not measured in phase 0's run (the run's own profile measures them)";
    } else if !missing.is_empty() {
        known.extend(measure(&missing));
        source = if missing.len() == units.len() { "measured now" } else { "partly measured now" };
        if let Some(path) = cache {
            write_cache(&path, &known);
        }
    }
    (units.iter().map(|u| (*u, known.get(u).copied().flatten())).collect(), source.to_owned())
}

/// The roof cache: `<temp>/rerail-roofs/<digest>.json`, per machine (its temporary directory) and day (`read_cache`).
/// The roofs are this machine running the loops below, so the digest is of what decides their speed: the loops'
/// code (this file's text), and how it was compiled (the build's profile, `target/<profile>`, and its debug
/// assertions). Keyed by the binary (its path, size and time), every rebuild measured them again: 10 s of CPU at the
/// next profiled run's report, a cold run's cost seen from outside (`life`).
fn cache_file() -> Option<std::path::PathBuf> {
    use sha2::Digest as _;
    let exe = std::env::current_exe().ok()?;
    let key = sha2::Sha256::digest(format!("{}\0{}\0{}", ROOF_LOOPS, build_profile(&exe), cfg!(debug_assertions)).as_bytes());
    let name: String = key.iter().take(8).map(|b| format!("{b:02x}")).collect();
    Some(std::env::temp_dir().join("rerail-roofs").join(format!("{name}.json")))
}

/// The loops' code: this file, as built.
const ROOF_LOOPS: &str = include_str!("roofline.rs");

/// The build's profile, by where Cargo put the binary: the directory under `target` (`debug`, `profiling`, …), or the
/// binary's own directory elsewhere.
fn build_profile(exe: &std::path::Path) -> String {
    let parts: Vec<String> = exe.parent().into_iter().flat_map(|p| p.components()).map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    match parts.iter().rposition(|c| c == "target") {
        Some(i) if i + 1 < parts.len() => parts[i + 1].clone(),
        _ => parts.last().cloned().unwrap_or_default(),
    }
}

/// A cache written within the last day, by unit name.
fn read_cache(path: &std::path::Path) -> Option<BTreeMap<Unit, Option<f64>>> {
    let age = std::fs::metadata(path).ok()?.modified().ok()?.elapsed().ok()?;
    if age > std::time::Duration::from_secs(24 * 3600) {
        return None;
    }
    let stored: BTreeMap<String, Option<f64>> = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    Some(Unit::ALL.iter().filter_map(|u| stored.get(u.name()).map(|ns| (*u, *ns))).collect())
}

fn write_cache(path: &std::path::Path, roofs: &BTreeMap<Unit, Option<f64>>) {
    let stored: BTreeMap<&str, Option<f64>> = roofs.iter().map(|(u, ns)| (u.name(), *ns)).collect();
    if let (Some(dir), Ok(text)) = (path.parent(), serde_json::to_string(&stored)) {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(path, text);
    }
}

fn measure(units: &[Unit]) -> BTreeMap<Unit, Option<f64>> {
    let units = units.to_vec();
    rerail_accel::exec::service("profile roofline", None, move || {
        // What the memory sampler names while the loops run (not a span: `write` holds the profile's records).
        let _doing = super::memory::Doing::new("profile: the machine's roofs measured (roofline, not the run's work)");
        #[cfg(target_os = "macos")]
        unsafe {
            unsafe extern "C" {
                fn pthread_set_qos_class_self_np(qos: u32, priority: i32) -> i32;
            }
            // QOS_CLASS_USER_INTERACTIVE
            pthread_set_qos_class_self_np(0x21, 0);
        }
        units.iter().map(|&u| (u, roof(u))).collect()
    })
    .ok()
    .and_then(|t| t.join().ok())
    .unwrap_or_default()
}

fn best(runs: usize, mut f: impl FnMut() -> (f64, u64)) -> f64 {
    (0..runs).map(|_| {
        let (s, n) = f();
        s * 1e9 / n.max(1) as f64
    })
    .fold(f64::INFINITY, f64::min)
}

fn roof(u: Unit) -> Option<f64> {
    match u {
        Unit::VmStep => {
            let db = rusqlite::Connection::open_in_memory().ok()?;
            db.execute_batch(
                "CREATE TABLE t(a INTEGER PRIMARY KEY, b INTEGER, c INTEGER);
                 WITH RECURSIVE s(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM s WHERE i < 200000)
                 INSERT INTO t SELECT i, i % 1000, i % 7 FROM s;
                 CREATE INDEX t_b ON t(b);",
            )
            .ok()?;
            let mut per = f64::INFINITY;
            for q in ["SELECT count(*) FROM t WHERE c = 3", "SELECT count(*) FROM t t1 JOIN t t2 ON t2.b = t1.a WHERE t1.a < 2000"] {
                let mut st = db.prepare(q).ok()?;
                per = per.min(best(5, || {
                    st.reset_status(rusqlite::StatementStatus::VmStep);
                    let t = Instant::now();
                    let _: i64 = st.query_row([], |r| r.get(0)).unwrap_or(0);
                    (t.elapsed().as_secs_f64(), st.get_status(rusqlite::StatementStatus::VmStep) as u64)
                }));
            }
            Some(per)
        }
        Unit::JoinRow => {
            let n = 1u64 << 20;
            let mut map: hashbrown::HashMap<u64, u32, foldhash::fast::FixedState> = hashbrown::HashMap::with_capacity_and_hasher(n as usize, Default::default());
            let mix = |x: u64| x.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29);
            for i in 0..n {
                map.insert(mix(i), i as u32);
            }
            Some(best(5, || {
                let t = Instant::now();
                let mut hit = 0u64;
                for i in 0..n {
                    hit += map.get(&mix(i.wrapping_mul(7919) % n)).copied().unwrap_or(0) as u64;
                }
                std::hint::black_box(hit);
                (t.elapsed().as_secs_f64(), n)
            }))
        }
        // A synthetic sample parses slower per byte than real code (fewer comments and blank lines), so it is no
        // roof for it.
        Unit::SourceByte => None,
        Unit::Node => {
            let n = 1usize << 20;
            let mut buf = vec![0u64; 4 * n];
            Some(best(5, || {
                let mut set: hashbrown::HashMap<u64, u32, foldhash::fast::FixedState> = hashbrown::HashMap::with_capacity_and_hasher(n / 4, Default::default());
                let t = Instant::now();
                for i in 0..n {
                    let v = ((i % (n / 4)) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
                    let len = set.len() as u32;
                    let id = *set.entry(v).or_insert(len);
                    buf[4 * i..4 * i + 4].copy_from_slice(&[i as u64, id as u64, (i / 2) as u64, v]);
                }
                std::hint::black_box((&buf, &set));
                (t.elapsed().as_secs_f64(), n as u64)
            }))
        }
        Unit::Cell => {
            let words = 1usize;
            let n = (64usize << 20) / 8;
            let mut buf = vec![0u64; n];
            Some(best(5, || {
                let t = Instant::now();
                for (i, w) in buf.iter_mut().enumerate() {
                    *w = i as u64;
                }
                std::hint::black_box(&buf);
                (t.elapsed().as_secs_f64(), (n / words) as u64)
            }))
        }
        Unit::Value => {
            let n = 1usize << 20;
            Some(best(5, || {
                let mut set: hashbrown::HashSet<u64, foldhash::fast::FixedState> = hashbrown::HashSet::with_capacity_and_hasher(n / 2, Default::default());
                let t = Instant::now();
                for i in 0..n as u64 {
                    set.insert((i % (n as u64 / 2)).wrapping_mul(0x9E37_79B9_7F4A_7C15));
                }
                std::hint::black_box(&set);
                (t.elapsed().as_secs_f64(), n as u64)
            }))
        }
        Unit::Byte => {
            use sha2::Digest;
            let buf = vec![7u8; 64 << 20];
            Some(best(5, || {
                let t = Instant::now();
                std::hint::black_box(sha2::Sha256::digest(&buf));
                (t.elapsed().as_secs_f64(), buf.len() as u64)
            }))
        }
        Unit::IndexEntry => {
            use std::hash::BuildHasher as _;
            let n = 1usize << 20;
            let rows: Vec<[i64; 2]> = (0..n as i64).map(|i| [i.wrapping_mul(0x9E37_79B9_7F4A_7C15u64 as i64), i & 1023]).collect();
            let state = foldhash::fast::FixedState::default();
            Some(best(5, || {
                let t = Instant::now();
                let mut table: hashbrown::HashTable<u32> = hashbrown::HashTable::new();
                for (id, row) in rows.iter().enumerate() {
                    let h = state.hash_one(row);
                    table.insert_unique(h, id as u32, |&i| state.hash_one(rows[i as usize]));
                }
                std::hint::black_box(&table);
                (t.elapsed().as_secs_f64(), n as u64)
            }))
        }
        Unit::Candidate => None,
        Unit::Literal => {
            // 2^16 body literals (atoms of 3 variables) in the image's layout: a node's kind, size and value, in three
            // columns (`program_image::Image::node`), 4 nodes a literal. A pass that rebuilds the program writes each
            // node into a new image's columns, as this does.
            let n = 1usize << 16;
            let kinds: Vec<u8> = (0..n * 4).map(|i| if i % 4 == 0 { 1 } else { 2 }).collect();
            let sizes: Vec<u32> = (0..n * 4).map(|i| if i % 4 == 0 { 4 } else { 1 }).collect();
            let vals: Vec<u32> = (0..n * 4).map(|i| if i % 4 == 0 { (i / 4 % 97) as u32 } else { (i % 7) as u32 }).collect();
            Some(best(3, || {
                let t = Instant::now();
                let (mut k, mut z, mut v) = (Vec::new(), Vec::new(), Vec::new());
                for i in 0..kinds.len() {
                    k.push(kinds[i]);
                    z.push(sizes[i]);
                    v.push(vals[i]);
                }
                std::hint::black_box((k, z, v));
                (t.elapsed().as_secs_f64(), n as u64)
            }))
        }
        Unit::FileStat => {
            let path = std::env::current_exe().ok()?;
            let n = 20_000u64;
            Some(best(3, || {
                let t = Instant::now();
                for _ in 0..n {
                    std::hint::black_box(std::fs::metadata(&path).ok());
                }
                (t.elapsed().as_secs_f64(), n)
            }))
        }
    }
}

/// One span's line of the roofline.
struct Line {
    path: String,
    own: f64,
    units: BTreeMap<Unit, u64>,
    roof_s: f64,
    /// Native joins: rows into their steps, and rows out of their last steps.
    join: Option<(u64, u64)>,
}

/// The roofline section of the report. `work` holds what was charged by span; `own` each span's own CPU.
/// Returns the calibration's wall and CPU (the process's, every thread: its loops run on threads of their own) and
/// where its roofs came from: the report's own work, after the CPU the spans account for (`life`).
pub fn report(r: &mut String, own: &BTreeMap<String, f64>, work: &BTreeMap<(String, Unit), u64>, joins: &BTreeMap<String, (u64, u64)>) -> (f64, f64, String) {
    let t = Instant::now();
    let cpu0 = super::life::cpu(false);
    let charged: std::collections::BTreeSet<Unit> = work.keys().map(|(_, u)| *u).collect();
    let (roofs, source) = calibrate(&charged);
    let calibration = t.elapsed().as_secs_f64();
    let calibration_cpu = super::life::cpu(false) - cpu0;
    let load = std::fs::read_to_string("/proc/loadavg").ok().or_else(|| {
        std::process::Command::new("sysctl").args(["-n", "vm.loadavg"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    });
    let _ = writeln!(
        r,
        "\n== Roofline: each span's work against the best this machine does that work (its roof); excess = own CPU less units x roof"
    );
    let _ = writeln!(r, "Roofs of the {} kinds of work charged, {source} in {calibration:.2} s ({calibration_cpu:.2} s CPU){}:", roofs.len(), load.map(|l| format!(" (load {l})")).unwrap_or_default());
    for (u, ns) in &roofs {
        match ns {
            Some(ns) => {
                let _ = writeln!(r, "  {:<12} {:>9.2} ns  {}", u.name(), ns, u.roof_loop());
            }
            None => {
                let _ = writeln!(r, "  {:<12} {:>9}     {}", u.name(), "-", u.roof_loop());
            }
        }
    }
    let mut lines: BTreeMap<String, Line> = BTreeMap::new();
    for ((path, u), &n) in work {
        let l = lines.entry(path.clone()).or_insert_with(|| Line { path: path.clone(), own: own.get(path).copied().unwrap_or(0.0), units: BTreeMap::new(), roof_s: 0.0, join: None });
        *l.units.entry(*u).or_default() += n;
        l.roof_s += roofs.get(u).copied().flatten().map_or(0.0, |ns| ns * n as f64 / 1e9);
    }
    for (path, &io) in joins {
        if let Some(l) = lines.get_mut(path) {
            l.join = Some(io);
        }
    }
    let all: f64 = own.values().sum();
    let counted: f64 = lines.values().map(|l| l.own).sum();
    let roof_all: f64 = lines.values().map(|l| l.roof_s.min(l.own)).sum();
    let _ = writeln!(
        r,
        "Own CPU {all:.2} s; {counted:.2} s ({:.0}%) in spans with a work counter, whose roof time is {roof_all:.2} s: {:.0}% of their CPU",
        100.0 * counted / all.max(1e-9),
        100.0 * roof_all / counted.max(1e-9)
    );
    // By kind of work: units, CPU of the spans charged mostly with it, attained against the roof.
    let _ = writeln!(r, "\nBy kind of work (a span counts under the kind that is most of its roof time):");
    let _ = writeln!(r, "{:<12} {:>14} {:>9} {:>11} {:>9} {:>8} {:>9}", "kind", "units", "own s", "ns/unit", "roof ns", "of roof", "excess s");
    let mut by: BTreeMap<Unit, (u64, f64, f64)> = BTreeMap::new();
    for l in lines.values() {
        let main = l.units.iter().max_by(|a, b| {
            let w = |(u, n): (&Unit, &u64)| roofs.get(u).copied().flatten().unwrap_or(0.0) * *n as f64;
            w(*a).total_cmp(&w(*b))
        });
        if let Some((u, _)) = main {
            let e = by.entry(*u).or_default();
            e.0 += l.units[u];
            e.1 += l.own;
            e.2 += l.roof_s;
        }
    }
    for (u, (n, own, roof_s)) in &by {
        let ns = own * 1e9 / (*n).max(1) as f64;
        let roof = roofs.get(u).copied().flatten();
        let _ = writeln!(
            r,
            "{:<12} {:>14} {:>9.2} {:>11.2} {:>9} {:>8} {:>9.2}",
            u.name(),
            n,
            own,
            ns,
            roof.map_or("-".into(), |x| format!("{x:.2}")),
            roof.map_or("-".into(), |x| format!("{:.1}%", 100.0 * x / ns.max(1e-9))),
            own - roof_s
        );
    }
    // Spans by excess.
    let mut ranked: Vec<&Line> = lines.values().filter(|l| l.own > all * 0.002).collect();
    ranked.sort_by(|a, b| (b.own - b.roof_s).total_cmp(&(a.own - a.roof_s)));
    let _ = writeln!(r, "\nSpans by excess CPU (spans with a work counter, over 0.2% of the run; the first 40):");
    let _ = writeln!(r, "{:>9} {:>9} {:>8} {:>9}  {:<44} span", "own s", "roof s", "of roof", "excess s", "work (units; join rows in -> out, amplification)");
    for l in ranked.iter().take(40) {
        let units: Vec<String> = l.units.iter().map(|(u, n)| format!("{} {}", n, u.name())).collect();
        let join = l.join.map(|(i, o)| format!("; {i} -> {o}, x{:.0}", i as f64 / o.max(1) as f64)).unwrap_or_default();
        let _ = writeln!(
            r,
            "{:>9.3} {:>9.3} {:>7.1}% {:>9.3}  {:<44} {}",
            l.own,
            l.roof_s,
            100.0 * l.roof_s / l.own.max(1e-9),
            l.own - l.roof_s,
            format!("{}{join}", units.join(", ")),
            l.path
        );
    }
    // Joins whose work most exceeds their output.
    let mut amp: Vec<(&String, &(u64, u64))> = joins.iter().filter(|(_, (i, _))| *i > 100_000).collect();
    amp.sort_by(|a, b| (b.1.0 as f64 / b.1.1.max(1) as f64).total_cmp(&(a.1.0 as f64 / a.1.1.max(1) as f64)));
    let _ = writeln!(r, "\nNative joins by amplification (rows into steps per row out; over 100k rows in; the first 20):");
    for (path, (i, o)) in amp.iter().take(20) {
        let _ = writeln!(r, "  x{:<9.0} {:>11} -> {:<10} {}", *i as f64 / (*o).max(1) as f64, i, o, path);
    }
    // CPU no counter explains.
    let mut bare: Vec<(&String, &f64)> = own.iter().filter(|(p, s)| !lines.contains_key(*p) && **s > all * 0.002).collect();
    bare.sort_by(|a, b| b.1.total_cmp(a.1));
    let bare_all: f64 = own.iter().filter(|(p, _)| !lines.contains_key(*p)).map(|(_, s)| s).sum();
    let _ = writeln!(r, "\nOwn CPU with no work counter: {bare_all:.2} s in all; spans over 0.2% of the run:");
    for (p, s) in bare.iter().take(25) {
        let _ = writeln!(r, "  {s:>8.3} s  {p}");
    }
    (calibration, calibration_cpu, format!("the roofline's roofs, {source}"))
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// The literal's roof is measured on the image's node layout: a positive cost per literal.
    #[test]
    fn the_literal_roof_rewrites_image_nodes() {
        let per = roof(Unit::Literal).expect("the literal's roof");
        assert!(per.is_finite() && per > 0.0, "{per}");
    }

    /// Roofs written to the cache read back by unit name, `None` included; an unknown name is skipped.
    #[test]
    fn the_roof_cache_reads_back_what_it_wrote() {
        let path = std::env::temp_dir().join(format!("rerail-roofs-test-{}.json", std::process::id()));
        let roofs: BTreeMap<Unit, Option<f64>> = [(Unit::JoinRow, Some(3.5)), (Unit::SourceByte, None)].into();
        write_cache(&path, &roofs);
        assert_eq!(read_cache(&path), Some(roofs));
        std::fs::write(&path, r#"{"no such unit": 1.0}"#).unwrap();
        assert_eq!(read_cache(&path), Some(BTreeMap::new()));
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod cache_tests {
    /// A rebuild keeps the roofs: the cache is named by the loops and the build's profile, not by the binary.
    #[test]
    fn the_roof_cache_is_the_builds_profile_not_its_binary() {
        let at = |p: &str| super::build_profile(std::path::Path::new(p));
        assert_eq!(at("/w/target/debug/rerail-engine"), "debug");
        assert_eq!(at("/w/target/debug/deps/rerail_engine-56c28bf847ab8017"), "debug");
        assert_eq!(at("/w/target/profiling/rerail-engine"), "profiling");
        assert_eq!(at("/w/tmp/cold/engine-copy"), "cold");
        assert!(super::ROOF_LOOPS.contains("fn measure("));
    }
}
