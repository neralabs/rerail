//! The workspace on the SQL engine (`Engine::Sql`): every AST clause of the workspace's code rules
//! is compiled into one program as the rules are registered (`register`, from `add_code_rule`),
//! the program is finished on first use, and an apply's files are matched in batches on sessions
//! pooled across applies (`evaluate::run_jobs`). The native per-file pipeline around the matching
//! (input limits, syntax errors, required text and anchor selection, rendering, suppressions,
//! failures) is the native engine's own. A clause the engine refuses fails on every file it reads
//! with `SQL_UNSUPPORTED`, never persisted; `SqlEngine::report` says why.

use super::{ClauseId, WorkspaceState};
use crate::code_rule_dsl::eval::Engine;
use crate::code_rule_dsl::matcher::GrammarMatcher;
use crate::code_rule_dsl::model::{ClauseKind, CompiledCodeRule, Query};
use crate::code_rule_dsl::sql::{
    ClauseRun, CompilePhases, Compiled, FileContext, ProgramCompiler, Session, SqlError, SqlMatch,
    Stats, Timing,
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

use crate::code_rule_dsl::sql::DEFAULT_BATCH_SIZE;

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

/// The workspace's SQL program: compiled as its rules are registered, then finished into the
/// engine its applies run on.
pub(super) enum SqlState {
    Compiling(Box<SqlProgram>),
    Finished(Arc<SqlEngine>),
}

impl Default for SqlState {
    fn default() -> Self {
        Self::Compiling(Box::default())
    }
}

/// Compiles the AST clauses of the code rule `rule` (the last registered) into the workspace's
/// SQL program, where it matches on SQL: a workspace pays its rules' SQL compile when it registers
/// them, as it pays their native compile, never in an apply. A program finished before (an engine
/// report between registrations) is compiled again with every rule.
pub(super) fn register(state: &mut WorkspaceState, rule: usize) {
    if state.engine.engine != Engine::Sql {
        return;
    }
    match &mut state.sql {
        SqlState::Compiling(program) => program.add(&state.code_rules, rule),
        SqlState::Finished(_) => {
            let mut program = Box::<SqlProgram>::default();
            for rule in 0..=rule {
                program.add(&state.code_rules, rule);
            }
            state.sql = SqlState::Compiling(program);
        }
    }
}

/// The workspace's SQL engine where it matches on SQL, its program finished on first use.
pub(super) fn engine_of(state: &mut WorkspaceState) -> Option<Arc<SqlEngine>> {
    if state.engine.engine != Engine::Sql {
        return None;
    }
    if let SqlState::Compiling(program) = &mut state.sql {
        let program = std::mem::take(program);
        state.sql = SqlState::Finished(Arc::new(program.finish(state.engine.batch_size)));
    }
    match &state.sql {
        SqlState::Finished(engine) => Some(Arc::clone(engine)),
        SqlState::Compiling(_) => unreachable!("finished above"),
    }
}

/// A clause (one grammar matcher of it) the engine refuses, and why.
struct Refusal {
    rule: String,
    clause: String,
    language: String,
    reason: String,
}

/// The registered code rules' AST clauses, compiled into one SQL program so far.
#[derive(Default)]
pub(super) struct SqlProgram {
    compiler: ProgramCompiler,
    /// Per code rule, clause and grammar matcher (its index in the clause's matchers): the
    /// program's clause, `None` where the engine refuses it.
    clauses: HashMap<(ClauseId, usize), Option<usize>>,
    refusals: Vec<Refusal>,
    /// The names of the program's clauses: rule and clause id.
    names: Vec<(String, String)>,
}

impl SqlProgram {
    /// Compiles every AST clause of `code_rules[rule]`.
    fn add(&mut self, code_rules: &[Arc<CompiledCodeRule>], rule: usize) {
        let code_rule = &code_rules[rule];
        for (clause, compiled) in code_rule.clauses.iter().enumerate() {
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &compiled.kind else {
                continue;
            };
            for (index, grammar) in matchers.iter().enumerate() {
                let outcome = self
                    .compiler
                    .clause(grammar, compiled.violation.each.as_deref());
                let statement = match outcome {
                    Ok(statement) => {
                        self.names.push((code_rule.id.clone(), compiled.id.clone()));
                        Some(statement)
                    }
                    Err(unsupported) => {
                        // The workspace tests run on SQL show which of them hold a clause the
                        // engine refuses.
                        let reason = SqlError::Unsupported(unsupported).to_string();
                        #[cfg(test)]
                        eprintln!("sql-refused {}/{}: {reason}", code_rule.id, compiled.id);
                        self.refusals.push(Refusal {
                            rule: code_rule.id.clone(),
                            clause: compiled.id.clone(),
                            language: grammar.language.to_string(),
                            reason,
                        });
                        None
                    }
                };
                self.clauses.insert(((rule, clause), index), statement);
            }
        }
    }

    /// The engine of the program as compiled, running `batch_size` files per batch.
    fn finish(self, batch_size: usize) -> SqlEngine {
        let Self {
            compiler,
            clauses,
            refusals,
            names,
        } = self;
        let (compiled, phases) = compiler.finish();
        SqlEngine {
            compiled,
            phases,
            clauses,
            refusals,
            ran: (0..names.len()).map(|_| AtomicBool::new(false)).collect(),
            batch_size: batch_size.max(1),
            sessions: Mutex::default(),
            stats: Mutex::default(),
            pipeline: Mutex::default(),
            #[cfg(feature = "unbounded-work")]
            profile: Mutex::default(),
            names,
        }
    }
}

/// The workspace's code rules compiled for the SQL engine.
pub(super) struct SqlEngine {
    compiled: Compiled,
    /// Where the program's compile went.
    phases: CompilePhases,
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
    /// The native per-file steps around the batches' matching (`evaluate::sql_batch`).
    pipeline: Mutex<Pipeline>,
    /// The benchmark build's profile: `report()`'s `statements.profile` and `profile`.
    #[cfg(feature = "unbounded-work")]
    profile: Mutex<Profile>,
    /// The names of the program's clauses: rule and clause id.
    #[cfg_attr(not(feature = "unbounded-work"), allow(dead_code))]
    names: Vec<(String, String)>,
}

/// The time an apply's batches spent in the native per-file steps around their SQL run, summed
/// over the workers: restoring each file's document (its parse and tape where none is stored),
/// planning its clauses' runs (`SqlFile::plan`: anchor index and root selection), and rendering
/// its matches (`SqlFile::render`).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Pipeline {
    pub(super) parse: Timing,
    pub(super) plan: Timing,
    pub(super) render: Timing,
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
    /// Adds a batch's native per-file steps (`Pipeline`).
    pub(super) fn add_pipeline(&self, batch: Pipeline) {
        let mut pipeline = self
            .pipeline
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        pipeline.parse.add(batch.parse);
        pipeline.plan.add(batch.plan);
        pipeline.render.add(batch.render);
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
    /// its scopes read beyond its tape. A session whose batch failed is emptied by it, and pooled
    /// as any other: sessions are made once per worker, and kept across batches and applies.
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
        // A new session's setup counts in the engine's statistics.
        let (mut session, before) = match pooled {
            Some(session) => {
                let before = session.stats();
                (session, before)
            }
            None => (self.compiled.session()?, Stats::default()),
        };
        #[cfg(feature = "unbounded-work")]
        let paths: Vec<String> = docs
            .iter()
            .map(|(_, context)| context.path.clone().unwrap_or_default())
            .collect();
        #[cfg(feature = "unbounded-work")]
        let (found, statements, clause_times) = profiled(|| session.run(docs, runs));
        #[cfg(not(feature = "unbounded-work"))]
        let found = session.run(docs, runs);
        let after = session.stats();
        {
            let mut stats = self.stats.lock().unwrap_or_else(|error| error.into_inner());
            stats.add_since(&after, &before);
        }
        let found = match found {
            Ok(found) => found,
            Err(error) => {
                self.sessions
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(session);
                return Err(error);
            }
        };
        for run in runs.iter().filter(|run| !run.files.is_empty()) {
            self.ran[run.statement].store(true, Ordering::Relaxed);
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
            let share = after.stages.since(before.stages).wall / paths.len().max(1) as u64;
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
        let census = self.compiled.census();
        let prepared: usize = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .map(Session::prepared_statements)
            .sum();
        let pipeline = *self
            .pipeline
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let phases = &self.phases;
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
            // Each timer twice, summed over the workers: `…Milliseconds` wall time and
            // `…CpuMilliseconds` the threads' CPU time.
            "statements": {
                "count": stats.statements,
                "rows": stats.rows,
                "milliseconds": stats.time.wall_milliseconds(),
                "cpuMilliseconds": stats.time.cpu_milliseconds(),
                "stageMilliseconds": stats.stages.wall_milliseconds(),
                "stageCpuMilliseconds": stats.stages.cpu_milliseconds(),
                "selectMilliseconds": stats.select.wall_milliseconds(),
                "selectCpuMilliseconds": stats.select.cpu_milliseconds(),
                "inputMilliseconds": stats.input.wall_milliseconds(),
                "inputCpuMilliseconds": stats.input.cpu_milliseconds(),
                // The fact terminals' native evaluation, inside the statements' time.
                "factsMilliseconds": stats.facts.wall_milliseconds(),
                "factsCpuMilliseconds": stats.facts.cpu_milliseconds(),
                // The batches' files' node facts, before their statements.
                "nodeFactsMilliseconds": stats.node_facts.wall_milliseconds(),
                "nodeFactsCpuMilliseconds": stats.node_facts.cpu_milliseconds(),
                // What the emptiness rules passed over (`sql::skip`); `count` is what ran.
                "skipped": stats.skipped,
                "skippedRuns": stats.skipped_runs,
                "skippedBlocks": stats.skipped_blocks,
                "skippedClears": stats.skipped_clears,
            },
            // The program as sessions run it: its steps, their distinct texts and those texts'
            // bytes, and its tables; and as it is stored: its stages' and clauses' distinct
            // shapes, and its texts' distinct skeletons and the bytes they are stored in.
            "program": {
                "steps": census.steps,
                "texts": census.texts,
                "bytes": census.bytes,
                "frames": census.frames,
                "btrees": census.btrees,
                "shapes": census.shapes,
                "skeletons": census.skeletons,
                "storedBytes": census.stored,
            },
            // The program's compile, as the workspace's rules were registered (`finish` on
            // first use): lowering, emitting the statements (the stages' among them), planning
            // each clause's, and planning the stages.
            "compile": {
                "lowerMilliseconds": phases.lower.wall_milliseconds(),
                "lowerCpuMilliseconds": phases.lower.cpu_milliseconds(),
                "emitMilliseconds": phases.emit.wall_milliseconds(),
                "emitCpuMilliseconds": phases.emit.cpu_milliseconds(),
                "planMilliseconds": phases.plan.wall_milliseconds(),
                "planCpuMilliseconds": phases.plan.cpu_milliseconds(),
                "finishMilliseconds": phases.finish.wall_milliseconds(),
                "finishCpuMilliseconds": phases.finish.cpu_milliseconds(),
            },
            // The sessions made and their setup (`sql::Session::activate`): the texts their
            // caches were sized for, the tables they made, and the statements the pooled ones
            // hold prepared.
            "sessions": {
                "count": stats.sessions,
                "setupMilliseconds": stats.setup.wall_milliseconds(),
                "setupCpuMilliseconds": stats.setup.cpu_milliseconds(),
                "texts": stats.texts,
                "tables": stats.tables,
                "prepared": prepared,
            },
            // The native per-file steps around the batches' SQL runs (`Pipeline`).
            "pipeline": {
                "parseMilliseconds": pipeline.parse.wall_milliseconds(),
                "parseCpuMilliseconds": pipeline.parse.cpu_milliseconds(),
                "planMilliseconds": pipeline.plan.wall_milliseconds(),
                "planCpuMilliseconds": pipeline.plan.cpu_milliseconds(),
                "renderMilliseconds": pipeline.render.wall_milliseconds(),
                "renderCpuMilliseconds": pipeline.render.cpu_milliseconds(),
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

    /// This process's resident set now, in MiB (macOS; 0 elsewhere).
    fn resident_mib() -> f64 {
        #[cfg(target_os = "macos")]
        {
            // SAFETY: `proc_pidinfo` fills the struct it is given, of the size given.
            let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
            let filled = unsafe {
                libc::proc_pidinfo(
                    libc::getpid(),
                    libc::PROC_PIDTASKINFO,
                    0,
                    (&raw mut info).cast(),
                    size,
                )
            };
            if filled == size {
                return info.pti_resident_size as f64 / 1048576.0;
            }
        }
        0.0
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
        eprintln!("program {}", report["program"]);
        eprintln!("sessions {}", report["sessions"]);
    }

    /// The SQL pipeline's per-file plan (`SqlFile::plan`) against the native dispatch's per-file
    /// preparation it mirrors (`evaluate::NATIVE_PLAN`), thread CPU summed over the workers, per
    /// unit of a loop record (`RERAIL_LOOP_RECORD`, its last round's SQL units, those of the
    /// `RERAIL_PLAN_LINTERS` linters, comma-separated, where set) over the loop's subset: each
    /// unit's rule as the loop builds it (the policy's clauses of the unit, and its definitions),
    /// applied on a native and a SQL workspace. Run by hand:
    /// `cargo test --profile loop --lib workspace::sql::tests::per_file_plan_against_native --
    /// --ignored --nocapture`.
    #[test]
    #[ignore = "reads the data-layer benchmark's policies, corpus and loop record from outside the repository"]
    fn per_file_plan_against_native() {
        use super::super::evaluate::NATIVE_PLAN;
        let home = std::env::var("HOME").expect("HOME");
        let corpus = std::env::var("RERAIL_DATA_LAYER_CORPUS")
            .unwrap_or_else(|_| format!("{home}/rerail-bench/nera-data-layer-fce4dc72"));
        let policies = std::env::var("RERAIL_BENCH_POLICIES")
            .unwrap_or_else(|_| format!("{home}/rerail-bench/data-layer/policies"));
        let list = std::env::var("RERAIL_SQL_FILES")
            .unwrap_or_else(|_| format!("{home}/rerail-bench/distill/subset.txt"));
        let record: Value = serde_json::from_str(
            &std::fs::read_to_string(std::env::var("RERAIL_LOOP_RECORD").expect("a loop record"))
                .expect("the loop record"),
        )
        .expect("JSON");
        let linters: Option<Vec<String>> = std::env::var("RERAIL_PLAN_LINTERS")
            .ok()
            .map(|list| list.split(',').map(str::to_owned).collect());
        let root = std::path::Path::new(&corpus);
        let sources: Vec<(String, String)> = std::fs::read_to_string(&list)
            .expect("the file list")
            .lines()
            .filter(|path| !path.is_empty())
            .filter_map(|path| {
                let source = String::from_utf8(std::fs::read(root.join(path)).ok()?).ok()?;
                Some((path.to_owned(), source))
            })
            .collect();
        let units = record["rounds"]
            .as_array()
            .and_then(|rounds| rounds.last())
            .and_then(|round| round["sql"].as_array())
            .expect("the record's units");
        let mut totals: std::collections::BTreeMap<String, (f64, f64, usize, f64)> =
            std::collections::BTreeMap::new();
        for unit in units {
            let linter = unit["unit"]["linter"]
                .as_str()
                .expect("a linter")
                .to_owned();
            if linters
                .as_ref()
                .is_some_and(|linters| !linters.contains(&linter))
            {
                continue;
            }
            let ids: std::collections::HashSet<&str> = unit["unit"]["clauses"]
                .as_array()
                .expect("clauses")
                .iter()
                .map(|name| &name.as_str().expect("a clause")[linter.len() + 1..])
                .collect();
            let mut document: Value = serde_json::from_str(
                &std::fs::read_to_string(format!("{policies}/{linter}/policy.json"))
                    .expect("policy"),
            )
            .expect("JSON");
            let clauses: Vec<Value> = document["clauses"]
                .as_array()
                .expect("clauses")
                .iter()
                .filter(|clause| {
                    clause.get("define").is_some()
                        || clause["id"].as_str().is_some_and(|id| ids.contains(id))
                })
                .cloned()
                .collect();
            document["clauses"] = Value::Array(clauses);
            let compiled = CodeRuleDslRuntime::compile(
                document.to_string(),
                "json".to_owned(),
                CAPABILITIES.to_owned(),
            )
            .unwrap_or_else(|error| panic!("{linter}: {}", error.reason));
            let mut plan = [0.0; 2];
            let mut postings = 0.0;
            for (side, engine) in ["native", "sql"].into_iter().enumerate() {
                let mut workspace = CodeRuleDslWorkspace::with_options(Some(WorkspaceOptions {
                    engine: Some(engine.to_owned()),
                    sql_batch_size: None,
                }))
                .expect("a workspace");
                workspace
                    .add_code_rule(&compiled, None)
                    .expect("a code rule");
                let files: Vec<Value> = sources
                    .iter()
                    .filter(|(path, _)| workspace.accepts_path(path.clone()))
                    .map(|(path, source)| {
                        let mut file =
                            json!({ "language": "python", "path": path, "source": source });
                        if let Some(module) = crate::code_rule_dsl::language::adapter::module_path(
                            "python",
                            &root.join(path),
                        ) {
                            file["module"] = module;
                        }
                        file
                    })
                    .collect();
                let native_before = *NATIVE_PLAN.lock().expect("the native plan");
                let postings_before = *crate::code_rule_dsl::sql::POSTINGS_TIME
                    .lock()
                    .expect("postings");
                let mut task = WorkspaceApplyTask {
                    state: Arc::clone(&workspace.state),
                    closed: Arc::clone(&workspace.closed),
                    request_json: json!({ "revision": 1, "files": files, "removed": [] })
                        .to_string(),
                    request_buffer: None,
                    request_direct: None,
                };
                task.compute().expect("the apply");
                plan[side] = if engine == "native" {
                    NATIVE_PLAN
                        .lock()
                        .expect("the native plan")
                        .since(native_before)
                        .cpu_milliseconds()
                } else {
                    postings = crate::code_rule_dsl::sql::POSTINGS_TIME
                        .lock()
                        .expect("postings")
                        .since(postings_before)
                        .cpu_milliseconds();
                    let report: Value =
                        serde_json::from_str(&workspace.engine_report().expect("a report"))
                            .expect("JSON");
                    report["pipeline"]["planCpuMilliseconds"]
                        .as_f64()
                        .expect("the SQL plan")
                };
            }
            let loop_plan = unit["phases"]["timers"]["pipeline.planCpuMilliseconds"]
                .as_f64()
                .unwrap_or(0.0);
            eprintln!(
                "plan native {:8.1} ms, SQL {:8.1} ms ({:+6.1}%), loop SQL {loop_plan:8.1} ms, SQL postings {postings:7.1} ms  {linter} ({} clauses: {})",
                plan[0],
                plan[1],
                (plan[1] / plan[0] - 1.0) * 100.0,
                ids.len(),
                unit["unit"]["clauses"][0].as_str().unwrap_or_default(),
            );
            let total = totals.entry(linter).or_default();
            total.0 += plan[0];
            total.1 += plan[1];
            total.2 += 1;
            total.3 += postings;
        }
        let (mut native, mut sql) = (0.0, 0.0);
        for (linter, (linter_native, linter_sql, count, linter_postings)) in &totals {
            eprintln!(
                "total native {linter_native:9.1} ms, SQL {linter_sql:9.1} ms ({:+6.1}%), SQL postings {linter_postings:8.1} ms  {linter} ({count} units)",
                (linter_sql / linter_native - 1.0) * 100.0
            );
            native += linter_native;
            sql += linter_sql;
        }
        eprintln!(
            "total native {native:9.1} ms, SQL {sql:9.1} ms ({:+6.1}%)",
            (sql / native - 1.0) * 100.0
        );
    }

    /// The bytes jemalloc holds allocated now, in MiB (0 where it is not the allocator).
    fn allocated_mib() -> f64 {
        #[cfg(not(windows))]
        {
            let _ = tikv_jemalloc_ctl::epoch::advance();
            tikv_jemalloc_ctl::stats::allocated::read().unwrap_or(0) as f64 / 1048576.0
        }
        #[cfg(windows)]
        0.0
    }

    /// What registering every benchmark policy on a SQL workspace costs: their native compile,
    /// then their SQL compile as `add_code_rule` runs it, by phase (`compile` of the engine
    /// report), with peak and current RSS and the bytes allocated after each, and a first
    /// apply's session. The test build's check of each shape (`sql::VERIFY_SHAPES`) is off: it is
    /// no cost of the compile. Run by hand:
    /// `cargo test --release --lib workspace::sql::tests::the_sql_program_compile -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore = "reads the data-layer benchmark's policies from outside the repository"]
    fn the_sql_program_compile() {
        crate::code_rule_dsl::sql::VERIFY_SHAPES.store(false, std::sync::atomic::Ordering::Relaxed);
        let home = std::env::var("HOME").expect("HOME");
        let policies = std::env::var("RERAIL_BENCH_POLICIES")
            .unwrap_or_else(|_| format!("{home}/rerail-bench/data-layer/policies"));
        let mut workspace = CodeRuleDslWorkspace::with_options(Some(WorkspaceOptions {
            engine: Some("sql".to_owned()),
            sql_batch_size: None,
        }))
        .expect("a SQL workspace");
        let mut paths: Vec<_> = std::fs::read_dir(&policies)
            .expect("policies")
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("policy.json"))
            .filter(|path| path.exists())
            .collect();
        paths.sort();
        let started = std::time::Instant::now();
        let compiled: Vec<_> = paths
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
        eprintln!(
            "native compile {:.2}s, peak {:.0} MiB, resident {:.0} MiB, allocated {:.0} MiB",
            started.elapsed().as_secs_f64(),
            peak_mib(),
            resident_mib(),
            allocated_mib(),
        );
        let started = std::time::Instant::now();
        for compiled in &compiled {
            workspace
                .add_code_rule(compiled, None)
                .expect("a code rule");
        }
        eprintln!(
            "add rules (SQL compile) {:.2}s, peak {:.0} MiB, resident {:.0} MiB, allocated {:.0} MiB",
            started.elapsed().as_secs_f64(),
            peak_mib(),
            resident_mib(),
            allocated_mib(),
        );
        let started = std::time::Instant::now();
        let report: Value =
            serde_json::from_str(&workspace.engine_report().expect("a report")).expect("JSON");
        eprintln!(
            "finish {:.2}s, peak {:.0} MiB, resident {:.0} MiB, allocated {:.0} MiB",
            started.elapsed().as_secs_f64(),
            peak_mib(),
            resident_mib(),
            allocated_mib(),
        );
        eprintln!("compile {}", report["compile"]);
        eprintln!("program {}", report["program"]);
        let started = std::time::Instant::now();
        let source = "import os\n\nclass Store(Base):\n    async def get(self, key):\n        if hasattr(self, key):\n            return await self.session.execute(key)\n";
        let mut task = WorkspaceApplyTask {
            state: Arc::clone(&workspace.state),
            closed: Arc::clone(&workspace.closed),
            request_json: json!({ "revision": 1, "files": [{ "language": "python", "path": "probe.py", "source": source }], "removed": [] }).to_string(),
            request_buffer: None,
            request_direct: None,
        };
        task.compute().expect("the apply");
        let report: Value =
            serde_json::from_str(&workspace.engine_report().expect("a report")).expect("JSON");
        eprintln!(
            "first apply {:.2}s, peak {:.0} MiB; sessions {}",
            started.elapsed().as_secs_f64(),
            peak_mib(),
            report["sessions"]
        );
    }
}
