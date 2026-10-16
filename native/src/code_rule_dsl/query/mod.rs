//! The query engine: rules run as SQLite statements over virtual tables.
//!
//! SQLite drives execution. The tables do the work SQL cannot:
//! - `rerail_files`: the file versions of a batch;
//! - `rerail_matches`: terminal rows (transducer programs, ast-grep patterns, kinds), executed once
//!   per batch and terminal and answered from that cache (`transducer::sql_backend`);
//! - `rerail_nodes`: the syntax tree of a file or derived document, walked along an axis in the
//!   matcher's order;
//! - `rerail_facts`: the semantic facts a Rerail predicate reads at a node (a reference's binding,
//!   the binding's site, value, annotation and occurrences, a format call's arguments, a parsed
//!   literal), computed once per node and environment and cached for the batch. Sub-rules over
//!   them are ordinary SQL.
//!
//! Functions test one node a statement already holds (`compile.rs` lists them); they never
//! produce node sets. Capture-free structural sub-rules are answered by a native kernel
//! (`structure`) that remembers each file's answers.

mod catalog;
mod compile;
pub(in crate::code_rule_dsl) mod crossfile;
mod docs;
mod env;
mod facts;
mod functions;
mod recursion;
pub(in crate::code_rule_dsl) mod select_when;
mod structure;
mod tables;

use crate::code_rule_dsl::language::GrammarDialect;
#[cfg(test)]
use crate::code_rule_dsl::transducer::sql_backend::BatchCounters;
use crate::code_rule_dsl::transducer::sql_backend::{
    BatchFile, MatchRegistry, Terminal, TextSource, register_module,
};
pub(super) use catalog::{Catalog, CatalogBuilder};
pub(super) use compile::{ClausePlan, compile_clause};
pub(super) use env::Env;
use env::EnvArena;
use rusqlite::Connection;
use std::sync::Arc;

