//! The `scalar` engine: setfix's scalar joins over a captured launch's strata, in the engine's own run of them.
//!
//! A stratum's views are read through its chunk's tape batch, which only the corpus pipeline makes, from the
//! compiled policy, its phases and its typing: the batch is made as the engine makes it by running the engine. So the
//! capture's run (`run.txt`: its working directory, arguments and `RERAIL_` settings) is run again, linked into this
//! binary (`join-lab rerun`, a process of its own, as each setting is read once at its first use), with the scalar
//! joins on the host (`RERAIL_GPU=cpu`, `RERAIL_CPU_JOINS=scalar`), over the run's stores (the tape cache, the image
//! store) but a costs store of its own, one thread as the capture's run had, and its outputs in a directory of its own.
//! Each stratum a captured launch ran is known in it by its identity across runs (`setfix::replay`), and its record
//! (each relation's live rows and their digest) is held to the capture's (`strata.txt`). A launch is the same where
//! every stratum it ran made the same rows; its CPU is its strata's runs', each one's whole (every wave of it), and
//! its instructions and cycles theirs, counted on the thread while each ran. `--sample` samples them alone.

use crate::{Wave, counters, sample};
use rerail_setfix::code_rule_dsl::logic::native::setfix::replay::{self, Record};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The capture's run (`run.txt`).
pub struct RunOf {
    pub cwd: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl RunOf {
    pub fn read(capture: &Path) -> Result<RunOf, String> {
        let path = capture.join("run.txt");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e} (a capture made before its run was recorded)", path.display()))?;
        let mut run = RunOf { cwd: PathBuf::new(), args: Vec::new(), env: Vec::new() };
        for line in text.lines() {
            match line.split_once(' ') {
                Some(("cwd", d)) => run.cwd = PathBuf::from(d),
                Some(("arg", a)) => run.args.push(a.to_owned()),
                Some(("env", kv)) => {
                    let (k, v) = kv.split_once('=').ok_or_else(|| format!("{}: {line}", path.display()))?;
                    run.env.push((k.to_owned(), v.to_owned()));
                }
                _ => return Err(format!("{}: {line}", path.display())),
            }
        }
        if run.args.len() < 2 {
            return Err(format!("{}: no command", path.display()));
        }
        Ok(run)
    }

    /// Its settings as the scalar run has them: the profile's and the catalogue's off (they would write the capture's
    /// own), a costs store of its own (the capture's run started from none), the scalar joins on the host, its strata
    /// replayed to `out`.
    fn env_for(&self, capture: &Path, out: &Path) -> Vec<(String, String)> {
        const OFF: [&str; 6] = ["RERAIL_PROFILE", "RERAIL_JOIN_SHAPES", "RERAIL_COSTS", "RERAIL_GPU", "RERAIL_CPU_JOINS", "RERAIL_JOIN_REPLAY"];
        let mut env: Vec<(String, String)> = self.env.iter().filter(|(k, _)| !OFF.iter().any(|o| k.starts_with(o))).cloned().collect();
        if !env.iter().any(|(k, _)| k == "RERAIL_THREADS") {
            // As a capturing run has it (`rerail_context::threads`): its strata in one order.
            env.push(("RERAIL_THREADS".into(), "1".into()));
        }
        let path = |p: PathBuf| p.to_string_lossy().into_owned();
        env.extend([
            ("RERAIL_COSTS".into(), path(out.join("costs.sqlite"))),
            ("RERAIL_GPU".into(), "cpu".into()),
            ("RERAIL_CPU_JOINS".into(), "scalar".into()),
            ("RERAIL_JOIN_REPLAY".into(), path(capture.to_path_buf())),
            ("RERAIL_JOIN_REPLAY_OUT".into(), path(out.join("replay.txt"))),
        ]);
        // The capture's run traced its strata's identities (`RERAIL_JOIN_REPLAY_TRACE`): this one traces its own beside.
        if self.env.iter().any(|(k, _)| k == "RERAIL_JOIN_REPLAY_TRACE") {
            env.push(("RERAIL_JOIN_REPLAY_TRACE".into(), path(out.join("opened.txt"))));
        }
        env
    }

    /// Its arguments as the scalar run has them: its command's, its last (its out directory) `out`'s.
    fn args_for(&self, out: &Path) -> Vec<String> {
        let mut args = self.args[1..].to_vec();
        if let Some(last) = args.last_mut() {
            *last = out.join("out").to_string_lossy().into_owned();
        }
        args
    }
}

/// The scalar run's records, by stratum, its strata's instructions and cycles, its samples folded.
pub struct Rerun {
    pub records: HashMap<u64, Record>,
    pub counts: HashMap<u64, (u64, u64)>,
    pub folded: Option<String>,
    /// The whole run's wall seconds.
    pub seconds: f64,
}

