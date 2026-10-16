//! The substrate perf probe: every Python clause of the rule packs the SQL engine compiles,
//! matched by both engines over a large Python corpus, file by file (native) and batch by batch
//! (SQL). Reports each engine's matching time per clause, the statements a batch runs, their rows
//! and time, and checks both engines find the same roots.
//!
//! Ignored by default; run it in release:
//! `RERAIL_SQL_PROBE_CORPUS=~/rerail-bench/nera-data-layer-fce4dc72 cargo test --release
//! sql::tests::probe -- --ignored --nocapture`. `RERAIL_SQL_PROBE_FILES` caps the files,
//! `RERAIL_SQL_PROBE_BATCH` sets the batch size (default 64, the workspace's), and
//! `RERAIL_SQL_PROBE_WHOLE_ONLY` skips the clause-by-clause SQL runs.

use super::differential::tape;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{GrammarMatcher, run_clause, with_file_memo};
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE;
use crate::code_rule_dsl::tape::TapeDoc;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use crate::code_rule_dsl::{CodeRuleDslRuntime, CompiledCodeRuleDsl};
use ast_grep_core::AstGrep;
use rayon::prelude::*;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn python_files(directory: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            python_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "py") {
            found.push(path);
        }
    }
}

/// Every pack rule document, compiled; those the front end rejects are skipped.
fn pack_rules() -> Vec<(String, CompiledCodeRuleDsl)> {
    let packs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs");
    let mut documents = Vec::new();
    for pack in std::fs::read_dir(&packs).expect("packs").flatten() {
        let rules = pack.path().join("rules");
        let Ok(entries) = std::fs::read_dir(&rules) else {
            continue;
        };
        for entry in entries.flatten() {
            if entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "json")
            {
                documents.push(entry.path());
            }
        }
    }
    documents.sort();
    documents
        .into_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(&path).ok()?;
            let compiled =
                CodeRuleDslRuntime::compile(source, "json".to_owned(), CAPABILITIES.to_owned())
                    .ok()?;
            Some((path.file_stem()?.to_string_lossy().into_owned(), compiled))
        })
        .collect()
}

