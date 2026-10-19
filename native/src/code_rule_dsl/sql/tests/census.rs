//! What the SQL engine refuses in the data-layer benchmark's policies: every clause's grammar
//! matchers lowered with every refusal recorded (`exec::census`), counted per operator. The
//! policies live outside the repository (`RERAIL_BENCH_POLICIES`, default
//! `~/rerail-bench/data-layer/policies`), so the census is run by hand:
//! `cargo test --release sql::tests::census -- --ignored --nocapture`.

use crate::code_rule_dsl::CodeRuleDslRuntime;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::sql::exec::census;
use crate::code_rule_dsl::test_support::CAPABILITIES;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct Count {
    /// Refusals recorded, one per lowering site.
    sites: usize,
    /// Clauses with at least one.
    clauses: BTreeSet<(String, String)>,
}

fn query(
    query: &Query,
    each: Option<&str>,
    found: &mut Vec<String>,
    lowered: &mut usize,
    matchers: &mut usize,
) {
    match query {
        Query::Regex { .. } => {}
        Query::Ast { matchers: all, .. } => {
            for grammar in all {
                *matchers += 1;
                let refusals = census(grammar, each);
                if refusals.is_empty() {
                    *lowered += 1;
                    audit(grammar, each);
                }
                found.extend(refusals);
            }
        }
        // `select_when` combines its leaves in SQL (`sql::select_when`, which the stateless and
        // workspace evaluators route to on the SQL engine): only its leaves can refuse.
        Query::SelectWhen { select, exists } => {
            self::query(select, each, found, lowered, matchers);
            self::query(exists, None, found, lowered, matchers);
        }
    }
}

/// The plan audit of `emitted_joins_never_scan_a_whole_table_on_the_inner_side`, on a benchmark
/// clause the engine lowers: no statement reads a whole table on the inner side of a join.
fn audit(grammar: &crate::code_rule_dsl::matcher::GrammarMatcher, each: Option<&str>) {
    let program = crate::code_rule_dsl::sql::compile_enumerating(&[grammar], each)
        .expect("a clause that lowers compiles");
    let session = program.session().expect("a session");
    for (sql, plan) in session.plans().expect("plans") {
        let scans = super::differential::inner_scans(&plan);
        assert!(
            scans.is_empty(),
            "inner scans {scans:?} in {}",
            sql.chars().take(600).collect::<String>()
        );
    }
}

/// Whether a query's matches come from a three-valued grammar matcher (for `select_when`, its
/// `select` side, which a clause's `violation.each` enumerates).
fn three_valued(query: &Query) -> bool {
    match query {
        Query::Regex { .. } => false,
        Query::Ast { matchers, .. } => matchers.iter().any(|grammar| grammar.three_valued),
        Query::SelectWhen { select, .. } => three_valued(select),
    }
}

#[test]
#[ignore = "reads the data-layer benchmark's policies from outside the repository"]
fn the_benchmark_policies_census() {
    // Compiling the largest policies recurses deeper than a test thread's stack.
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(run_census)
        .expect("a census thread")
        .join()
        .expect("the census");
}

