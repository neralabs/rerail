//! Anchor-selection laboratory: a policy's anchor plans, lowered by the current code and run over
//! captured batch databases (`RERAIL_SQL_PROFILE_DIR`). Seconds per iteration, no addon build, no
//! corpus parse. Reports SQLite VM steps (load-independent) and rows per plan (for parity).
//!
//! RERAIL_LAB_POLICY=<policy.json>[:<policy.json>...] RERAIL_LAB_CAPTURES=<dir> [RERAIL_LAB_LIMIT=n]
//!   [RERAIL_LAB_PER_PLAN=1] (each plan lowered and run alone, after the batch)
//!   [RERAIL_LAB_BATCH=1] (production batches: each capture's requested files, captures spread
//!     over the run, and a warm vocabulary, which a corpus store fills once per term)
//!   [RERAIL_LAB_DUMP=<dir>] (writes each policy's lowered stages per language)
//!   [RERAIL_LAB_FALLBACK=1] (lowers as the pure-SQL fallback, `lower::fallback`: relations read
//!     the tape through `nera_tree`)
//!   cargo test --release --lib -- --ignored anchor_lab --nocapture
//!
//! From Pulling-it-all-together's `matcher/anchors/sql/lab.rs`, pointed at the content store: a
//! capture is a store holding one batch's prefilter rows (`capture`), opened as a `ContentStore`,
//! statements are lowered with the capture's statistics (`stats::inputs`) and run on its
//! connection. Gone with decoding: clearing `decoded_*` tables and the structural fill.
pub(super) mod capture;
#[cfg(any(test, feature = "test-hooks"))]
mod replay;

#[cfg(any(test, feature = "test-hooks"))]
use super::{execute, lower, tests::{open, temp}};
#[cfg(any(test, feature = "test-hooks"))]
use crate::code_rule_dsl::stats::inputs::Statistics;
#[cfg(any(test, feature = "test-hooks"))]
use crate::content_store::ContentStore;
#[cfg(any(test, feature = "test-hooks"))]
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[cfg(any(test, feature = "test-hooks"))]
const STEP: i32 = 1000;

#[cfg(any(test, feature = "test-hooks"))]
#[test]
#[ignore = "requires a policy and captured candidate databases"]
fn anchor_lab() {
    let policies = std::env::var("RERAIL_LAB_POLICY").expect("RERAIL_LAB_POLICY");
    for policy in policies.split(':') {
        run(policy);
    }
}

