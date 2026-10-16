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
    ClauseRun, Compiled, Session, SqlError, SqlMatch, Stats, compile_each,
};
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
}

impl SqlEngine {
    pub(super) fn new(code_rules: &[Arc<CompiledCodeRule>], batch_size: usize) -> Self {
        let mut keys = Vec::new();
        let mut grammars: Vec<&GrammarMatcher> = Vec::new();
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
                    let key = ((rule, clause), index);
                    // `violation.each` re-matches each root natively (`matcher::enumerate`).
                    if compiled.violation.each.is_some() {
                        clauses.insert(key, None);
                        refuse(
                            &mut refusals,
                            key,
                            SqlError::unsupported("violation.each").to_string(),
                        );
                        continue;
                    }
                    keys.push(key);
                    grammars.push(grammar);
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
        Self {
            compiled,
            clauses,
            refusals,
            ran: (0..statements).map(|_| AtomicBool::new(false)).collect(),
            batch_size: batch_size.max(1),
            sessions: Mutex::default(),
            stats: Mutex::default(),
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

    /// Runs one batch on a pooled session (a new one where none is free). A session whose batch
    /// failed is dropped.
    pub(super) fn run(
        &self,
        docs: Vec<TapeDoc>,
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
        let found = session.run(docs, runs)?;
        for run in runs.iter().filter(|run| !run.files.is_empty()) {
            self.ran[run.statement].store(true, Ordering::Relaxed);
        }
        let after = session.stats();
        {
            let mut stats = self.stats.lock().unwrap_or_else(|error| error.into_inner());
            stats.statements += after.statements - before.statements;
            stats.rows += after.rows - before.rows;
            stats.nanos += after.nanos - before.nanos;
            stats.stage_nanos += after.stage_nanos - before.stage_nanos;
            stats.select_nanos += after.select_nanos - before.select_nanos;
            stats.input_nanos += after.input_nanos - before.input_nanos;
            stats.facts_nanos += after.facts_nanos - before.facts_nanos;
        }
        self.sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(session);
        Ok(found)
    }

    /// The engine's report: its clauses, those it refuses and why, those that ran, and what its
    /// statements did.
    pub(super) fn report(&self) -> Value {
        let stats = *self.stats.lock().unwrap_or_else(|error| error.into_inner());
        let milliseconds = |nanos: u64| nanos as f64 / 1e6;
        json!({
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
            },
        })
    }
}