type TextRows = Vec<(Option<(usize, usize)>, bool)>;
type UtilityMemoKey = (i64, i64, i64, i64, u32);
type UtilityMemoValue = (Option<i64>, usize);
type SiblingKey = (i64, u32, Option<i64>);
#[cfg(test)]
thread_local! {
    static ROWS_STEPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
#[cfg(test)]
static MAX_REGISTERED_FILES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
pub(super) fn max_registered_files() -> usize {
    MAX_REGISTERED_FILES.load(std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
#[allow(dead_code, reason = "retained for SQL profiling assertions")]
pub(super) fn reset_rows_stepped() {
    ROWS_STEPPED.with(|rows| rows.set(0));
}

#[cfg(test)]
#[allow(dead_code, reason = "retained for SQL profiling assertions")]
pub(super) fn rows_stepped() -> usize {
    ROWS_STEPPED.with(std::cell::Cell::get)
}
/// A predicate's facts at one node: `(term, document, node, environment)`.
type FactKey = (usize, i64, u32, i64);

/// What a batch's functions read: the catalog its statements index, and its environments.
pub(super) struct BatchState {
    catalog: Arc<Catalog>,
    envs: EnvArena,
    statement_executions: std::sync::atomic::AtomicUsize,
    rows: std::sync::atomic::AtomicUsize,
    text_rows: std::sync::Mutex<foldhash::HashMap<usize, Arc<Vec<TextRows>>>>,
    /// Recursive utility verdicts are pure for a fixed node, environment and recursion depth.
    /// The cached cost is replayed so `maxVisitedNodes` remains observable.
    util_memo: std::sync::Mutex<foldhash::HashMap<UtilityMemoKey, UtilityMemoValue>>,
    /// The roots an anchor index selected for the statement running now (`rerail_nodes`'
    /// `selected = 1`); `None` when it selected every root.
    roots: std::sync::Mutex<Option<Arc<[u32]>>>,
    batch_roots: std::sync::Mutex<Vec<Option<Arc<[u32]>>>>,
    /// The literals parsed so far, as documents past the batch's files (`docs`).
    derived: std::sync::Mutex<Vec<Arc<docs::Derived>>>,
    /// Each predicate's facts, by node and environment (`rerail_facts`).
    facts: std::sync::Mutex<foldhash::HashMap<FactKey, Arc<facts::Facts>>>,
    /// The structural kernel's answers so far, by file.
    structure: std::sync::Mutex<foldhash::HashMap<i64, structure::Memo>>,
    /// `nthChild`'s counted siblings by file, parent ordinal and, with an `ofRule`, its term:
    /// listed once for every clause and candidate of the batch, and dropped with it.
    siblings: parking_lot::Mutex<foldhash::HashMap<SiblingKey, Arc<functions::Siblings>>>,
    /// A parent's children by their class under an `eq` comparison, by file, parent ordinal and
    /// the comparison's term (`tq_same_key`): classified once for every candidate of the batch.
    same_keys: parking_lot::Mutex<foldhash::HashMap<(i64, u32, i64), Arc<functions::SameKeys>>>,
}

impl BatchState {
    fn selected_roots(&self, file: i64) -> Option<Arc<[u32]>> {
        let batch = self
            .batch_roots
            .lock()
            .expect("batch root selection mutex poisoned");
        if !batch.is_empty() {
            return usize::try_from(file)
                .ok()
                .and_then(|file| batch.get(file).cloned().flatten());
        }
        self.roots
            .lock()
            .expect("root selection mutex poisoned")
            .clone()
    }
}

/// One SQLite connection with the engine's tables and functions registered.
pub(super) struct QueryConnection {
    connection: Connection,
    registry: Arc<MatchRegistry>,
    /// Statements its cache keeps (`with_file`).
    capacity: usize,
}

impl QueryConnection {
    pub(super) fn open() -> rusqlite::Result<Self> {
        let connection = Connection::open_in_memory()?;
        let registry = Arc::new(MatchRegistry::default());
        register_module(&connection, Arc::clone(&registry))?;
        tables::register(&connection, &registry)?;
        facts::register(&connection, &registry)?;
        functions::register(&connection, &registry)?;
        Ok(Self {
            connection,
            registry,
            capacity: 0,
        })
    }

    #[cfg(test)]
    pub(super) fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Registers a batch; statements address it by the returned token. Its terminals poll the
    /// deadline of this thread's budget, on whatever thread they run.
    #[allow(dead_code, reason = "used by the restored SQL test harness")]
    pub(super) fn register(
        &self,
        terminals: Arc<Vec<Terminal>>,
        files: Vec<BatchFile>,
        catalog: Arc<Catalog>,
    ) -> QueryBatch<'_> {
        self.register_with_deadline(
            terminals,
            files,
            catalog,
            crate::code_rule_dsl::budget::current_deadline(),
        )
    }

    fn register_with_deadline(
        &self,
        terminals: Arc<Vec<Terminal>>,
        files: Vec<BatchFile>,
        catalog: Arc<Catalog>,
        deadline: Option<crate::code_rule_dsl::budget::MatchDeadline>,
    ) -> QueryBatch<'_> {
        #[cfg(test)]
        MAX_REGISTERED_FILES.fetch_max(files.len(), std::sync::atomic::Ordering::Relaxed);
        let state = Arc::new(BatchState {
            catalog,
            envs: EnvArena::default(),
            statement_executions: std::sync::atomic::AtomicUsize::new(0),
            rows: std::sync::atomic::AtomicUsize::new(0),
            text_rows: std::sync::Mutex::default(),
            util_memo: std::sync::Mutex::default(),
            roots: std::sync::Mutex::new(None),
            batch_roots: std::sync::Mutex::default(),
            derived: std::sync::Mutex::new(Vec::new()),
            facts: std::sync::Mutex::default(),
            structure: std::sync::Mutex::default(),
            siblings: parking_lot::Mutex::default(),
            same_keys: parking_lot::Mutex::default(),
        });
        let token =
            self.registry
                .register_query_batch(terminals, files, Arc::clone(&state) as _, deadline);
        QueryBatch {
            owner: self,
            token,
            state,
        }
    }
}

impl Drop for QueryConnection {
    fn drop(&mut self) {
        // SAFETY: the handle is this open connection's; its cached recursive statements are
        // finalized before it closes.
        recursion::forget(unsafe { self.connection.handle() });
    }
}

/// A registered batch; dropping it releases its files and cached rows.
pub(super) struct QueryBatch<'a> {
    owner: &'a QueryConnection,
    token: i64,
    state: Arc<BatchState>,
}

pub(super) struct QueryCounters {
    pub(super) statements: usize,
    pub(super) rows: usize,
    pub(super) terminal_executions: usize,
    pub(super) virtual_table_filters: usize,
}

