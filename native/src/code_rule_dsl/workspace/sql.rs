//! The workspace on the SQL engine (`Engine::Sql`): every AST clause of the workspace's code rules
//! is compiled once into one program (`sql::compile_each`), and an apply's files are matched in
//! batches on sessions pooled across applies (`evaluate::run_jobs`). The native per-file pipeline
//! around the matching (input limits, syntax errors, required text and anchor selection, rendering,
//! suppressions, failures) is the native engine's own. A clause the engine refuses fails on every
//! file it reads with `SQL_UNSUPPORTED`, never persisted; `SqlEngine::report` says why.

use super::{ClauseId, WorkspaceState};
use crate::code_rule_dsl::eval::Engine;
use crate::code_rule_dsl::matcher::GrammarMatcher;
use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, Query};
use crate::code_rule_dsl::sql::{
    ClauseRun, Compiled, FileContext, Session, SqlError, SqlMatch, Stats, compile_each,
};
#[cfg(feature = "unbounded-work")]
use crate::code_rule_dsl::sql::{StatementProfile, profiled};
use crate::code_rule_dsl::tape::TapeDoc;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The failure of a clause the SQL engine refuses, on each file it reads.
pub(super) const SQL_UNSUPPORTED: &str = "sql-unsupported";
/// Its message; the workspace's engine report names the operator the engine refuses.
pub(super) const SQL_UNSUPPORTED_MESSAGE: &str =
    "the SQL engine does not lower this clause (see the workspace's engine report)";

/// Files per batch when the workspace's options set none.
pub(super) const DEFAULT_BATCH_SIZE: usize = 64;

/// How a workspace matches its AST clauses: the engine and, on SQL, the files per batch.
#[derive(Clone, Copy, Debug)]
pub(super) struct EngineOptions {
    pub(super) engine: Engine,
    pub(super) batch_size: usize,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            engine: Engine::Native,
            batch_size: DEFAULT_BATCH_SIZE,
        }
    }
}

impl EngineOptions {
    /// The options a workspace's host passes: `engine` is `native` (the default) or `sql`.
    pub(super) fn parse(engine: Option<&str>, batch_size: Option<u32>) -> napi::Result<Self> {
        Ok(Self {
            engine: engine.map_or(Ok(Engine::Native), Engine::parse)?,
            batch_size: batch_size.map_or(DEFAULT_BATCH_SIZE, |size| (size as usize).max(1)),
        })
    }
}

/// The workspace's SQL engine where it matches on SQL, compiled on first use.
pub(super) fn engine_of(state: &mut WorkspaceState) -> Option<Arc<SqlEngine>> {
    if state.engine.engine != Engine::Sql {
        return None;
    }
    let WorkspaceState {
        code_rules,
        engine,
        sql,
        ..
    } = state;
    Some(Arc::clone(sql.get_or_insert_with(|| {
        Arc::new(SqlEngine::new(code_rules, engine.batch_size))
    })))
}

/// A clause (one grammar matcher of it) the engine refuses, and why.
struct Refusal {
    rule: String,
    clause: String,
    language: String,
    reason: String,
}

/// The workspace's code rules compiled for the SQL engine.
pub(super) struct SqlEngine {
    compiled: Compiled,
    /// Per code rule, clause and grammar matcher (its index in the clause's matchers): the
    /// program's clause, `None` where the engine refuses it.
    clauses: HashMap<(ClauseId, usize), Option<usize>>,
    refusals: Vec<Refusal>,
    /// Which of the program's clauses ran on some file.
    ran: Vec<AtomicBool>,
    pub(super) batch_size: usize,
    /// Sessions between batches: each keeps its tables and cached statements.
    sessions: Mutex<Vec<Session>>,
    stats: Mutex<Stats>,
    /// The benchmark build's profile: `report()`'s `statements.profile` and `profile`.
    #[cfg(feature = "unbounded-work")]
    profile: Mutex<Profile>,
    /// The benchmark build's names of the program's clauses: rule and clause id.
    #[cfg(feature = "unbounded-work")]
    names: Vec<(String, String)>,
}