fn run_census() {
    let root = std::env::var("RERAIL_BENCH_POLICIES").unwrap_or_else(|_| {
        format!(
            "{}/rerail-bench/data-layer/policies",
            std::env::var("HOME").expect("HOME")
        )
    });
    let mut policies: Vec<_> = std::fs::read_dir(&root)
        .unwrap_or_else(|error| panic!("{root}: {error}"))
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("policy.json"))
        .filter(|path| path.exists())
        .collect();
    policies.sort();
    let started = std::time::Instant::now();
    let mut counts: BTreeMap<String, Count> = BTreeMap::new();
    let (mut clauses, mut matchers, mut lowered) = (0usize, 0usize, 0usize);
    // The clauses enumerating `violation.each` over a three-valued matcher, and whether each
    // lowered without a refusal.
    let mut three_each: Vec<(String, bool)> = Vec::new();
    for path in &policies {
        let started = std::time::Instant::now();
        let source = std::fs::read_to_string(path).expect("policy");
        let compiled =
            CodeRuleDslRuntime::compile(source, "json".to_owned(), CAPABILITIES.to_owned())
                .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason));
        let policy = path
            .parent()
            .and_then(|parent| parent.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        eprintln!(
            "{}: compiled in {:.1}s",
            path.display(),
            started.elapsed().as_secs_f64()
        );
        for clause in &compiled.inner.clauses {
            clauses += 1;
            let mut found = Vec::new();
            match &clause.kind {
                ClauseKind::Forbid(forbid) => query(
                    forbid,
                    clause.violation.each.as_deref(),
                    &mut found,
                    &mut lowered,
                    &mut matchers,
                ),
                // `require_each` and `budget` join their producers' rows in SQL
                // (`sql::crossfile`): only the producers' queries can refuse.
                ClauseKind::RequireEach { subject, evidence } => {
                    for producer in [subject, evidence] {
                        query(
                            &producer.ast.query,
                            None,
                            &mut found,
                            &mut lowered,
                            &mut matchers,
                        );
                    }
                }
                ClauseKind::Budget {
                    query: producer, ..
                } => query(
                    &producer.ast.query,
                    None,
                    &mut found,
                    &mut lowered,
                    &mut matchers,
                ),
                ClauseKind::Llm => found.push("llm".to_owned()),
            }
            if let ClauseKind::Forbid(forbid) = &clause.kind
                && clause.violation.each.is_some()
                && three_valued(forbid)
            {
                three_each.push((format!("{policy}/{}", clause.id), found.is_empty()));
            }
            for operator in found {
                if std::env::var_os("RERAIL_CENSUS_VERBOSE").is_some() {
                    eprintln!("  {policy}/{}: {operator}", clause.id);
                }
                let count = counts.entry(operator).or_default();
                count.sites += 1;
                count.clauses.insert((policy.clone(), clause.id.clone()));
            }
        }
    }
    eprintln!(
        "census: {:.1}s, {} policies, {clauses} clauses, {matchers} grammar matchers, {lowered} lowered without a refusal",
        started.elapsed().as_secs_f64(),
        policies.len()
    );
    eprintln!("| Unsupported | sites | clauses |\n|---|---|---|");
    for (operator, count) in &counts {
        eprintln!(
            "| `{operator}` | {} | {} |",
            count.sites,
            count.clauses.len()
        );
    }
    eprintln!(
        "three-valued violation.each: {} clauses, {} lowered without a refusal: {three_each:?}",
        three_each.len(),
        three_each.iter().filter(|(_, clean)| *clean).count()
    );
    for gone in [
        "recursive utility",
        "unfilled utility",
        "violation.each",
        "three-valued violation.each",
        "pattern with more than 32 capture columns",
        "call mapping more than 4 parameters to one capture",
    ] {
        assert!(!counts.contains_key(gone), "{gone} is still refused");
    }
}

/// The corpus files the benchmark evaluates: the tracked Python files of a nera-data-layer
/// checkout (`RERAIL_DATA_LAYER_CORPUS`), as a snapshot's files.
fn corpus_files(root: &std::path::Path) -> Vec<serde_json::Value> {
    let listed = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .expect("git ls-files");
    let mut paths: Vec<String> = String::from_utf8(listed.stdout)
        .expect("paths")
        .split('\0')
        .filter(|path| path.ends_with(".py"))
        .map(str::to_owned)
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|path| {
            let absolute = root.join(&path);
            let source = String::from_utf8(std::fs::read(&absolute).ok()?).ok()?;
            let mut file =
                serde_json::json!({ "language": "python", "path": path, "source": source });
            if let Some(module) =
                crate::code_rule_dsl::language::adapter::module_path("python", &absolute)
            {
                file["module"] = module;
            }
            Some(file)
        })
        .collect()
}

/// Every benchmark clause the SQL engine evaluates (it refuses the others), evaluated alone by
/// both engines over the whole corpus with limits lifted: their outputs must be identical. Run by
/// hand: `cargo test --release sql::tests::census -- --ignored --nocapture`.
#[test]
#[ignore = "reads the data-layer benchmark's policies and corpus from outside the repository"]
fn the_benchmark_clauses_the_engine_lowers_agree_on_the_corpus() {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(run_corpus)
        .expect("a corpus thread")
        .join()
        .expect("the corpus run");
}