impl QueryBatch<'_> {
    #[cfg(test)]
    pub(super) fn token(&self) -> i64 {
        self.token
    }

    pub(super) fn execution_counters(&self) -> QueryCounters {
        use std::sync::atomic::Ordering;
        let (terminal_executions, virtual_table_filters) = self
            .owner
            .registry
            .query_execution_counters(self.token)
            .unwrap_or_default();
        QueryCounters {
            statements: self.state.statement_executions.load(Ordering::Relaxed),
            rows: self.state.rows.load(Ordering::Relaxed),
            terminal_executions,
            virtual_table_filters,
        }
    }

    #[cfg(test)]
    pub(super) fn counters(&self) -> BatchCounters {
        self.owner
            .registry
            .counters(self.token)
            .expect("registered batch")
    }

    /// Runs one clause's statement until `visit` ends it. Matching roots are yielded in preorder
    /// while the matcher's clause context remains active.
    pub(super) fn run_while(
        &self,
        plan: &ClausePlan,
        file: usize,
        roots: Option<Arc<[u32]>>,
        matcher: &crate::code_rule_dsl::matcher::GrammarMatcher,
        work: &mut usize,
        mut visit: impl FnMut(u32, Arc<Env>, &mut usize) -> rusqlite::Result<bool>,
    ) -> rusqlite::Result<()> {
        let sql = if roots.is_some() {
            &plan.selected_sql
        } else {
            &plan.sql
        };
        *self
            .state
            .roots
            .lock()
            .expect("root selection mutex poisoned") = roots;
        crate::code_rule_dsl::matcher::begin_clause(matcher, *work);
        functions::reset_any_frames();
        let mut clause_active = true;
        let result = self.rows_while(sql, (self.token, file as i64), |node, env| {
            let (left, limit) = crate::code_rule_dsl::matcher::end_clause();
            clause_active = false;
            *work = left;
            if let Some(limit) = limit {
                return Err(rusqlite::Error::UserFunctionError(limit.message().into()));
            }
            let keep_going = visit(node, env, work)?;
            if keep_going {
                crate::code_rule_dsl::matcher::begin_clause(matcher, *work);
                clause_active = true;
            }
            Ok(keep_going)
        });
        let limit = if clause_active {
            let (left, limit) = crate::code_rule_dsl::matcher::end_clause();
            *work = left;
            limit
        } else {
            None
        };
        *self
            .state
            .roots
            .lock()
            .expect("root selection mutex poisoned") = None;
        match (result, limit) {
            (Ok(_), Some(limit)) => Err(rusqlite::Error::UserFunctionError(limit.message().into())),
            (result, _) => result,
        }
    }

    /// Runs one clause once across every file in this batch. The caller switches matcher, budget,
    /// scope, and adapter context in `enter` before SQLite evaluates a file's first root.
    pub(super) fn run_batch_while(
        &self,
        plan: &ClausePlan,
        roots: Vec<Option<Arc<[u32]>>>,
        mut enter: impl FnMut(i64) -> rusqlite::Result<bool>,
        mut visit: impl FnMut(usize, u32, Arc<Env>) -> rusqlite::Result<bool>,
    ) -> rusqlite::Result<()> {
        use std::sync::atomic::Ordering;
        *self
            .state
            .batch_roots
            .lock()
            .expect("batch root selection mutex poisoned") = roots;
        self.state
            .statement_executions
            .fetch_add(1, Ordering::Relaxed);
        let result = functions::with_file_hook(&mut enter, || {
            let mut statement = self.owner.connection.prepare_cached(&plan.batch_sql)?;
            let mut rows = statement.query((self.token,))?;
            while let Some(row) = rows.next()? {
                self.state.rows.fetch_add(1, Ordering::Relaxed);
                let Some(env) = row.get::<_, Option<i64>>(2)? else {
                    continue;
                };
                if env < 0 {
                    continue;
                }
                let file = usize::try_from(row.get::<_, i64>(0)?)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, -1))?;
                let node = u32::try_from(row.get::<_, i64>(1)?)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, -1))?;
                let env = self.state.envs.get(env).ok_or_else(|| {
                    rusqlite::Error::ModuleError("statement returned an unknown environment".into())
                })?;
                if !visit(file, node, env)? {
                    break;
                }
            }
            Ok(())
        });
        self.state
            .batch_roots
            .lock()
            .expect("batch root selection mutex poisoned")
            .clear();
        result
    }

    #[cfg(test)]
    /// Runs one clause's statement to completion and collects every matching root.
    pub(super) fn run(
        &self,
        plan: &ClausePlan,
        file: usize,
        roots: Option<Arc<[u32]>>,
        matcher: &crate::code_rule_dsl::matcher::GrammarMatcher,
        work: &mut usize,
    ) -> rusqlite::Result<Vec<(u32, Arc<Env>)>> {
        let mut matched = Vec::new();
        self.run_while(plan, file, roots, matcher, work, |node, env, _| {
            matched.push((node, env));
            Ok(true)
        })?;
        Ok(matched)
    }

    /// `violation.each` at one root (`GrammarMatcher::match_each`): `first` is the root's match with
    /// nothing excluded; the root is matched again with every binding of `name` recorded so far
    /// excluded, until a match binds nothing new or `room` matches were found. A match that leaves
    /// the capture unbound is kept only as the first.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn enumerate(
        &self,
        plan: &ClausePlan,
        file: usize,
        node: u32,
        first: Arc<Env>,
        matcher: &crate::code_rule_dsl::matcher::GrammarMatcher,
        work: &mut usize,
        name: &str,
        room: usize,
    ) -> rusqlite::Result<Vec<Arc<Env>>> {
        let mut found = Vec::new();
        if room == 0 {
            return Ok(found);
        }
        let view = self.view();
        let target = view.file(file as i64).expect("the batch's file");
        let scope = crate::code_rule_dsl::matcher::EachScope::start(name);
        let mut matched = first;
        loop {
            let fresh = scope.record(&matched.to_meta(target), name);
            if fresh || found.is_empty() {
                found.push(matched);
            }
            if !fresh || found.len() >= room {
                return Ok(found);
            }
            let mut next = None;
            self.within_clause(matcher, work, || {
                self.rows_while(
                    &plan.root_sql,
                    (self.token, file as i64, i64::from(node)),
                    |_, env| {
                        next = Some(env);
                        Ok(false)
                    },
                )
            })?;
            let Some(next) = next else {
                return Ok(found);
            };
            matched = next;
        }
    }

    /// Runs `rows` with `matcher`'s clause context (see `run_while`).
    fn within_clause<T>(
        &self,
        matcher: &crate::code_rule_dsl::matcher::GrammarMatcher,
        work: &mut usize,
        rows: impl FnOnce() -> rusqlite::Result<T>,
    ) -> rusqlite::Result<T> {
        crate::code_rule_dsl::matcher::begin_clause(matcher, *work);
        functions::reset_any_frames();
        let result = rows();
        let (left, limit) = crate::code_rule_dsl::matcher::end_clause();
        *work = left;
        match (result, limit) {
            (Ok(_), Some(limit)) => Err(rusqlite::Error::UserFunctionError(limit.message().into())),
            (result, _) => result,
        }
    }
    fn rows_while(
        &self,
        sql: &str,
        parameters: impl rusqlite::Params,
        mut visit: impl FnMut(u32, Arc<Env>) -> rusqlite::Result<bool>,
    ) -> rusqlite::Result<()> {
        use std::sync::atomic::Ordering;
        self.state
            .statement_executions
            .fetch_add(1, Ordering::Relaxed);
        let mut statement = self.owner.connection.prepare_cached(sql)?;
        let mut rows = statement.query(parameters)?;
        while let Some(row) = rows.next()? {
            self.state.rows.fetch_add(1, Ordering::Relaxed);
            #[cfg(test)]
            ROWS_STEPPED.with(|stepped| stepped.set(stepped.get() + 1));
            let Some(env) = row.get::<_, Option<i64>>(1)? else {
                continue;
            };
            if env < 0 {
                continue;
            }
            let node = u32::try_from(row.get::<_, i64>(0)?)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, -1))?;
            let env = self.state.envs.get(env).ok_or_else(|| {
                rusqlite::Error::ModuleError("statement returned an unknown environment".into())
            })?;
            if !visit(node, env)? {
                break;
            }
        }
        Ok(())
    }

    /// The ordered rows of a text terminal, followed by its completion row. A text statement
    /// never touches `rerail_nodes`, so a text-only batch remains untaped.
    pub(super) fn text_rows(&self, terminal: usize, file: usize) -> rusqlite::Result<TextRows> {
        use std::sync::atomic::Ordering;
        if let Some(rows) = self
            .state
            .text_rows
            .lock()
            .expect("text row cache mutex poisoned")
            .get(&terminal)
            .cloned()
        {
            return Ok(rows.get(file).cloned().unwrap_or_default());
        }
        self.state
            .statement_executions
            .fetch_add(1, Ordering::Relaxed);
        let mut statement = self.owner.connection.prepare_cached(
            "SELECT file_index, start_byte, end_byte, complete FROM rerail_matches \
             WHERE batch = ?1 AND program_index = ?2 \
             ORDER BY file_index, match_index IS NULL, match_index",
        )?;
        let mut query = statement.query((self.token, terminal as i64))?;
        let mut by_file = (0..self.view().files().len())
            .map(|_| Vec::new())
            .collect::<Vec<TextRows>>();
        while let Some(row) = query.next()? {
            self.state.rows.fetch_add(1, Ordering::Relaxed);
            let file_index = usize::try_from(row.get::<_, i64>(0)?)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, -1))?;
            let complete = row.get::<_, i64>(3)? != 0;
            let span = match (row.get::<_, Option<i64>>(1)?, row.get::<_, Option<i64>>(2)?) {
                (Some(start), Some(end)) => Some((start as usize, end as usize)),
                _ => None,
            };
            by_file
                .get_mut(file_index)
                .ok_or_else(|| {
                    rusqlite::Error::ModuleError(
                        "text terminal returned an unknown file".to_owned(),
                    )
                })?
                .push((span, complete));
        }
        let by_file = Arc::new(by_file);
        self.state
            .text_rows
            .lock()
            .expect("text row cache mutex poisoned")
            .insert(terminal, Arc::clone(&by_file));
        Ok(by_file.get(file).cloned().unwrap_or_default())
    }
}

