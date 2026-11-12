//! The profile accounts for every CPU second: each thread's CPU, under its spans and outside them.
//! With the profile's tests that read the engine (a `.rr` program, the report, a temporary directory), which the
//! profile's own crate cannot build.

use crate::code_rule_dsl::logic::profile::{self, report};

fn burn(seconds: f64) {
    let start = std::time::Instant::now();
    let mut x = 0u64;
    while start.elapsed().as_secs_f64() < seconds {
        x = std::hint::black_box(x.wrapping_mul(6364136223846793005).wrapping_add(1));
    }
}

/// Work a span starts on threads that open no span of their own (rayon's, a helper thread) is in the report: the
/// threads' CPU adds up to the process's, and what no span covered is named by thread.
#[test]
fn the_profile_accounts_for_every_threads_cpu() {
    profile::set(true);
    let (park, parked) = std::sync::mpsc::channel::<()>();
    let helper = std::thread::Builder::new()
        .name("xhelper".into())
        .spawn(move || {
            burn(0.15);
            let _ = parked.recv();
        })
        .unwrap();
    {
        let _s = profile::span(|| "xthreads".into());
        use rayon::prelude::*;
        (0..64).into_par_iter().for_each(|_| burn(0.005));
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    let account = profile::threads();
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("threads");
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
    profile::set(true);
    let t = std::thread::Builder::new()
        .name("xgaps".into())
        .spawn(|| {
            drop(profile::span(|| "xgap before".into()));
            burn(0.1);
            drop(profile::span(|| "xgap after".into()));
        })
        .unwrap();
    t.join().unwrap();
    let gaps = profile::gaps();
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("gaps");
    report::write(&dir, "header\n").unwrap();
    let text = std::fs::read_to_string(dir.join("profile.txt")).unwrap();
    profile::set(false);
    let _ = std::fs::remove_dir_all(&dir);
    let between = gaps.iter().find(|g| g.after == "xgap before" && g.before == "xgap after").expect("the gap between the spans");
    assert!(between.cpu > 0.08, "{} s between the spans", between.cpu);
    assert!(text.contains("xgap before -> xgap after"), "{text}");
}

/// The report names each stratum's engine and plan, the device kernels, the decisions, the strata that fell back to
/// SQLite and what kept each off Metal and the native engine, and the SQL that waited inside the waves.
#[test]
fn explain_shows_engines_plans_devices_and_decisions() {
    let _serial = profile::one_run();
    profile::set(true);
    profile::reset();
    profile::decision("xtopic", || "2 chunks".into());
    profile::stratum("xstratum", "native set engine", || "every rule has a native step".into());
    profile::stratum("xstratum", "native set engine", String::new);
    let plan = profile::RulePlan {
        work: 10.0,
        rows: 2.0,
        subsets: 4,
        candidates: 6,
        steps: vec![
            profile::StepPlan { literal: "a($x)".into(), access: "scan".into(), source: "input".into(), rows: 5.0, passed: Vec::new() },
            profile::StepPlan { literal: "b($x)".into(), access: "lookup on (c0)".into(), source: "derived".into(), rows: 2.0, passed: vec![("c($x)".into(), 7.0)] },
        ],
    };
    profile::planned("xstratum", "seed xrule", Some(3), &plan, 0.01, 0, Some(&[(1, 5), (5, 2)]), None);
    profile::planned("xstratum", "seed xrule", Some(3), &plan, 0.01, 1, Some(&[(1, 4), (4, 1)]), None);
    {
        let _s = profile::span(|| "xspan".into());
        profile::device("xkernel", 1, 0.002, 0.003, 100);
    }
    // A stratum that fell back to SQLite, and its span.
    profile::stratum("xsql", "SQL wave kernel", || "the native engine declined".into());
    profile::fallback("xsql", "aggregate join (string concatenation)".into());
    profile::executor("xsql", "SQLite", Some("aggregate join (string concatenation)".into()), vec!["string join: string join".into()]);
    profile::executor("xstratum", "native CPU", None, vec!["computed term: @sql_instr in a comparison".into()]);
    profile::executor("xstratum", "Metal", None, Vec::new());
    drop(profile::span(|| "stratum xsql".into()));
    // SQL before the waves is the run's beginning; inside them, from a wave's thread, a hazard; from the store's
    // writer (behind the run), none.
    profile::sql_in_wave("run", "SELECT 1 FROM xbefore", 1_000);
    {
        let _w = profile::waves();
        let _s = profile::span(|| "xwave".into());
        profile::sql_in_wave("store", "BEGIN IMMEDIATE", 2_000_000);
        std::thread::spawn(|| {
            profile::behind_the_run();
            profile::sql_in_wave("store", "INSERT INTO xbehind VALUES (1)", 1_000);
        })
        .join()
        .unwrap();
    }
    profile::sql_in_wave("run", "SELECT 1 FROM xafter", 1_000);
    let dir = crate::code_rule_dsl::prefilter::tests::temp().with_extension("explain");
    report::write(&dir, "header\n").unwrap();
    let report = std::fs::read_to_string(dir.join("profile.txt")).unwrap();
    profile::set(false);
    assert!(report.contains("Stratum xstratum  [engine: native set engine]  (loops=2"), "{report}");
    assert!(report.contains("why: every rule has a native step"));
    assert!(report.contains("== Fell back to SQLite: 1 strata"), "{report}");
    // Other tests' strata may be recorded beside these (the profile is the process's): the heading alone.
    assert!(report.contains("== Executor hazards: what kept each stratum off Metal and off the native engine ("), "{report}");
    assert!(report.contains("native: aggregate join (string concatenation)") && report.contains("metal: computed term: @sql_instr in a comparison"), "{report}");
    assert!(report.contains("xstratum: Metal x1, native CPU x1; metal: computed term: @sql_instr in a comparison"), "{report}");
    assert!(report.contains("== Forces SQL waits (hazards): 1 statements, 1 runs, 2.0 ms inside the waves"), "{report}");
    assert!(report.contains("a write to the store on a wave's thread") && report.contains("xwave"), "{report}");
    assert!(!report.contains("xbefore") && !report.contains("xafter") && !report.contains("xbehind"), "{report}");
    assert!(report.contains("xsql: aggregate join (string concatenation)"), "{report}");
    assert!(report.contains("rule #0003 (seed xrule)") && report.contains("loops=2 replans=1"));
    assert!(report.contains("passed over c($x)") && report.contains("7.0 rows a binding"), "the joins a step passed over: {report}");
    // Actual rows add over the loops.
    assert!(report.contains("actual 9 -> 3"), "{report}");
    assert!(report.contains("xkernel  (xspan)"));
    assert!(report.contains("xtopic:\n  - 2 chunks"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A scalar input relation's projection provenance follows its readers to an output.
#[test]
fn projection_reason_traces_real_consumers_and_ignores_dead_scalars() {
    let _lock = profile::one_run();
    profile::reset();
    profile::set(true);
    let p = crate::code_rule_dsl::logic::rr::parse(
        r#"
#include "core.rr"
.decl selected(n:node)
selected(n) :- text(n,"request").
.decl result(n:node)
result(n) :- selected(n).
.output result
"#,
    )
    .unwrap();
    profile::projection_plan(&p, &std::collections::BTreeSet::from(["text".into()]));
    let decisions = profile::decisions();
    profile::set(false);
    assert!(
        decisions
            .keys()
            .any(|(topic, text)| topic == "projection provenance"
                && text == "text <- selected <- result")
    );
}