fn run_corpus() {
    use serde_json::{Value, json};
    let home = std::env::var("HOME").expect("HOME");
    let root = std::env::var("RERAIL_BENCH_POLICIES")
        .unwrap_or_else(|_| format!("{home}/rerail-bench/data-layer/policies"));
    let corpus = std::env::var("RERAIL_DATA_LAYER_CORPUS")
        .unwrap_or_else(|_| format!("{home}/orca/workspaces/nera-data-layer"));
    let started = std::time::Instant::now();
    let files = corpus_files(std::path::Path::new(&corpus));
    let snapshot = json!({
        "facts": [],
        "files": files,
        "format": "rerail.repository-snapshot/v1",
        "revision": 0,
    })
    .to_string();
    eprintln!("corpus: {} files", files.len());
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    capabilities["limits"]["maxWitnesses"] = json!(100_000_000);
    capabilities["limits"]["maxVisitedNodes"] = json!(1_000_000_000_000u64);
    let capabilities = capabilities.to_string();
    let mut policies: Vec<_> = std::fs::read_dir(&root)
        .expect("policies")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("policy.json"))
        .filter(|path| path.exists())
        .collect();
    policies.sort();
    let (mut compared, mut refused, mut differ) = (0usize, 0usize, Vec::new());
    let (mut each, mut recursive, mut findings) = (0usize, 0usize, 0usize);
    for path in &policies {
        let policy: Value =
            serde_json::from_str(&std::fs::read_to_string(path).expect("policy")).expect("JSON");
        // Each clause alone, with the policy's `define`s it may read (they report nothing).
        let clauses = policy["clauses"].as_array().expect("clauses");
        let defines: Vec<&Value> = clauses
            .iter()
            .filter(|clause| clause.get("define").is_some())
            .collect();
        for clause in clauses
            .iter()
            .filter(|clause| clause.get("define").is_none())
        {
            // `RERAIL_CORPUS_CLAUSES`: only the clauses whose `policy/id` holds this text.
            let name = format!(
                "{}/{}",
                path.parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default(),
                clause["id"].as_str().unwrap_or_default()
            );
            if std::env::var("RERAIL_CORPUS_CLAUSES").is_ok_and(|only| !name.contains(&only)) {
                continue;
            }
            let mut document = policy.clone();
            document["clauses"] = json!(defines.iter().chain([&clause]).collect::<Vec<_>>());
            let compiled = CodeRuleDslRuntime::compile(
                document.to_string(),
                "json".to_owned(),
                capabilities.clone(),
            )
            .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason));
            let evaluate = |engine: &str| {
                compiled
                    .evaluate(
                        snapshot.clone(),
                        "report".to_owned(),
                        Some(engine.to_owned()),
                    )
                    .map_err(|error| error.reason)
            };
            let clause_started = std::time::Instant::now();
            crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE.set(Some(Default::default()));
            let sql = evaluate("sql");
            let profile = crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE
                .take()
                .unwrap_or_default();
            if sql
                .as_ref()
                .is_err_and(|reason| reason.contains("Unsupported("))
            {
                refused += 1;
                continue;
            }
            let sql_time = clause_started.elapsed();
            let native = evaluate("native");
            let text = clause.to_string();
            let enumerates = clause["violation"].get("each").is_some();
            let recurses = compiled.inner.clauses.iter().any(|compiled| {
                matches!(&compiled.kind, ClauseKind::Forbid(Query::Ast { matchers, .. })
                    if matchers.iter().any(|grammar| recursion_in(&grammar.matcher)))
            });
            each += usize::from(enumerates);
            recursive += usize::from(recurses);
            compared += 1;
            let count = native
                .as_ref()
                .ok()
                .and_then(|output| serde_json::from_str::<Value>(output).ok())
                .and_then(|output| output["violations"].as_array().map(Vec::len))
                .unwrap_or(0);
            findings += count;
            eprintln!(
                "{} {}: {count} findings{}{}, sql {:.2}s, both {:.2}s{}",
                path.parent()
                    .and_then(|parent| parent.file_name())
                    .map(|name| name.to_string_lossy())
                    .unwrap_or_default(),
                clause["id"].as_str().unwrap_or_default(),
                if enumerates { ", each" } else { "" },
                if recurses { ", recursive" } else { "" },
                sql_time.as_secs_f64(),
                clause_started.elapsed().as_secs_f64(),
                if native == sql { "" } else { " DIFFERS" },
            );
            if native != sql {
                differ.push((text, native, sql));
            }
            // Where a slow clause's time went: its costliest statements.
            if sql_time.as_secs() >= 10 {
                let mut costly: Vec<_> = profile.iter().collect();
                costly.sort_by_key(|(_, (nanos, _, _))| std::cmp::Reverse(*nanos));
                for (statement, (nanos, rows, runs)) in costly.into_iter().take(4) {
                    eprintln!(
                        "  {:>8.3}s {rows:>10} rows {runs:>6} runs  {}",
                        *nanos as f64 / 1e9,
                        statement.chars().take(300).collect::<String>()
                    );
                }
            }
        }
    }
    eprintln!(
        "corpus parity: {compared} clauses compared ({each} under violation.each, {recursive} recursive, {findings} findings), {refused} refused, {} differ, {:.1}s",
        differ.len(),
        started.elapsed().as_secs_f64()
    );
    for (clause, native, sql) in differ.iter().take(3) {
        let cut = |text: &str| text.chars().take(1500).collect::<String>();
        eprintln!(
            "clause: {}\n native: {}\n sql: {}",
            cut(clause),
            cut(&format!("{native:?}")),
            cut(&format!("{sql:?}"))
        );
    }
    assert!(differ.is_empty(), "{} clauses differ", differ.len());
}