/// The roots `grammar` matches in each file, natively, and the time it took.
fn native_roots(grammar: &GrammarMatcher, docs: &[TapeDoc]) -> (Vec<Vec<u32>>, Duration) {
    let started = Instant::now();
    let roots = docs
        .iter()
        .map(|doc| {
            let root = AstGrep::doc(doc.clone());
            let mut found = Vec::new();
            let mut work = usize::MAX;
            with_file_memo(&root, || {
                run_clause(&root, grammar, None, usize::MAX, &mut work, |node, _, _| {
                    found.push(node);
                    Ok::<_, std::convert::Infallible>(true)
                })
            })
            .expect("infallible")
            .unwrap_or_else(|_| panic!("a native limit"));
            found
        })
        .collect();
    (roots, started.elapsed())
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "perf probe over a local corpus"]
fn probe() {
    let corpus = std::env::var("RERAIL_SQL_PROBE_CORPUS").unwrap_or_else(|_| {
        format!(
            "{}/rerail-bench/nera-data-layer-fce4dc72",
            std::env::var("HOME").expect("HOME")
        )
    });
    let limit = env_usize("RERAIL_SQL_PROBE_FILES", usize::MAX);
    let batch = env_usize("RERAIL_SQL_PROBE_BATCH", 64);
    let rules = pack_rules();
    let mut clauses: Vec<(String, &GrammarMatcher)> = Vec::new();
    let mut refused = std::collections::BTreeMap::<String, usize>::new();
    for (name, compiled) in &rules {
        for clause in &compiled.inner.clauses {
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else {
                continue;
            };
            let Some(grammar) = matchers
                .iter()
                .find(|grammar| grammar.language == GrammarDialect::Python && grammar.applies)
            else {
                continue;
            };
            match crate::code_rule_dsl::sql::compile(&[grammar]) {
                Ok(_) => clauses.push((format!("{name}/{}", clause.id), grammar)),
                Err(error) => *refused.entry(error.to_string()).or_default() += 1,
            }
        }
    }
    eprintln!(
        "probe: {} Python clauses compile, {} refused",
        clauses.len(),
        refused.values().sum::<usize>()
    );
    for (reason, count) in &refused {
        eprintln!("  refused {count:>4} {reason}");
    }
    let mut paths = Vec::new();
    python_files(Path::new(&corpus), &mut paths);
    paths.truncate(limit);
    let started = Instant::now();
    let docs: Vec<TapeDoc> = paths
        .par_iter()
        .filter_map(|path| {
            let source = std::fs::read_to_string(path).ok()?;
            std::panic::catch_unwind(|| tape(GrammarDialect::Python, &source)).ok()
        })
        .collect();
    let nodes: usize = docs.iter().map(TapeDoc::transducer_len).sum();
    eprintln!(
        "probe: {} files, {nodes} nodes, taped in {:?}; batch {batch}",
        docs.len(),
        started.elapsed()
    );
    // `RERAIL_SQL_PROBE_WHOLE_ONLY`: the SQL engine runs the whole set only (for profiling it).
    let whole_only = std::env::var_os("RERAIL_SQL_PROBE_WHOLE_ONLY").is_some();
    let mut native_total = Duration::ZERO;
    let mut sql_total = Duration::ZERO;
    let mut report = Vec::new();
    for (name, grammar) in &clauses {
        let (native, native_time) = native_roots(grammar, &docs);
        let found: usize = native.iter().map(Vec::len).sum();
        native_total += native_time;
        if whole_only {
            report.push((
                name.clone(),
                found,
                native_time,
                Duration::ZERO,
                Default::default(),
            ));
            continue;
        }
        let (sql, sql_time, stats) = super::probe_sql(grammar, &docs, batch);
        assert_eq!(native, sql, "{name}: the engines' roots differ");
        sql_total += sql_time;
        report.push((name.clone(), found, native_time, sql_time, stats));
    }
    report.sort_by_key(|(_, _, _, sql, _)| std::cmp::Reverse(*sql));
    eprintln!(
        "{:<70} {:>7} {:>10} {:>10} {:>7} {:>6} {:>10} {:>10}",
        "clause", "roots", "native", "sql", "ratio", "stmts", "rows/stmt", "us/stmt"
    );
    let mut totals = crate::code_rule_dsl::sql::Stats::default();
    for (name, found, native, sql, stats) in &report {
        eprintln!(
            "{:<70} {:>7} {:>10.1?} {:>10.1?} {:>7.2} {:>6} {:>10.1} {:>10.1}",
            name,
            found,
            native,
            sql,
            sql.as_secs_f64() / native.as_secs_f64().max(1e-9),
            stats.statements,
            stats.rows as f64 / stats.statements.max(1) as f64,
            sql.as_secs_f64() * 1e6 / stats.statements.max(1) as f64,
        );
        totals.statements += stats.statements;
        totals.rows += stats.rows;
        totals.nanos += stats.nanos;
        totals.stage_nanos += stats.stage_nanos;
        totals.select_nanos += stats.select_nanos;
        totals.input_nanos += stats.input_nanos;
        totals.facts_nanos += stats.facts_nanos;
    }
    eprintln!(
        "probe total, clause by clause: native {native_total:.2?}, sql {sql_total:.2?}, ratio {:.2}",
        sql_total.as_secs_f64() / native_total.as_secs_f64()
    );
    breakdown(&totals);
    // The whole clause set as one program on one session, every clause over every file of a
    // batch: what a workspace apply runs.
    let grammars: Vec<&GrammarMatcher> = clauses.iter().map(|(_, grammar)| *grammar).collect();
    let started = Instant::now();
    STATEMENT_PROFILE.set(Some(Default::default()));
    let (compiled, outcomes) = crate::code_rule_dsl::sql::compile_each(&grammars);
    let mut session = compiled.session().expect("a session");
    let setup = started.elapsed();
    let mut matched = 0;
    for chunk in docs.chunks(batch.max(1)) {
        let runs: Vec<_> = outcomes
            .iter()
            .map(|outcome| crate::code_rule_dsl::sql::ClauseRun {
                statement: *outcome.as_ref().expect("compiles"),
                files: (0..chunk.len()).map(|file| (file, None)).collect(),
            })
            .collect();
        let found = session.run(chunk.to_vec(), &runs).expect("runs");
        matched += found.iter().flatten().map(Vec::len).sum::<usize>();
    }
    let whole = started.elapsed();
    let expected: usize = report.iter().map(|(_, found, ..)| found).sum();
    assert_eq!(matched, expected, "the whole set finds what its clauses do");
    eprintln!(
        "probe total, whole set on one session: sql {whole:.2?} (session setup {setup:.2?}), ratio {:.2}",
        whole.as_secs_f64() / native_total.as_secs_f64()
    );
    breakdown(&session.stats());
    attribute(STATEMENT_PROFILE.take().unwrap_or_default());
}

