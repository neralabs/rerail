//! The profile accounts for every CPU second: each thread's CPU, under its spans and outside them.
//! They read the engine (a temporary directory, the report), which the profile's own crate cannot build.

use crate::code_rule_dsl::logic::profile::{self, report};

/// Spins until this thread has taken `seconds` of CPU, not of wall time: on a loaded machine a wall-time spin gets a
/// fraction of its time on a core (0.08 s of 0.15 s at load 64), and the assertions below are of CPU.
fn burn(seconds: f64) {
    let start = thread_cpu();
    let mut x = 0u64;
    while thread_cpu() - start < seconds {
        x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
    }
}

/// The calling thread's CPU seconds.
#[cfg(unix)]
fn thread_cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: fills `t` with the calling thread's CPU time.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
}

/// Without a thread clock, the wall clock's seconds since first asked.
#[cfg(not(unix))]
fn thread_cpu() -> f64 {
    static START: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);
    START.elapsed().as_secs_f64()
}

/// Work a span starts on threads that open no span of their own (rayon's, a helper thread) is in the report: the
/// threads' CPU adds up to the process's, and what no span covered is named by thread.
#[test]
fn the_profile_accounts_for_every_threads_cpu() {
    // The test's own run, profiled (decision 0442: the switch is its caller's, the records its run's).
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    profile::set(true);
    let (park, parked) = std::sync::mpsc::channel::<()>();
    let (burnt, done) = std::sync::mpsc::channel::<()>();
    let helper = std::thread::Builder::new()
        .name("xhelper".into())
        .spawn(move || {
            burn(0.15);
            let _ = burnt.send(());
            let _ = parked.recv();
        })
        .unwrap();
    {
        let _s = profile::span(|| "xthreads".into());
        use rayon::prelude::*;
        (0..64).into_par_iter().for_each(|_| burn(0.005));
    }
    // The helper's CPU is all taken before the account (a wall-time wait fell short at load 90: 0.05 s of 0.15).
    done.recv().unwrap();
    let account = profile::threads();
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("threads");
    report::write(&dir, "header\n").unwrap();
    let text = std::fs::read_to_string(dir.join("profile.txt")).unwrap();
    let tsv = std::fs::read_to_string(dir.join("threads.tsv")).unwrap();
    profile::set(false);
    drop(park);
    helper.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(text.contains("== CPU by thread"), "{text}");
    let helper = account.rows.iter().find(|t| t.name == "xhelper").expect("the helper thread is listed");
    assert!(helper.cpu > 0.1 && helper.covered < 0.01, "the helper's CPU, none under a span: {} {}", helper.cpu, helper.covered);
    let listed: f64 = account.rows.iter().map(|t| t.cpu).sum();
    assert!(listed <= account.process * 1.02 + 0.01 && listed >= account.process * 0.9, "threads {listed} s of the process's {} s", account.process);
    assert!(tsv.starts_with("thread\tcpu_s\tunder_spans_s\toutside_spans_s\n") && tsv.contains("xhelper\t"), "{tsv}");
}

/// CPU a thread takes between its spans is named by where it falls: after the span that closed, before the one that
/// opened next, so CPU outside every span points at the code that runs it.
#[test]
fn cpu_between_spans_is_named_by_the_spans_around_it() {
    // The test's own run, profiled (decision 0442: the switch is its caller's, the records its run's).
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    profile::set(true);
    let run = profile::run_own();
    let t = std::thread::Builder::new()
        .name("xgaps".into())
        .spawn(move || {
            // In the test's run, as a job of it is.
            let _in = profile::enter(run);
            // A thread of its own, profiled as its caller is (the switch is a caller's, carried to its jobs).
            profile::set(true);
            drop(profile::span(|| "xgap before".into()));
            burn(0.1);
            drop(profile::span(|| "xgap after".into()));
        })
        .unwrap();
    t.join().unwrap();
    let gaps = profile::gaps();
    let dir = crate::code_rule_dsl::fixtures::temp().with_extension("gaps");
    report::write(&dir, "header\n").unwrap();
    // gaps.tsv has every gap; profile.txt only the largest 12, which other tests' runs in this process may fill.
    let text = std::fs::read_to_string(dir.join("gaps.tsv")).unwrap();
    profile::set(false);
    let _ = std::fs::remove_dir_all(&dir);
    let between = gaps.iter().find(|g| g.after == "xgap before" && g.before == "xgap after").expect("the gap between the spans");
    assert!(between.cpu > 0.08, "{} s between the spans", between.cpu);
    assert!(text.lines().any(|l| l.starts_with("xgap before\txgap after\t")), "{text}");
}

/// Decision 0288: the run's thread's time is charged to the span innermost when it passed, phase by phase. A span's
/// sleep is its wall off a core, its burn its CPU, its child's time is the child's own, and a phase's time is the sum
/// of what it charged.
#[test]
fn the_run_threads_time_is_charged_to_its_innermost_span_by_phase() {
    let _run = crate::code_rule_dsl::logic::run::scope(crate::code_rule_dsl::logic::native::width::Settings::from_env);
    profile::set(true);
    profile::track_run_thread();
    {
        let _outer = profile::span(|| "xouter".into());
        std::thread::sleep(std::time::Duration::from_millis(60));
        {
            let _inner = profile::span(|| "xinner".into());
            burn(0.03);
        }
    }
    profile::run_thread_phase("xfirst");
    std::thread::sleep(std::time::Duration::from_millis(20));
    profile::run_thread_phase("xsecond");
    profile::untrack_run_thread();
    let clock = profile::PhaseClock { wall: 0.2, own: 0.05, helped: 0.0, process_cpu: 0.1, runnable: 0.15, involuntary: 3 };
    let text = profile::run_thread_report(&[("xfirst", clock)]);
    profile::set(false);
    let row = |phase: &str, path: &str| -> Vec<f64> {
        let block = text.split("\n-- ").find(|b| b.starts_with(phase)).unwrap_or_else(|| panic!("phase {phase}: {text}"));
        let line = block.lines().find(|l| l.ends_with(&format!("  {path}"))).unwrap_or_else(|| panic!("{path} in {phase}: {block}"));
        line.split_whitespace().take(3).map(|x| x.parse().unwrap()).collect()
    };
    // xouter's own: its sleep, off a core; xinner's: its burn, on one.
    let (outer, inner) = (row("xfirst", "xouter"), row("xfirst", "xouter/xinner"));
    assert!(outer[0] >= 0.06 && outer[1] < 0.02 && outer[2] >= 0.05, "xouter: wall, cpu, off {outer:?}\n{text}");
    assert!(inner[1] >= 0.03 && inner[0] >= inner[1], "xinner: wall, cpu, off {inner:?}\n{text}");
    // The second phase is its sleep, outside every span: none of the first phase's time is in it.
    let outside = row("xsecond", "(outside every span)");
    assert!(outside[0] >= 0.02 && outside[0] < 0.06, "xsecond: {outside:?}\n{text}");
    // The table: the phase's clocks, and what is derived from them.
    let first = text.lines().find(|l| l.starts_with("xfirst ")).expect("the phase's row");
    let cols: Vec<&str> = first.split_whitespace().collect();
    assert_eq!(&cols[1..], &["0.200", "0.050", "0.000", "0.150", "0.050", "0.50", "0.050", "3"], "{first}");
}