impl Drop for QueryBatch<'_> {
    fn drop(&mut self) {
        functions::forget_batch();
        self.owner.registry.remove_batch(self.token);
    }
}

impl QueryBatch<'_> {
    /// The batch's files, for turning returned ordinals into nodes.
    pub(super) fn view(&self) -> crate::code_rule_dsl::transducer::sql_backend::BatchView {
        self.owner
            .registry
            .files(self.token)
            .expect("registered batch")
    }
}

/// A code_rule set's compiled statements: one per (code_rule, clause, grammar).
pub(super) struct SqlPlans {
    terminals: Arc<Vec<Terminal>>,
    catalog: Arc<Catalog>,
    plans: std::collections::HashMap<(usize, usize, GrammarDialect), ClausePlan>,
    regexes: std::collections::HashMap<(usize, usize), usize>,
    /// Distinct statements across the plans: what a connection's cache must hold for each
    /// file's run not to prepare them again.
    statements: usize,
}

impl SqlPlans {
    pub(super) fn compile(
        code_rules: &[Arc<crate::code_rule_dsl::model::CompiledCodeRule>],
    ) -> Self {
        use crate::code_rule_dsl::model::{ClauseKind, Query};
        let mut builder = CatalogBuilder::default();
        let mut plans = std::collections::HashMap::new();
        let mut regexes = std::collections::HashMap::new();
        for (code_rule_index, code_rule) in code_rules.iter().enumerate() {
            for (clause_index, clause) in code_rule.clauses.iter().enumerate() {
                if let ClauseKind::Forbid(Query::Regex { regex, source }) = &clause.kind {
                    let source = match source {
                        crate::code_rule_dsl::model::RegexSource::Content => TextSource::Content,
                        crate::code_rule_dsl::model::RegexSource::Path => TextSource::Path,
                    };
                    let terminal = builder.terminal(Terminal::Text(
                        regex.clone(),
                        source,
                        code_rule
                            .capabilities
                            .limits
                            .max_witnesses
                            .saturating_add(1),
                    ));
                    regexes.insert((code_rule_index, clause_index), terminal);
                    continue;
                }
                let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else {
                    continue;
                };
                let each = clause.violation.each.is_some();
                for matcher in matchers {
                    if let Ok(plan) = compile_clause(matcher, &mut builder, each) {
                        plans.insert((code_rule_index, clause_index, matcher.language), plan);
                    }
                }
            }
        }
        let (terminals, catalog) = builder.finish();
        let statements = plans
            .values()
            .flat_map(|plan: &ClausePlan| plan.statements())
            .collect::<std::collections::HashSet<_>>()
            .len();
        Self {
            terminals: Arc::new(terminals),
            catalog: Arc::new(catalog),
            plans,
            regexes,
            statements,
        }
    }