/// What the benchmark (`unbounded-work`) build's SQL engine measured over its batches.
#[cfg(feature = "unbounded-work")]
#[derive(Default)]
struct Profile {
    statements: StatementProfile,
    /// Per program clause and file path: the clause's time. A batch's time for a clause is split
    /// evenly over the files it ran on, so it is the file's own at batch size 1.
    clause_files: HashMap<(usize, String), u64>,
    /// Per file path: the batch's stage time, split evenly over the batch's files.
    stage_files: HashMap<String, u64>,
}

impl SqlEngine {
    pub(super) fn new(code_rules: &[Arc<CompiledCodeRule>], batch_size: usize) -> Self {
        let mut keys = Vec::new();
        let mut grammars: Vec<(&GrammarMatcher, Option<&str>)> = Vec::new();
        let mut refusals = Vec::new();
        let mut clauses = HashMap::new();
        let refuse = |refusals: &mut Vec<Refusal>, key: (ClauseId, usize), reason: String| {
            let code_rule = &code_rules[key.0.0];
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &code_rule.clauses[key.0.1].kind
            else {
                unreachable!("an AST clause")
            };
            refusals.push(Refusal {
                rule: code_rule.id.clone(),
                clause: code_rule.clauses[key.0.1].id.clone(),
                language: matchers[key.1].language.to_string(),
                reason,
            });
        };
        for (rule, code_rule) in code_rules.iter().enumerate() {
            for (clause, compiled) in code_rule.clauses.iter().enumerate() {
                let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &compiled.kind else {
                    continue;
                };
                for (index, grammar) in matchers.iter().enumerate() {
                    keys.push(((rule, clause), index));
                    grammars.push((grammar, compiled.violation.each.as_deref()));
                }
            }
        }
        let (compiled, outcomes) = compile_each(&grammars);
        for (key, outcome) in keys.into_iter().zip(outcomes) {
            match outcome {
                Ok(statement) => {
                    clauses.insert(key, Some(statement));
                }
                Err(unsupported) => {
                    clauses.insert(key, None);
                    refuse(
                        &mut refusals,
                        key,
                        SqlError::Unsupported(unsupported).to_string(),
                    );
                }
            }
        }
        // The workspace tests run on SQL show which of them hold a clause the engine refuses.
        #[cfg(test)]
        for refusal in &refusals {
            eprintln!(
                "sql-refused {}/{}: {}",
                refusal.rule, refusal.clause, refusal.reason
            );
        }
        let statements = clauses.values().flatten().count();
        #[cfg(feature = "unbounded-work")]
        let names = {
            let mut names = vec![(String::new(), String::new()); statements];
            for (((rule, clause), _), statement) in &clauses {
                if let Some(statement) = statement {
                    names[*statement] = (
                        code_rules[*rule].id.clone(),
                        code_rules[*rule].clauses[*clause].id.clone(),
                    );
                }
            }
            names
        };
        Self {
            compiled,
            clauses,
            refusals,
            ran: (0..statements).map(|_| AtomicBool::new(false)).collect(),
            batch_size: batch_size.max(1),
            sessions: Mutex::default(),
            stats: Mutex::default(),
            #[cfg(feature = "unbounded-work")]
            profile: Mutex::default(),
            #[cfg(feature = "unbounded-work")]
            names,
        }
    }

    /// The program's clause for `id`'s `grammar` (one of its matchers); `None` where the engine
    /// refuses it.
    pub(super) fn statement(
        &self,
        code_rules: &[Arc<CompiledCodeRule>],
        id: ClauseId,
        grammar: &GrammarMatcher,
    ) -> Option<usize> {
        let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &code_rules[id.0].clauses[id.1].kind
        else {
            unreachable!("an AST candidate")
        };
        let index = matchers
            .iter()
            .position(|candidate| std::ptr::eq(candidate, grammar))
            .expect("a matcher of the clause");
        self.clauses[&(id, index)]
    }

