//! How a session's setup time grows with its tables. Run by hand:
//! `cargo test --release --lib sql::tests::setup -- --ignored --nocapture --test-threads 1`.
//!
//! - `tables_made_alone`: making `n` TEMP B-trees against `n` frames, on a fresh connection each.
//!   A `CREATE TABLE` scans the whole schema to parse its own entry, so B-trees take time growing
//!   with the square of `n`; a frame is a module registration and takes constant time.
//! - `the_benchmark_policies_session_setup`: sessions over growing prefixes of the data-layer
//!   benchmark's lowered grammar matchers (`RERAIL_SETUP_GRAMMARS`, default `50,100,200,400`),
//!   each compiled as a workspace compiles them (`workspace::sql::SqlEngine`).

use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::sql::frames::FrameStore;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use rusqlite::Connection;
use std::sync::Arc;
use std::time::Instant;

#[test]
#[ignore = "a measurement, run by hand"]
fn tables_made_alone() {
    let counts: Vec<usize> = std::env::var("RERAIL_SETUP_TABLES")
        .unwrap_or_else(|_| "1000,2000,4000,8000,16000".to_owned())
        .split(',')
        .map(|count| count.trim().parse().expect("a count"))
        .collect();
    let btree_limit: usize = std::env::var("RERAIL_SETUP_BTREES")
        .ok()
        .and_then(|limit| limit.parse().ok())
        .unwrap_or(16_000);
    for n in counts {
        // B-trees past `RERAIL_SETUP_BTREES` would take minutes: skipped.
        let btrees = (n <= btree_limit).then(|| {
            let connection = Connection::open_in_memory().expect("opens");
            connection
                .execute_batch("PRAGMA temp_store = MEMORY")
                .expect("memory");
            let started = Instant::now();
            for table in 0..n {
                connection
                    .execute_batch(&format!(
                        "CREATE TEMP TABLE t{table}(k INTEGER PRIMARY KEY, file, node, s0, s1)"
                    ))
                    .expect("a B-tree");
            }
            started.elapsed().as_secs_f64()
        });
        let connection = Connection::open_in_memory().expect("opens");
        let store = Arc::new(FrameStore::default());
        let started = Instant::now();
        for table in 0..n {
            store
                .register(
                    &connection,
                    &format!("t{table}(k INTEGER PRIMARY KEY, file, node, s0, s1)"),
                )
                .expect("a frame");
        }
        // A frame's table is connected on first use: read each once.
        for table in 0..n {
            connection
                .query_row(&format!("SELECT count(*) FROM t{table}"), [], |row| {
                    row.get::<_, i64>(0)
                })
                .expect("reads");
        }
        let frames = started.elapsed().as_secs_f64();
        let btrees = btrees.map_or_else(
            || "B-trees skipped".to_owned(),
            |btrees| format!("B-trees {btrees:.3}s ({:.1} µs/table)", btrees * 1e6 / n as f64),
        );
        eprintln!(
            "{n:>7} tables: {btrees}, frames made and read once {frames:.3}s ({:.1} µs/table)",
            frames * 1e6 / n as f64
        );
    }
}

#[test]
#[ignore = "reads the data-layer benchmark's policies from outside the repository"]
fn the_benchmark_policies_session_setup() {
    // Compiling the largest policies recurses deeper than a test thread's stack.
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(run)
        .expect("a setup thread")
        .join()
        .expect("the setup");
}

fn run() {
    let root = std::env::var("RERAIL_BENCH_POLICIES").unwrap_or_else(|_| {
        format!(
            "{}/rerail-bench/data-layer/policies",
            std::env::var("HOME").expect("HOME")
        )
    });
    let prefixes: Vec<usize> = std::env::var("RERAIL_SETUP_GRAMMARS")
        .unwrap_or_else(|_| "50,100,200,400".to_owned())
        .split(',')
        .map(|count| count.trim().parse().expect("a count"))
        .collect();
    let mut policies: Vec<_> = std::fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("{root}: {error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("policy.json"))
        .filter(|path| path.exists())
        .collect();
    policies.sort();
    let compiled: Vec<_> = policies
        .iter()
        .map(|path| {
            CodeRuleDslRuntime::compile(
                std::fs::read_to_string(path).expect("policy"),
                "json".to_owned(),
                CAPABILITIES.to_owned(),
            )
            .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason))
        })
        .collect();
    let mut grammars = Vec::new();
    for policy in &compiled {
        for clause in &policy.inner.clauses {
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for grammar in matchers {
                    grammars.push((grammar, clause.violation.each.as_deref()));
                }
            }
        }
    }
    for count in prefixes {
        let prefix = &grammars[..count.min(grammars.len())];
        let started = Instant::now();
        let (program, outcomes) = crate::code_rule_dsl::sql::compile_each(prefix);
        let compiled_in = started.elapsed();
        let lowered = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
        let started = Instant::now();
        let session = program.session().expect("a session");
        let setup = started.elapsed().as_secs_f64();
        let (btrees, frames) = session.tables().expect("tables");
        let tables = btrees + frames;
        eprintln!(
            "{:>5} grammar matchers ({lowered} lowered): setup {setup:.3}s for {tables} tables ({btrees} B-trees, {frames} frames), {:.1} µs/table; program compiled in {:.1}s",
            prefix.len(),
            setup * 1e6 / tables.max(1) as f64,
            compiled_in.as_secs_f64(),
        );
    }
}