#[cfg(any(test, feature = "test-hooks"))]
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
    // One statement per language and batch, lowered with the capture's statistics and run on its
    // connection; the progress handler counts VM steps for the statement.
    let lowered = |store: &ContentStore, plans: &[&crate::code_rule_dsl::matcher::AnchorPlan], lang: u32, fids: &[i64]| {
        let statistics = store
            .candidate_transaction(false, |db| Statistics::read(db, plans.iter().map(|plan| (lang, *plan))))
            .unwrap();
        if std::env::var_os("RERAIL_LAB_FALLBACK").is_some() {
            lower::fallback(plans, lang, fids, &statistics)
        } else {
            lower::batch(plans, lang, fids, &statistics)
        }
    };
    let run_statement = |store: &ContentStore, statement: &lower::Statement, counter: &Arc<AtomicU64>, emit: &mut dyn FnMut(usize)| {
        let observed = Arc::clone(counter);
        store
            .candidate_progress(STEP, Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }))
            .unwrap();
        let result = store.candidate_transaction(statement.writes(), |db| execute::rows(db, statement, &mut |qid, _, _| emit(qid)));
        store.candidate_progress(0, None::<fn() -> bool>).unwrap();
        result.unwrap();
    };
    let mut databases: Vec<_> = std::fs::read_dir(&captures)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "sqlite"))
        .collect();
    let batch = std::env::var_os("RERAIL_LAB_BATCH").is_some();
    if batch {
        let id = |path: &std::path::PathBuf| -> u64 {
            let stem = path.file_stem().unwrap().to_string_lossy();
            stem.rsplit('-').next().unwrap().parse().unwrap()
        };
        databases.sort_by_key(id);
        let count = databases.len();
        let chosen = limit.min(count);
        databases = (0..chosen)
            .map(|i| databases[i * count / chosen].clone())
            .collect();
    } else {
        databases.sort();
        databases.truncate(limit);
    }
    let (mut steps, mut cpu, mut files) = (0u64, 0u64, 0usize);
    let mut rows: std::collections::BTreeMap<(u32, usize), usize> = Default::default();
    let mut compile = 0u64;
    let mut broadened = std::collections::BTreeSet::new();
    let mut per_plan: std::collections::BTreeMap<(u32, usize), u64> = Default::default();
    for database in &databases {
        let copy = temp();
        std::fs::copy(database, &copy).unwrap();
        let store = open(&copy);
        let requested: Option<Vec<i64>> = batch.then(|| {
            let json: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(database.with_extension("json")).unwrap(),
            )
            .unwrap();
            json["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|fid| fid.as_i64().unwrap())
                .collect()
        });
        let language_files = |store: &ContentStore, lang: u32| -> Vec<i64> {
            store
                .query_rows(
                    "SELECT s.tape_id FROM prefilter_sets s JOIN tape_objects t USING(tape_id) WHERE t.grammar_id=?1 ORDER BY 1",
                    [lang],
                    |row| row.get(0),
                )
                .unwrap()
                .into_iter()
                .filter(|fid| requested.as_ref().is_none_or(|files| files.contains(fid)))
                .collect()
        };
        if batch {
            // The corpus store's vocabulary already holds every earlier term; a capture made under
            // another policy has none of this policy's predicates.
            for (&lang, plans) in &plans {
                let fids = language_files(&store, lang);
                if fids.is_empty() {
                    continue;
                }
                let refs: Vec<_> = plans.iter().map(Arc::as_ref).collect();
                let statement = lowered(&store, &refs, lang, &fids);
                run_statement(&store, &statement, &Arc::new(AtomicU64::new(0)), &mut |_| {});
            }
        }
        let counter = Arc::new(AtomicU64::new(0));
        for (&lang, plans) in &plans {
            let fids = language_files(&store, lang);
            if fids.is_empty() {
                continue;
            }
            files += fids.len();
            let refs: Vec<_> = plans.iter().map(Arc::as_ref).collect();
            let started = cpu_ns();
            let statement = lowered(&store, &refs, lang, &fids);
            compile += cpu_ns() - started;
            broadened.extend(
                statement
                    .admissions
                    .iter()
                    .filter(|admission| admission.broadened)
                    .map(|admission| (lang, admission.consumer)),
            );
            if let Ok(dump) = std::env::var("RERAIL_LAB_DUMP") {
                let mut text = String::new();
                for stage in &statement.stages {
                    text += &format!("-- {}\n{};\n", stage.name, stage.sql);
                }
                for query in &statement.queries {
                    text += &format!("-- results\n{query};\n");
                }
                let stem = std::path::Path::new(policy)
                    .file_stem()
                    .unwrap()
                    .to_string_lossy();
                std::fs::write(format!("{dump}/{stem}.{lang}.sql"), text).unwrap();
            }
            let started = cpu_ns();
            run_statement(&store, &statement, &counter, &mut |qid| {
                *rows.entry((lang, qid)).or_default() += 1;
            });
            cpu += cpu_ns() - started;
        }
        steps += counter.load(Ordering::Relaxed) * STEP as u64;
        if std::env::var_os("RERAIL_LAB_PER_PLAN").is_some() {
            for (&lang, plans) in &plans {
                let fids = language_files(&store, lang);
                let only = std::env::var("RERAIL_LAB_ONLY").ok();
                for (qid, plan) in plans.iter().enumerate() {
                    if only
                        .as_ref()
                        .is_some_and(|only| only.split(',').all(|q| q != qid.to_string()))
                    {
                        continue;
                    }
                    let statement = lowered(&store, &[plan.as_ref()], lang, &fids);
                    if let Ok(dump) = std::env::var("RERAIL_LAB_DUMP") {
                        let text: String = statement
                            .stages
                            .iter()
                            .map(|stage| format!("-- {}\n{};\n", stage.name, stage.sql))
                            .chain(
                                statement
                                    .queries
                                    .iter()
                                    .map(|query| format!("-- results\n{query};\n")),
                            )
                            .collect();
                        let stem = std::path::Path::new(policy)
                            .file_stem()
                            .unwrap()
                            .to_string_lossy();
                        std::fs::write(format!("{dump}/{stem}.{lang}.q{qid}.sql"), text).unwrap();
                    }
                    let before = counter.load(Ordering::Relaxed);
                    run_statement(&store, &statement, &counter, &mut |_| {});
                    *per_plan.entry((lang, qid)).or_default() +=
                        (counter.load(Ordering::Relaxed) - before) * STEP as u64;
                }
            }
        }
    }
    let mut ranked: Vec<_> = per_plan.iter().collect();
    ranked.sort_by_key(|(_, steps)| std::cmp::Reverse(**steps));
    for ((lang, qid), steps) in ranked.into_iter().take(25) {
        println!("LAB plan {lang}:{qid} vmSteps={:.1}M", *steps as f64 / 1e6);
    }
    println!(
        "LAB {} databases={} files={files} vmSteps={:.3}G executeCpu={:.2}s lowerCpu={:.2}s",
        std::path::Path::new(policy)
            .file_stem()
            .unwrap()
            .to_string_lossy(),
        databases.len(),
        steps as f64 / 1e9,
        cpu as f64 / 1e9,
        compile as f64 / 1e9
    );
    println!(
        "LAB broadened {} {}",
        std::path::Path::new(policy)
            .file_stem()
            .unwrap()
            .to_string_lossy(),
        broadened
            .iter()
            .map(|(lang, qid)| format!("{lang}:{qid}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "LAB rows {} {}",
        std::path::Path::new(policy)
            .file_stem()
            .unwrap()
            .to_string_lossy(),
        rows.iter()
            .map(|((lang, qid), n)| format!("{lang}:{qid}={n}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
}

#[cfg(any(test, feature = "test-hooks"))]
fn cpu_ns() -> u64 {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `now` is a valid timespec for the call to fill.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
    now.tv_sec as u64 * 1_000_000_000 + now.tv_nsec as u64
}

#[cfg(any(test, feature = "test-hooks"))]
#[test]
fn a_capture_replays_to_the_candidates_of_its_statement() {
    use crate::code_rule_dsl::{language::{self, GrammarDialect}, matcher::AnchorPlan};
    let store = open(&temp());
    let lang = GrammarDialect::TypeScript;
    let tapes: Vec<i64> = ["function target(a) { return a; }", "const x = target(1);", "let y = 2;"]
        .iter()
        .map(|source| super::tests::put(&store, source, lang).0.row())
        .collect();
    let rules = [serde_json::json!({"pattern": "target($A)"}), serde_json::json!({"regex": "^tar", "kind": "identifier"})];
    let matchers: Vec<_> = rules.iter().map(|rule| super::tests::matcher(rule, lang)).collect();
    let plans: Vec<_> = matchers.iter().map(AnchorPlan::compile).collect();
    let refs: Vec<_> = plans.iter().collect();
    let tag = u32::from(language::tape_tag(lang).unwrap());
    let statistics = store.candidate_transaction(false, |db| Statistics::read(db, refs.iter().map(|plan| (tag, *plan)))).unwrap();
    let statement = lower::batch(&refs, tag, &tapes, &statistics);
    let mut expected = Vec::new();
    execute::candidates(&store, &statement, |plan, tape, pre| expected.push((plan as i64, tape, pre as i64))).unwrap();
    expected.sort_unstable();
    assert!(!expected.is_empty());

    let directory = temp();
    let capture = capture::save_to(&store, &statement, &directory).unwrap();
    let (summary, rows) = replay::replay(&capture);
    assert_eq!(rows, expected);
    assert_eq!(summary["rows"].as_u64(), Some(expected.len() as u64));
    // The capture holds the batch's rows and not the tapes' bytes.
    let captured = open(&directory.join(capture.with_extension("sqlite").file_name().unwrap()));
    let sizes = captured.query_rows("SELECT count(*), sum(length(frame)) FROM tape_objects", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).unwrap();
    assert_eq!(sizes, [(3, 0)]);
}


/// Captures (`RERAIL_SQL_PROFILE_DIR`) of the candidate SQL over a corpus, for the lab's check
/// against P on the same files (list step 12): every `.py` file of `RERAIL_LAB_CORPUS` is stored
/// and published, then the Python pack's policies are selected over `RERAIL_LAB_BATCHES` (a JSON
/// array of arrays of file names, P's batches) or one batch of every file.
///   RERAIL_LAB_CORPUS=<dir> RERAIL_SQL_PROFILE_DIR=<out> [RERAIL_LAB_BATCHES=<json>]
///   [RERAIL_CAPTURE_TAPES=1] cargo test --lib -- --ignored capture_lab_corpus
#[cfg(any(test, feature = "test-hooks"))]
#[test]
#[ignore = "requires a corpus directory"]
fn capture_lab_corpus() {
    use crate::code_rule_dsl::language::{self, GrammarDialect};
    let corpus = std::path::PathBuf::from(std::env::var("RERAIL_LAB_CORPUS").expect("RERAIL_LAB_CORPUS"));
    assert!(std::env::var_os("RERAIL_SQL_PROFILE_DIR").is_some(), "RERAIL_SQL_PROFILE_DIR");
    let store = open(&temp());
    let mut names = std::fs::read_dir(&corpus).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".py")).collect::<Vec<_>>();
    names.sort();
    let tapes = names
        .iter()
        .map(|name| (name.clone(), super::tests::put(&store, &std::fs::read_to_string(corpus.join(name)).unwrap(), GrammarDialect::Python).0))
        .collect::<std::collections::BTreeMap<_, _>>();
    let pack = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs/python");
    let mut documents = Vec::new();
    for dir in [pack.join("rules"), pack.join("candidates/rules")] {
        documents.extend(std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "json")));
    }
    documents.sort();
    let compiled = documents
        .iter()
        .filter_map(|path| {
            crate::code_rule_dsl::CodeRuleDslRuntime::compile(std::fs::read_to_string(path).unwrap(), "json".to_owned(), crate::code_rule_dsl::test_support::CAPABILITIES.to_owned()).ok()
        })
        .collect::<Vec<_>>();
    let plans = super::plans::plans(compiled.iter().map(|c| c.inner.as_ref()));
    let tag = u32::from(language::tape_tag(GrammarDialect::Python).unwrap());
    let refs = plans[&tag].iter().map(Arc::as_ref).collect::<Vec<_>>();
    let batches: Vec<Vec<String>> = match std::env::var("RERAIL_LAB_BATCHES") {
        Ok(path) => serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap(),
        Err(_) => vec![names.clone()],
    };
    for batch in &batches {
        let ids = batch.iter().filter_map(|name| tapes.get(name).copied()).collect::<Vec<_>>();
        super::select_indexed_sql(&store, tag, &refs, &ids).unwrap();
    }
    eprintln!("captured {} batches of {} files, {} policies, {} plans", batches.len(), tapes.len(), compiled.len(), refs.len());
}