/// The capture's run again with the scalar joins (`join-lab rerun`, this binary), its records read back.
pub fn rerun(capture: &Path, sample: bool) -> Result<Rerun, String> {
    let run = RunOf::read(capture)?;
    let out = capture.join("scalar");
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    // The user's alone, as the engine's launch checks the folders it maps files from (`tools::launch_checks`).
    rerail_blocks::mapped::make_folder(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let log = std::fs::File::create(out.join("log.txt")).map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("rerun").arg(capture).arg(&out);
    if sample {
        cmd.arg("--sample");
    }
    cmd.arg("--").args(run.args_for(&out));
    for (k, _) in std::env::vars().filter(|(k, _)| k.starts_with("RERAIL_")) {
        cmd.env_remove(k);
    }
    cmd.envs(run.env_for(capture, &out)).current_dir(&run.cwd).stdout(std::process::Stdio::null()).stderr(log);
    let start = std::time::Instant::now();
    let status = cmd.status().map_err(|e| format!("the scalar run: {e}"))?;
    let seconds = start.elapsed().as_secs_f64();
    if !status.success() {
        let log = std::fs::read_to_string(out.join("log.txt")).unwrap_or_default();
        let tail: Vec<&str> = log.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(format!("the scalar run failed ({status}): {}", tail.join(" | ")));
    }
    let records = replay::records(&std::fs::read_to_string(out.join("replay.txt")).unwrap_or_default());
    let counts = std::fs::read_to_string(out.join("counts.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let mut w = l.split(' ');
            Some((u64::from_str_radix(w.next()?, 16).ok()?, (w.next()?.parse().ok()?, w.next()?.parse().ok()?)))
        })
        .collect();
    let folded = std::fs::read_to_string(out.join("scalar.folded")).ok();
    Ok(Rerun { records, counts, folded, seconds })
}

/// The scalar run itself (`join-lab rerun <capture> <out> [--sample] -- <args>`, in the run's working directory): the engine's command as its
/// binary starts it (`rerail-engine`'s main), its strata watched on this thread (`replay::watch`): each one's
/// instructions and cycles to `<out>/counts.txt`, its samples to `<out>/scalar.folded`.
pub fn rerun_main(args: &[String]) -> Result<(), String> {
    let [_capture, out, rest @ ..] = args else { return Err("usage: join-lab rerun <capture> <out> [--sample] -- <args>".into()) };
    let sample = rest.first().is_some_and(|a| a == "--sample");
    let command: Vec<String> = rest.iter().skip_while(|a| *a != "--").skip(1).cloned().collect();
    rerail_engine::tools::watch();
    rerail_engine::tools::launch_checks()?;
    rerail_engine::threads::configure();
    let machine = rerail_engine::threads::Machine::here();
    let sampler = if sample { Some(std::rc::Rc::new(sample::Sampler::start(std::time::Duration::from_millis(1))?)) } else { None };
    let counted: std::rc::Rc<std::cell::RefCell<HashMap<u64, (u64, u64)>>> = Default::default();
    {
        let (counters, counted, sampler) = (counters::Counters::open(), std::rc::Rc::clone(&counted), sampler.clone());
        let mut from = counters::Counts::default();
        replay::watch(Some(Box::new(move |stratum, start| {
            let now = counters.as_ref().map_or(counters::Counts::default(), counters::Counters::read);
            if start {
                from = now;
                if let Some(s) = &sampler {
                    s.tag(1);
                }
            } else {
                if let Some(s) = &sampler {
                    s.tag(0);
                }
                let d = now - from;
                let mut c = counted.borrow_mut();
                let e = c.entry(stratum).or_default();
                e.0 += d.instructions;
                e.1 += d.cycles;
            }
        })));
    }
    let (result, _) = rerail_engine::tools::cli(&command, &machine);
    replay::watch(None);
    result?;
    let out = Path::new(out);
    let counts: String = counted.borrow().iter().map(|(s, (i, c))| format!("{s:016x} {i} {c}\n")).collect();
    std::fs::write(out.join("counts.txt"), counts).map_err(|e| e.to_string())?;
    if let Some(s) = sampler {
        let s = std::rc::Rc::try_unwrap(s).map_err(|_| "the sampler is still held")?;
        let by = sample::fold(&s.finish())?;
        let folded: String = by.get(&1).map_or(String::new(), |v| v.iter().map(|(p, n)| format!("{p} {n}\n")).collect());
        std::fs::write(out.join("scalar.folded"), folded).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A launch on the scalar engine: whether every stratum it ran made the capture's rows (else the first that did
/// not), and its strata's CPU, instructions and cycles.
pub struct Scalar {
    pub same: bool,
    pub differing: String,
    pub strata: usize,
    pub cpu: f64,
    pub instructions: u64,
    pub cycles: u64,
}

/// The capture's records (`strata.txt`).
pub fn captured(capture: &Path) -> Result<HashMap<u64, Record>, String> {
    let path = capture.join("strata.txt");
    std::fs::read_to_string(&path).map(|t| replay::records(&t)).map_err(|e| format!("{}: {e} (a capture made before strata were recorded)", path.display()))
}

/// Launch `w` held to the capture's records of its strata (`captured`) by the scalar run's (`rerun`).
pub fn of(w: &Wave, captured: &HashMap<u64, Record>, rerun: &Rerun) -> Result<Scalar, String> {
    let mut strata: Vec<u64> = w.labels.iter().map(|l| l.stratum).filter(|&s| s != 0).collect();
    strata.sort_unstable();
    strata.dedup();
    if strata.is_empty() {
        return Err("its labels name no stratum (a capture made before strata were identified)".into());
    }
    let mut s = Scalar { same: true, differing: String::new(), strata: strata.len(), cpu: 0.0, instructions: 0, cycles: 0 };
    for id in strata {
        let want = captured.get(&id).ok_or_else(|| format!("stratum {id:016x}: no record in the capture (strata.txt)"))?;
        match rerun.records.get(&id) {
            None => {
                if s.same {
                    s.differing = format!("stratum {id:016x} ({}): the scalar run did not reach it", want.name);
                }
                s.same = false;
            }
            Some(got) => {
                s.cpu += got.cpu;
                if let Some(&(i, c)) = rerun.counts.get(&id) {
                    s.instructions += i;
                    s.cycles += c;
                }
                if let Some(d) = want.differs(got).filter(|_| s.same) {
                    s.differing = format!("stratum {id:016x} ({}): the capture's {d} (the scalar run's)", want.name);
                    s.same = false;
                }
            }
        }
    }
    Ok(s)
}
