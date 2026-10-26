//! Anchor-selection laboratory: a policy's anchor plans, lowered by the current code and run over
//! captured batch databases (`RERAIL_SQL_PROFILE_DIR`). Seconds per iteration, no addon build, no
//! corpus parse. Reports SQLite VM steps (load-independent) and rows per plan (for parity).
//!
//! RERAIL_LAB_POLICY=<policy.json>[:<policy.json>...] RERAIL_LAB_CAPTURES=<dir> [RERAIL_LAB_LIMIT=n]
//!   cargo test --release --lib -- --ignored anchor_lab --nocapture
use super::{execute, lower, store::Store};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

const STEP: i32 = 1000;

#[test]
#[ignore = "requires a policy and captured candidate databases"]
fn anchor_lab() {
    let policies = std::env::var("RERAIL_LAB_POLICY").expect("RERAIL_LAB_POLICY");
    for policy in policies.split(':') {
        run(policy);
    }
}

fn run(policy: &str) {
    let captures = std::env::var("RERAIL_LAB_CAPTURES").expect("RERAIL_LAB_CAPTURES");
    let limit = std::env::var("RERAIL_LAB_LIMIT").map_or(usize::MAX, |n| n.parse().unwrap());
    let compiled = crate::code_rule_dsl::CodeRuleDslRuntime::compile(
        std::fs::read_to_string(policy).unwrap(),
        "json".to_owned(),
        crate::code_rule_dsl::test_support::CAPABILITIES.to_owned(),
    )
    .unwrap_or_else(|error| panic!("{}", error.reason));
    let plans = super::plans::plans([compiled.inner.as_ref()]);
    let mut databases: Vec<_> = std::fs::read_dir(&captures)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "sqlite"))
        .collect();
    databases.sort();
    databases.truncate(limit);
    let (mut steps, mut cpu, mut files) = (0u64, 0u64, 0usize);
    let mut rows: std::collections::BTreeMap<(u32, usize), usize> = Default::default();
    let mut compile = 0u64;
    for database in &databases {
        let copy = tempfile::NamedTempFile::new().unwrap();
        std::fs::copy(database, copy.path()).unwrap();
        let store = Store::open(copy.path()).unwrap();
        // Decoded facts are execution work; a capture holds the original run's.
        store
            .db
            .execute_batch("DELETE FROM decoded_properties; DELETE FROM decoded_files; DELETE FROM decoded_spans; DELETE FROM decoded_children;")
            .unwrap();
        let counter = Arc::new(AtomicU64::new(0));
        let observed = Arc::clone(&counter);
        store
            .db
            .progress_handler(
                STEP,
                Some(move || {
                    observed.fetch_add(1, Ordering::Relaxed);
                    false
                }),
            )
            .unwrap();
        for (&lang, plans) in &plans {
            let fids: Vec<i64> = store
                .db
                .prepare("SELECT fid FROM fk WHERE lang=?1 ORDER BY fid")
                .unwrap()
                .query_map([lang], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect();
            if fids.is_empty() {
                continue;
            }
            files += fids.len();
            let refs: Vec<_> = plans.iter().map(Arc::as_ref).collect();
            let started = cpu_ns();
            let statement = lower::batch_files(&refs, lang, &[]);
            compile += cpu_ns() - started;
            let started = cpu_ns();
            if statement.universal {
                execute::prepare_structural(&store, &fids).unwrap();
            }
            execute::rows_for(&store, &statement, &fids, true, |qid, _, _| {
                *rows.entry((lang, qid)).or_default() += 1;
            })
            .unwrap();
            cpu += cpu_ns() - started;
        }
        steps += counter.load(Ordering::Relaxed) * STEP as u64;
    }
    println!(
        "LAB {} databases={} files={files} vmSteps={:.3}G executeCpu={:.2}s lowerCpu={:.2}s",
        std::path::Path::new(policy).file_stem().unwrap().to_string_lossy(),
        databases.len(),
        steps as f64 / 1e9,
        cpu as f64 / 1e9,
        compile as f64 / 1e9
    );
    println!(
        "LAB rows {} {}",
        std::path::Path::new(policy).file_stem().unwrap().to_string_lossy(),
        rows.iter()
            .map(|((lang, qid), n)| format!("{lang}:{qid}={n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
}

fn cpu_ns() -> u64 {
    let mut now = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `now` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
    now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64
}