/// Whether a rule reaches a recursive utility.
fn recursion_in(rule: &crate::code_rule_dsl::matcher::AstMatcher) -> bool {
    use crate::code_rule_dsl::sql::recursion::Recursion;
    let mut recursion = Recursion::default();
    let mut stack = vec![rule.clone()];
    let mut seen = std::collections::HashSet::new();
    while let Some(rule) = stack.pop() {
        let mut found = Vec::new();
        crate::code_rule_dsl::sql::recursion::references_of(&rule, &mut found);
        for util in found {
            if recursion.cycle(&util).is_some() {
                return true;
            }
            if seen.insert((util.table.as_ptr() as usize, util.index)) {
                util.with_target(|target| stack.push(target.clone()));
            }
        }
    }
    false
}

/// Diagnosis of the differential's slowest case (`RERAIL_SQL_DIFF_SKIP`): each statement's rows
/// over growing prefixes of a Python fixture, against the file's node count. Run by hand:
/// `cargo test --release sql::tests::census::the_slow_case_rows -- --ignored --nocapture`.
#[test]
#[ignore = "a diagnosis, run by hand"]
fn the_slow_case_rows() {
    use crate::code_rule_dsl::sql::exec::STATEMENT_PROFILE;
    use crate::code_rule_dsl::test_support::{clause_document, compile_document_with};
    use serde_json::{Value, json};
    let rule = json!({
        "follows": { "count": { "max": 3, "min": 2, "precedes": { "kind": "if_statement", "pattern": "a == (\"a\" \"b\")", "stopBy": "end" } } },
        "inside": {
            "has": { "count": { "as": "N", "max": 0, "offset": 0, "precedes": { "nthChild": "2n+1", "stopBy": "end" } }, "stopBy": "end" },
            "not": { "pattern": "j := $A" },
            "stopBy": { "kind": "named_expression" },
        },
        "not": { "kind": "pass_statement" },
    });
    let document = clause_document(json!({
        "id": "slow", "forbid": { "ast": { "language": "python", "rule": rule } },
        "violation": { "anchor": "witness.span", "code": "X", "message": "x" },
    }));
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    capabilities["limits"]["maxWitnesses"] = json!(100_000_000);
    capabilities["limits"]["maxVisitedNodes"] = json!(1_000_000_000_000u64);
    let compiled = compile_document_with(document, &capabilities).expect("compiles");
    let source = std::fs::read_to_string(format!(
        "{}/python/scope/usage/F841.py",
        crate::code_rule_dsl::fixtures_dir!()
    ))
    .expect("fixture");
    let lines: Vec<&str> = source.lines().collect();
    for take in [10, 20, 40] {
        let prefix = lines[..take.min(lines.len())].join("\n") + "\n";
        let doc = super::differential::tape(
            crate::code_rule_dsl::language::dialect_by_label("python").expect("python"),
            &prefix,
        );
        // Nodes, (node, ancestor) pairs, and the descendants of each distinct node summed.
        let root = ast_grep_core::AstGrep::doc(doc.clone());
        let count = doc.transducer_len();
        let mut pairs = 0usize;
        let mut below = 0usize;
        for id in 0..count as u32 {
            let node = crate::code_rule_dsl::matcher::node_at(&root, id);
            pairs += node.ancestors().count();
            below += node.dfs().count() - 1;
        }
        let input = super::snapshot(&[("python", "a.py", &prefix)]);
        STATEMENT_PROFILE.set(Some(Default::default()));
        let started = std::time::Instant::now();
        let sql = compiled
            .evaluate(
                input.to_string(),
                "report".to_owned(),
                Some("sql".to_owned()),
            )
            .expect("sql");
        let sql_time = started.elapsed();
        let profile = STATEMENT_PROFILE.take().unwrap_or_default();
        let started = std::time::Instant::now();
        let native = compiled
            .evaluate(
                input.to_string(),
                "report".to_owned(),
                Some("native".to_owned()),
            )
            .expect("native");
        assert_eq!(native, sql);
        let mut rows: Vec<(usize, u64, String)> = profile
            .into_iter()
            .map(|(sql, (nanos, rows, _))| (rows, nanos, sql))
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.0));
        let total: usize = rows.iter().map(|row| row.0).sum();
        eprintln!(
            "{take} lines: {count} nodes, {pairs} (node, ancestor) pairs, {below} (node, descendant) pairs; sql {:.2}s, native {:.3}s, {total} rows written",
            sql_time.as_secs_f64(),
            started.elapsed().as_secs_f64()
        );
        for (rows, nanos, sql) in rows.iter().take(8) {
            let shown: String = sql.chars().take(260).collect();
            eprintln!("  {rows:>10} rows {:>8.3}s  {shown}", *nanos as f64 / 1e9);
        }
    }
}