/// The frame and stage statements' time by what they do, and the costliest statements.
fn attribute(profile: std::collections::HashMap<String, (u64, usize, usize)>) {
    let kind = |sql: &str| {
        if sql.contains("FROM pattern") {
            "pattern terminal"
        } else if sql.contains("text_match") {
            "regex terminal"
        } else if sql.starts_with("INSERT INTO stage_") {
            "stage result"
        } else if sql.contains("WITH RECURSIVE") {
            "recursive walk (rule stop)"
        } else if sql.contains("FROM runs") {
            "root frame"
        } else if sql.contains("GROUP BY") {
            "argmin / count (GROUP BY)"
        } else if sql.contains("JOIN node") {
            "relation / kind join with node"
        } else {
            "frame copy / anti-join / union"
        }
    };
    let total: u64 = profile.values().map(|(nanos, ..)| nanos).sum();
    let mut kinds = std::collections::BTreeMap::<&str, (u64, usize, usize)>::new();
    for (sql, (nanos, rows, runs)) in &profile {
        let entry = kinds.entry(kind(sql)).or_default();
        entry.0 += nanos;
        entry.1 += rows;
        entry.2 += runs;
    }
    let mut kinds: Vec<_> = kinds.into_iter().collect();
    kinds.sort_by_key(|(_, (nanos, ..))| std::cmp::Reverse(*nanos));
    eprintln!("statement time by kind (frames and stages, final selects excluded):");
    for (kind, (nanos, rows, runs)) in kinds {
        eprintln!(
            "  {:>5.1}% {:>8.0} ms {:>11} rows {:>7} runs  {kind}",
            100.0 * nanos as f64 / total.max(1) as f64,
            nanos as f64 / 1e6,
            rows,
            runs
        );
    }
    let mut statements: Vec<_> = profile.into_iter().collect();
    statements.sort_by_key(|(_, (nanos, ..))| std::cmp::Reverse(*nanos));
    eprintln!("costliest statements:");
    for (sql, (nanos, rows, runs)) in statements.into_iter().take(12) {
        eprintln!(
            "  {:>5.1}% {:>7.0} ms {:>10} rows {:>5} runs  {}",
            100.0 * nanos as f64 / total.max(1) as f64,
            nanos as f64 / 1e6,
            rows,
            runs,
            sql.chars().take(220).collect::<String>()
        );
    }
}

fn breakdown(stats: &crate::code_rule_dsl::sql::Stats) {
    let ms = |nanos: u64| nanos as f64 / 1e6;
    eprintln!(
        "  {} statements, {} rows ({:.1} rows/statement), {:.1} us/statement; statements {:.0} ms \
         (stages {:.0}, frames {:.0}, final selects {:.0}); batch inputs and clears {:.0} ms; \
         node facts {:.0} ms",
        stats.statements,
        stats.rows,
        stats.rows as f64 / stats.statements.max(1) as f64,
        stats.nanos as f64 / 1e3 / stats.statements.max(1) as f64,
        ms(stats.nanos),
        ms(stats.stage_nanos),
        ms(stats.nanos - stats.stage_nanos - stats.select_nanos),
        ms(stats.select_nanos),
        ms(stats.input_nanos),
        ms(stats.facts_nanos),
    );
}