    /// Runs one batch on a pooled session (a new one where none is free): each file with what
    /// its scopes read beyond its tape. A session whose batch failed is dropped.
    pub(super) fn run(
        &self,
        docs: Vec<(TapeDoc, FileContext)>,
        runs: &[ClauseRun],
    ) -> Result<Vec<Vec<Vec<SqlMatch>>>, SqlError> {
        let pooled = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop();
        let mut session = match pooled {
            Some(session) => session,
            None => self.compiled.session()?,
        };
        let before = session.stats();
        #[cfg(feature = "unbounded-work")]
        let paths: Vec<String> = docs
            .iter()
            .map(|(_, context)| context.path.clone().unwrap_or_default())
            .collect();
        #[cfg(feature = "unbounded-work")]
        let (found, statements, clause_times) = profiled(|| session.run(docs, runs));
        #[cfg(feature = "unbounded-work")]
        let found = found?;
        #[cfg(not(feature = "unbounded-work"))]
        let found = session.run(docs, runs)?;
        for run in runs.iter().filter(|run| !run.files.is_empty()) {
            self.ran[run.statement].store(true, Ordering::Relaxed);
        }
        let after = session.stats();
        {
            let mut stats = self.stats.lock().unwrap_or_else(|error| error.into_inner());
            stats.add_since(&after, &before);
        }
        #[cfg(feature = "unbounded-work")]
        {
            let mut profile = self
                .profile
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for (sql, (nanos, rows, count)) in statements {
                let entry = profile.statements.entry(sql).or_default();
                entry.0 += nanos;
                entry.1 += rows;
                entry.2 += count;
            }
            let by_statement: HashMap<usize, &ClauseRun> =
                runs.iter().map(|run| (run.statement, run)).collect();
            for (statement, nanos) in clause_times {
                let Some(run) = by_statement.get(&statement) else {
                    continue;
                };
                let share = nanos / run.files.len().max(1) as u64;
                for (file, _) in &run.files {
                    *profile
                        .clause_files
                        .entry((statement, paths[*file].clone()))
                        .or_default() += share;
                }
            }
            let share = (after.stage_nanos - before.stage_nanos) / paths.len().max(1) as u64;
            for path in paths {
                *profile.stage_files.entry(path).or_default() += share;
            }
        }
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(session);
        Ok(found)
    }

    /// The engine's report: its clauses, those it refuses and why, those that ran, and what its
    /// statements did.
    #[cfg_attr(not(feature = "unbounded-work"), allow(clippy::let_and_return))]
    pub(super) fn report(&self) -> Value {
        let stats = *self.stats.lock().unwrap_or_else(|error| error.into_inner());
        let milliseconds = |nanos: u64| nanos as f64 / 1e6;
        let report = json!({
            "engine": "sql",
            "batchSize": self.batch_size,
            "clauses": self.clauses.len(),
            "compiled": self.ran.len(),
            "ran": self.ran.iter().filter(|ran| ran.load(Ordering::Relaxed)).count(),
            "refused": self.refusals.iter().map(|refusal| json!({
                "ruleId": refusal.rule,
                "clauseId": refusal.clause,
                "language": refusal.language,
                "reason": refusal.reason,
            })).collect::<Vec<_>>(),
            "statements": {
                "count": stats.statements,
                "rows": stats.rows,
                "milliseconds": milliseconds(stats.nanos),
                "stageMilliseconds": milliseconds(stats.stage_nanos),
                "selectMilliseconds": milliseconds(stats.select_nanos),
                "inputMilliseconds": milliseconds(stats.input_nanos),
                "factsMilliseconds": milliseconds(stats.facts_nanos),
                // What the emptiness rules passed over (`sql::skip`); `count` is what ran.
                "skipped": stats.skipped,
                "skippedRuns": stats.skipped_runs,
                "skippedBlocks": stats.skipped_blocks,
                "skippedClears": stats.skipped_clears,
            },
        });
        #[cfg(feature = "unbounded-work")]
        let report = self.profile_report(report);
        report
    }