/// The resident set size of this process, in MiB (`mach` task info on macOS; else the peak).
fn resident_mib() -> f64 {
    // SAFETY: `getrusage` fills the struct it is given.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) };
    // macOS reports bytes, Linux KiB.
    let peak = usage.ru_maxrss as f64;
    if cfg!(target_os = "macos") {
        peak / (1024.0 * 1024.0)
    } else {
        peak / 1024.0
    }
}

/// What compiling the benchmark's policies for the SQL engine costs, as the workspace compiles
/// them (`compile_each`, every AST clause of a policy in one program): per clause alone and per
/// policy, the statements, SQL bytes, time and peak RSS. `RERAIL_FOOTPRINT_POLICY` keeps the
/// policies whose name holds it. Run by hand:
/// `cargo test --release --lib sql::tests::census::compile_footprint -- --ignored --nocapture`.
#[test]
#[ignore = "reads the data-layer benchmark's policies from outside the repository"]
fn compile_footprint() {
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(run_footprint)
        .expect("a footprint thread")
        .join()
        .expect("the footprint");
}

fn run_footprint() {
    use crate::code_rule_dsl::sql::compile_each;
    let home = std::env::var("HOME").expect("HOME");
    let root = std::env::var("RERAIL_BENCH_POLICIES")
        .unwrap_or_else(|_| format!("{home}/rerail-bench/data-layer/policies"));
    let only = std::env::var("RERAIL_FOOTPRINT_POLICY").ok();
    let mut policies: Vec<_> = std::fs::read_dir(&root)
        .expect("policies")
        .filter_map(Result::ok)
        .map(|entry| entry.path().join("policy.json"))
        .filter(|path| path.exists())
        .filter(|path| {
            only.as_ref()
                .is_none_or(|only| path.to_string_lossy().contains(only.as_str()))
        })
        .collect();
    policies.sort();
    let runtimes: Vec<_> = policies
        .iter()
        .map(|path| {
            let name = path
                .parent()
                .and_then(|parent| parent.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let compiled = CodeRuleDslRuntime::compile(
                std::fs::read_to_string(path).expect("policy"),
                "json".to_owned(),
                CAPABILITIES.to_owned(),
            )
            .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason));
            (name, compiled)
        })
        .collect();
    type Grammars<'c> = Vec<(
        &'c crate::code_rule_dsl::matcher::GrammarMatcher,
        Option<&'c str>,
    )>;
    fn grammars_of(
        compiled: &crate::code_rule_dsl::CompiledCodeRuleDsl,
    ) -> (Grammars<'_>, Vec<String>) {
        let mut grammars = Vec::new();
        let mut names = Vec::new();
        for clause in &compiled.inner.clauses {
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for grammar in matchers {
                    grammars.push((grammar, clause.violation.each.as_deref()));
                    names.push(clause.id.clone());
                }
            }
        }
        (grammars, names)
    }
    // `RERAIL_FOOTPRINT_ALL`: only every policy's clauses in one program, as the whole-tree
    // workspace compiles them (`SqlEngine::new`).
    if std::env::var_os("RERAIL_FOOTPRINT_ALL").is_some() {
        let grammars: Vec<_> = runtimes
            .iter()
            .flat_map(|(_, compiled)| grammars_of(compiled).0)
            .collect();
        let before = resident_mib();
        let started = std::time::Instant::now();
        let (program, outcomes) = compile_each(&grammars);
        let elapsed = started.elapsed().as_secs_f64();
        let (stages, clauses) = program.statement_count();
        eprintln!(
            "all {} policies: {} clauses ({} lower), {stages} stage + {clauses} clause statements, {:.1} MiB SQL, {elapsed:.2}s, peak RSS {before:.0} -> {:.0} MiB",
            runtimes.len(),
            grammars.len(),
            outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
            program.statement_bytes() as f64 / 1048576.0,
            resident_mib(),
        );
        return;
    }
    for (name, compiled) in &runtimes {
        let (grammars, names) = grammars_of(compiled);
        // Each clause alone: the costliest by SQL bytes.
        let mut alone = Vec::new();
        let clause_filter = std::env::var("RERAIL_FOOTPRINT_CLAUSE").ok();
        for (grammar, name) in grammars.iter().zip(&names) {
            if clause_filter
                .as_ref()
                .is_some_and(|only| !name.contains(only.as_str()))
            {
                continue;
            }
            let started = std::time::Instant::now();
            let (program, outcomes) = compile_each(std::slice::from_ref(grammar));
            if outcomes[0].is_ok() {
                let (stages, clauses) = program.statement_count();
                alone.push((
                    program.statement_bytes(),
                    stages + clauses,
                    started.elapsed().as_secs_f64(),
                    name.clone(),
                ));
                // `RERAIL_FOOTPRINT_DUMP=<dir>`: each clause's statements, one per line.
                if let Ok(dir) = std::env::var("RERAIL_FOOTPRINT_DUMP") {
                    let text: String = program
                        .statements()
                        .map(|statement| format!("{}\n", statement.replace('\n', " ")))
                        .collect();
                    std::fs::write(format!("{dir}/{}.sql", name.replace('/', "_")), text)
                        .expect("a dump");
                }
            }
        }
        alone.sort_by_key(|entry| std::cmp::Reverse(entry.0));
        if clause_filter.is_some() {
            for (bytes, statements, seconds, clause) in &alone {
                eprintln!(
                    "{name}/{clause}: {:.2} MiB SQL, {statements} statements, {seconds:.2}s",
                    *bytes as f64 / 1048576.0
                );
            }
            continue;
        }
        let before = resident_mib();
        let started = std::time::Instant::now();
        let (program, outcomes) = compile_each(&grammars);
        let elapsed = started.elapsed().as_secs_f64();
        let (stages, clauses) = program.statement_count();
        eprintln!(
            "{name}: {} clauses ({} lower), whole program {stages} stage + {clauses} clause statements, {:.1} MiB SQL, {elapsed:.2}s, peak RSS {before:.0} -> {:.0} MiB; alone, Σ {:.1} MiB SQL",
            grammars.len(),
            outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
            program.statement_bytes() as f64 / 1048576.0,
            resident_mib(),
            alone.iter().map(|entry| entry.0).sum::<usize>() as f64 / 1048576.0,
        );
        for (bytes, statements, seconds, clause) in alone.iter().take(4) {
            eprintln!(
                "  {:>8.2} MiB {statements:>7} statements {seconds:>7.2}s  {clause}",
                *bytes as f64 / 1048576.0
            );
        }
    }
}
