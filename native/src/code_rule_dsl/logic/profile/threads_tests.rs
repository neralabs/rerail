//! The profile accounts for every CPU second: each thread's CPU, under its spans and outside them.

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