    pub(super) fn plan(
        &self,
        code_rule: usize,
        clause: usize,
        language: GrammarDialect,
    ) -> Option<&ClausePlan> {
        self.plans.get(&(code_rule, clause, language))
    }

    pub(super) fn regex_terminal(&self, code_rule: usize, clause: usize) -> Option<usize> {
        self.regexes.get(&(code_rule, clause)).copied()
    }

    /// Runs `visit` with a one-file batch on this thread's connection.
    pub(super) fn with_file<T>(
        &self,
        file: BatchFile,
        visit: impl FnOnce(&QueryBatch<'_>) -> T,
    ) -> rusqlite::Result<T> {
        self.with_files(vec![file], visit)
    }

    /// Runs `visit` with one immutable multi-file batch on this thread's connection.
    pub(super) fn with_files<T>(
        &self,
        files: Vec<BatchFile>,
        visit: impl FnOnce(&QueryBatch<'_>) -> T,
    ) -> rusqlite::Result<T> {
        with_files(
            &self.terminals,
            &self.catalog,
            self.statements,
            files,
            visit,
        )
    }
}

/// Statements a connection caches at least.
const MIN_CACHED_STATEMENTS: usize = 128;

/// One thread's query connection. Every slot is registered in `SLOTS`, so an idle workspace can
/// release the connections of all threads that evaluated for it — Rayon workers and the libuv
/// threads that run small applies inline alike — from the thread that evicts.
type Slot = Arc<std::sync::Mutex<Option<QueryConnection>>>;

type SlotRegistry = Vec<std::sync::Weak<std::sync::Mutex<Option<QueryConnection>>>>;
static SLOTS: std::sync::LazyLock<std::sync::Mutex<SlotRegistry>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

thread_local! {
    static CONNECTION: Slot = {
        let slot = Slot::default();
        SLOTS
            .lock()
            .expect("query connection registry poisoned")
            .push(Arc::downgrade(&slot));
        slot
    };
}

/// Closes every thread's idle query connection and its prepared statements; a connection running
/// a statement now is left to its thread. Threads that have exited are forgotten.
pub(super) fn release_connections() {
    let mut slots = SLOTS.lock().expect("query connection registry poisoned");
    slots.retain(|slot| {
        let Some(slot) = slot.upgrade() else {
            return false;
        };
        if let Ok(mut connection) = slot.try_lock() {
            connection.take();
        }
        true
    });
}

/// Runs `visit` with one immutable file batch on this thread's connection. The statement cache
/// holds at least `statements`, so a catalog pass does not repeatedly prepare the same plans.
/// Statements and terminals poll this thread's current deadline.
fn with_files<T>(
    terminals: &Arc<Vec<Terminal>>,
    catalog: &Arc<Catalog>,
    statements: usize,
    files: Vec<BatchFile>,
    visit: impl FnOnce(&QueryBatch<'_>) -> T,
) -> rusqlite::Result<T> {
    let slot = CONNECTION.with(Arc::clone);
    let mut slot = slot
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(QueryConnection::open()?);
    }
    let connection = slot.as_mut().expect("opened connection");
    // Sized to the code_rule set, not a fixed maximum: an idle workspace's eviction closes the
    // connection with its cache (`release_connections`).
    let capacity = statements.max(MIN_CACHED_STATEMENTS);
    if connection.capacity < capacity {
        connection
            .connection
            .set_prepared_statement_cache_capacity(capacity);
        connection.capacity = capacity;
    }
    let connection = &*connection;
    let deadline = crate::code_rule_dsl::budget::current_deadline();
    // A multi-file statement changes the installed file budget as it advances. A SQLite progress
    // interruption would abort the other files. Terminals, walks, facts, and matcher callbacks
    // poll the active file deadline directly.
    let batch = connection.register_with_deadline(
        Arc::clone(terminals),
        files,
        Arc::clone(catalog),
        deadline,
    );
    let result = visit(&batch);
    drop(batch);
    connection
        .connection
        .progress_handler(0, None::<fn() -> bool>)?;
    Ok(result)
}

/// A regex clause's matches in `haystack`, in order, at most `limit`; and whether the scan finished.
pub(in crate::code_rule_dsl) fn regex_matches(
    regex: &regex::Regex,
    haystack: &str,
    limit: usize,
) -> (Vec<std::ops::Range<usize>>, bool) {
    let mut matches = Vec::new();
    for found in regex.find_iter(haystack) {
        if crate::code_rule_dsl::budget::matcher_budget_exhausted() || matches.len() >= limit {
            return (matches, false);
        }
        matches.push(found.range());
    }
    (matches, true)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod sql_tests;

#[cfg(test)]
mod parity_tests;