    /// The benchmark build's profile in the report, each list sorted by time, longest first:
    /// `statements.profile`, the `PROFILED_STATEMENTS` costliest statement texts' time, rows and
    /// runs; `profile.clauses`, each clause's time over every file; `profile.clauseFiles`, the
    /// `PROFILED_CLAUSE_FILES` costliest (clause, file) times; `profile.files`, every file's time:
    /// its clauses' and its share of its batches' stage time.
    #[cfg(feature = "unbounded-work")]
    fn profile_report(&self, mut report: Value) -> Value {
        const PROFILED_STATEMENTS: usize = 200;
        const PROFILED_CLAUSE_FILES: usize = 2000;
        let profile = self
            .profile
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let milliseconds = |nanos: u64| nanos as f64 / 1e6;
        let mut statements: Vec<_> = profile.statements.iter().collect();
        statements.sort_by(|left, right| {
            (right.1)
                .0
                .cmp(&(left.1).0)
                .then_with(|| left.0.cmp(right.0))
        });
        report["statements"]["profile"] = statements
            .into_iter()
            .take(PROFILED_STATEMENTS)
            .map(|(sql, (nanos, rows, runs))| {
                json!({ "sql": sql, "milliseconds": milliseconds(*nanos), "rows": rows, "runs": runs })
            })
            .collect();
        let mut clauses: HashMap<usize, (u64, usize)> = HashMap::new();
        let mut files: HashMap<&str, u64> = profile
            .stage_files
            .iter()
            .map(|(path, nanos)| (path.as_str(), *nanos))
            .collect();
        for ((statement, path), nanos) in &profile.clause_files {
            let entry = clauses.entry(*statement).or_default();
            entry.0 += nanos;
            entry.1 += 1;
            *files.entry(path.as_str()).or_default() += nanos;
        }
        let mut clauses: Vec<_> = clauses.into_iter().collect();
        clauses.sort_by_key(|(statement, (nanos, _))| (std::cmp::Reverse(*nanos), *statement));
        let mut clause_files: Vec<_> = profile.clause_files.iter().collect();
        clause_files.sort_by(|left, right| right.1.cmp(left.1).then_with(|| left.0.cmp(right.0)));
        let mut files: Vec<_> = files.into_iter().collect();
        files.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
        report["profile"] = json!({
            "clauses": clauses.into_iter().map(|(statement, (nanos, files))| json!({
                "ruleId": self.names[statement].0,
                "clauseId": self.names[statement].1,
                "statement": statement,
                "milliseconds": milliseconds(nanos),
                "files": files,
            })).collect::<Vec<_>>(),
            "clauseFiles": clause_files.into_iter().take(PROFILED_CLAUSE_FILES).map(|((statement, path), nanos)| json!({
                "ruleId": self.names[*statement].0,
                "clauseId": self.names[*statement].1,
                "path": path,
                "milliseconds": milliseconds(*nanos),
            })).collect::<Vec<_>>(),
            "files": files.into_iter().map(|(path, nanos)| json!({
                "path": path,
                "milliseconds": milliseconds(nanos),
            })).collect::<Vec<_>>(),
        });
        report
    }
}

#[cfg(test)]
mod tests {
    use super::super::api::{CodeRuleDslWorkspace, WorkspaceApplyTask, WorkspaceOptions};
    use crate::code_rule_dsl::CodeRuleDslRuntime;
    use crate::code_rule_dsl::test_support::CAPABILITIES;
    use napi::Task;
    use serde_json::{Value, json};
    use std::sync::Arc;

    /// This process's peak resident set size, in MiB.
    fn peak_mib() -> f64 {
        // SAFETY: `getrusage` fills the struct it is given.
        let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &raw mut usage) };
        // macOS reports bytes, Linux KiB.
        let peak = usage.ru_maxrss as f64;
        if cfg!(target_os = "macos") {
            peak / 1048576.0
        } else {
            peak / 1024.0
        }
    }

    /// The workspace's SQL engine over a list of the data-layer corpus's files, every benchmark
    /// policy in one workspace as the whole-tree benchmark runs them: `RERAIL_SQL_FILES`, one path
    /// per line relative to `RERAIL_DATA_LAYER_CORPUS`; `RERAIL_SQL_BATCH`, files per batch
    /// (default 64; the workspace caps it at the files per worker). Prints the apply's wall time,
    /// the engine report and the peak RSS. Run by hand:
    /// `cargo test --release --lib workspace::sql::tests -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the data-layer benchmark's policies and corpus from outside the repository"]
    fn the_sql_engine_over_a_file_list() {
        let home = std::env::var("HOME").expect("HOME");
        let corpus = std::env::var("RERAIL_DATA_LAYER_CORPUS")
            .unwrap_or_else(|_| format!("{home}/rerail-bench/nera-data-layer-fce4dc72"));
        let policies = std::env::var("RERAIL_BENCH_POLICIES")
            .unwrap_or_else(|_| format!("{home}/rerail-bench/data-layer/policies"));
        let list = std::env::var("RERAIL_SQL_FILES")
            .unwrap_or_else(|_| format!("{home}/rerail-bench/distill/subset.txt"));
        let batch = std::env::var("RERAIL_SQL_BATCH")
            .ok()
            .map(|size| size.parse().expect("RERAIL_SQL_BATCH is a number"));
        let started = std::time::Instant::now();
        let mut workspace = CodeRuleDslWorkspace::with_options(Some(WorkspaceOptions {
            engine: Some("sql".to_owned()),
            sql_batch_size: batch,
        }))
        .expect("a SQL workspace");
        let mut paths: Vec<_> = std::fs::read_dir(&policies)
            .expect("policies")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("policy.json"))
            .filter(|path| path.exists())
            .collect();
        paths.sort();
        for path in &paths {
            let compiled = CodeRuleDslRuntime::compile(
                std::fs::read_to_string(path).expect("policy"),
                "json".to_owned(),
                CAPABILITIES.to_owned(),
            )
            .unwrap_or_else(|error| panic!("{}: {}", path.display(), error.reason));
            workspace
                .add_code_rule(&compiled, None)
                .expect("a code rule");
        }
        let root = std::path::Path::new(&corpus);
        let files: Vec<Value> = std::fs::read_to_string(&list)
            .expect("the file list")
            .lines()
            .filter(|path| !path.is_empty())
            .filter(|path| workspace.accepts_path((*path).to_owned()))
            .filter_map(|path| {
                let absolute = root.join(path);
                let source = String::from_utf8(std::fs::read(&absolute).ok()?).ok()?;
                let mut file = json!({ "language": "python", "path": path, "source": source });
                if let Some(module) =
                    crate::code_rule_dsl::language::adapter::module_path("python", &absolute)
                {
                    file["module"] = module;
                }
                Some(file)
            })
            .collect();
        let setup = started.elapsed().as_secs_f64();
        let before = peak_mib();
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: json!({ "revision": 1, "files": files, "removed": [] }).to_string(),
            request_buffer: None,
            request_direct: None,
        };
        let started = std::time::Instant::now();
        let response: Value =
            serde_json::from_str(&task.compute().expect("the apply")).expect("a response");
        let apply = started.elapsed().as_secs_f64();
        let report: Value =
            serde_json::from_str(&workspace.engine_report().expect("a report")).expect("JSON");
        let statements = &report["statements"];
        eprintln!(
            "files {}, batch {}, setup {setup:.1}s, apply {apply:.2}s, violations {}, failures {}, peak RSS {before:.0} -> {:.0} MiB",
            files.len(),
            report["batchSize"],
            response["violations"].as_array().map_or(0, Vec::len),
            response["failures"].as_array().map_or(0, Vec::len),
            peak_mib(),
        );
        eprintln!("statements {statements}");
    }
}
