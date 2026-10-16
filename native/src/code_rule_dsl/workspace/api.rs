//! The N-API surface: the `CodeRuleDslWorkspace` class, the lease that guards its state, and the
//! storage, apply, completion and result page async tasks.

use super::crossfile::{ConsumerRefresh, CrossFileScheduler};
use super::documents;
use super::evaluate::{Job, JobContext, evaluate_file, run_jobs};
use super::global::{compile_query_plan, compile_relation_producer, evaluate_global};
use super::index::{RepositoryIndexOptions, WorkspaceIndexTask, reusable_directories};
use super::ingest::{
    Request, apply_request, decode_buffer_request, file_needs_update, inline_input, is_cheap,
    is_file_local, make_file, reclaim_candidate_sets,
};
#[cfg(feature = "code-rule-profile")]
use super::memory::record_workspace_residency;
use super::memory::workspace_memory_profile;
use super::persisted::{Persisted, candidate_catalog, persist_files};
use super::results::{
    ResultText, evicted_file_states, profile_result, replace_global_changes, serialize_result,
    serialize_result_page,
};
use super::storage::{Attached, StorageContext, attach_storage, unix_seconds};
use super::{
    CLAUSE_WORK_BUDGET, CandidateSets, STORED_RESULT_UNAVAILABLE, WorkspaceState, check_closed,
};
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::budget::MatchBudget;
use crate::code_rule_dsl::eval::evidence::Side;
use crate::code_rule_dsl::eval::{Engine, evaluation_json, overflow_evaluation};
use crate::code_rule_dsl::json::{invalid, json_error};
use crate::code_rule_dsl::model::{ClauseKind, CodeRuleScope, Evaluation, Metrics, Snapshot};
use crate::code_rule_dsl::placement::parse_placement;
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::{matcher::GrammarMatcher, model::Query, profile};
use crate::content_store::{OperationDeadline, StoreError, WorkspaceCompletion};
use crate::release_allocator_memory;
use napi::bindgen_prelude::{AsyncTask, Buffer};
use napi::{Env, Result, Task};
use napi_derive::napi;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

const INLINE_TIME_BUDGET: Duration = Duration::from_millis(5);

/// Distinct workspace instances in this process.
static WORKSPACE_INSTANCES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[napi]
pub struct CodeRuleDslWorkspace {
    pub(super) state: Arc<Mutex<WorkspaceState>>,
    pub(super) scopes: Mutex<Vec<CodeRuleScope>>,
    pub(super) closed: Arc<AtomicBool>,
}

impl Default for CodeRuleDslWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

/// A workspace's options (`new CodeRuleDslWorkspace(options?)`).
#[napi(object)]
pub struct WorkspaceOptions {
    /// Which engine matches the workspace's AST clauses: `native` (the default) or `sql`.
    pub engine: Option<String>,
    /// Files per batch on the SQL engine (default 64).
    pub sql_batch_size: Option<u32>,
}

#[napi]
impl CodeRuleDslWorkspace {
    /// A workspace on the native engine; in tests, on the engine `RERAIL_TEST_WORKSPACE_ENGINE`
    /// names (and `RERAIL_TEST_WORKSPACE_SQL_BATCH` files per SQL batch), so the workspace tests
    /// run on either engine.
    pub fn new() -> Self {
        #[cfg(test)]
        let options = super::sql::EngineOptions::parse(
            std::env::var("RERAIL_TEST_WORKSPACE_ENGINE")
                .ok()
                .as_deref(),
            std::env::var("RERAIL_TEST_WORKSPACE_SQL_BATCH")
                .ok()
                .and_then(|size| size.parse().ok()),
        )
        .expect("RERAIL_TEST_WORKSPACE_ENGINE is native or sql");
        #[cfg(not(test))]
        let options = super::sql::EngineOptions::default();
        Self::with_engine(options)
    }

    /// A workspace whose AST clauses `options` says which engine matches.
    #[napi(constructor)]
    pub fn with_options(options: Option<WorkspaceOptions>) -> Result<Self> {
        let options = options.unwrap_or(WorkspaceOptions {
            engine: None,
            sql_batch_size: None,
        });
        Ok(Self::with_engine(super::sql::EngineOptions::parse(
            options.engine.as_deref(),
            options.sql_batch_size,
        )?))
    }

    fn with_engine(engine: super::sql::EngineOptions) -> Self {
        let state = WorkspaceState {
            instance: WORKSPACE_INSTANCES.fetch_add(1, Ordering::Relaxed),
            engine,
            ..WorkspaceState::default()
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            scopes: Mutex::new(Vec::new()),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The engine that matches the workspace's AST clauses, as JSON: on SQL, its clauses, those it
    /// refuses and why, how many ran, and what its statements did.
    #[napi]
    pub fn engine_report(&self) -> Result<String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| invalid("policy workspace is unavailable"))?;
        let report = match super::sql::engine_of(&mut state) {
            Some(sql) => sql.report(),
            None => json!({ "engine": "native" }),
        };
        serde_json::to_string(&report).map_err(json_error)
    }

    /// Attaches the content store file at `store_path` to the Git repository containing `root`.
    /// The host chooses the file (each build owns its own); the binding never derives one.
    #[napi]
    pub fn enable_storage_async(
        &self,
        root: String,
        store_path: String,
    ) -> Result<AsyncTask<WorkspaceStorageTask>> {
        Ok(AsyncTask::new(self.storage_task(root, store_path)?))
    }

    pub(super) fn storage_task(
        &self,
        root: String,
        store_path: String,
    ) -> Result<WorkspaceStorageTask> {
        if store_path.is_empty() {
            return Err(invalid("content store path is required"));
        }
        Ok(WorkspaceStorageTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
            root,
            store_path: PathBuf::from(store_path),
        })
    }

    /// Registers `compiled` with this workspace. `placement`, when given, is the JSON array of
    /// `{"folder", "mode", "suppressions"?}` entries that places this workspace's copy of the code
    /// rule per folder of the checkout (`mode: null` is off there; `suppressions: "ignore"` reports
    /// violations a linter's suppression comment would silence, `"honour"` or no `suppressions`
    /// lets the comment silence them); the compiled code rule itself is never changed.
    #[napi]
    pub fn add_code_rule(
        &mut self,
        compiled: &CompiledCodeRuleDsl,
        placement: Option<String>,
    ) -> Result<()> {
        let placement = placement.as_deref().map(parse_placement).transpose()?;
        let guard = match self.state.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(error)) => {
                discard_poisoned_guard(error.into_inner(), &self.state, &self.closed);
                return Err(invalid("policy workspace is evaluating"));
            }
            Err(TryLockError::WouldBlock) => return Err(invalid("policy workspace is evaluating")),
        };
        let mut state = StateLease {
            guard: Some(guard),
            mutex: &self.state,
            closed: &self.closed,
        };
        if self.closed.load(Ordering::Acquire) || state.revision != 0 {
            return Err(invalid(
                "policies must be registered before workspace evaluation",
            ));
        }
        if state
            .code_rules
            .iter()
            .any(|code_rule| code_rule.id == compiled.inner.id)
        {
            return Ok(());
        }
        let index = state.code_rules.len();
        let globals = compiled
            .inner
            .clauses
            .iter()
            .filter(|clause| !is_file_local(clause))
            .cloned()
            .collect::<Vec<_>>();
        let mut resident = Arc::clone(&compiled.inner);
        if placement.is_some() {
            Arc::make_mut(&mut resident).scope.placement = placement;
        }
        if !globals.is_empty() {
            let code_rule = Arc::make_mut(&mut resident);
            for clause in &globals {
                match &clause.kind {
                    ClauseKind::Forbid(query) => {
                        let plan = compile_query_plan(query, clause, index, code_rule);
                        state.query_plans.insert((index, clause.id.clone()), plan);
                    }
                    ClauseKind::RequireEach { subject, evidence } => {
                        let subject = compile_relation_producer(
                            subject,
                            Side::Subject,
                            clause,
                            index,
                            code_rule,
                        );
                        let evidence = compile_relation_producer(
                            evidence,
                            Side::Evidence,
                            clause,
                            index,
                            code_rule,
                        );
                        state
                            .relation_plans
                            .insert((index, clause.id.clone()), vec![subject, evidence]);
                    }
                    ClauseKind::Budget { query, .. } => {
                        let id = compile_relation_producer(
                            query,
                            Side::Counted,
                            clause,
                            index,
                            code_rule,
                        );
                        state
                            .relation_plans
                            .insert((index, clause.id.clone()), vec![id]);
                    }
                    ClauseKind::Llm => {}
                }
            }
            state.globals.insert(index, globals);
            state.dirty_globals.insert(index);
        }
        self.scopes
            .lock()
            .map_err(|_| invalid("policy workspace is unavailable"))?
            .push(resident.scope.clone());
        #[cfg(feature = "code-rule-profile")]
        for clause in &resident.clauses {
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for GrammarMatcher {
                    language,
                    matcher,
                    anchors,
                    ..
                } in matchers
                {
                    profile::record_query(
                        &resident.id,
                        &clause.id,
                        *language,
                        matcher,
                        anchors.root_kinds().cloned(),
                    );
                }
            }
        }
        CrossFileScheduler::register(&mut state, index, Arc::make_mut(&mut resident))?;
        state.code_rules.push(resident);
        if let Err(error) = check_closed(&self.closed) {
            *self
                .scopes
                .lock()
                .map_err(|_| invalid("policy workspace is unavailable"))? = Vec::new();
            return Err(error);
        }
        Ok(())
    }

    #[napi]
    pub fn accepts_path(&self, path: String) -> bool {
        !self.closed.load(Ordering::Acquire)
            && self
                .scopes
                .lock()
                .is_ok_and(|scopes| scopes.iter().any(|scope| scope.matches(&path)))
    }

    #[napi]
    pub fn try_apply(&self, request_json: String) -> Result<Option<String>> {
        if self.closed.load(Ordering::Acquire) || request_json.len() > 8192 {
            return Ok(None);
        }
        let Ok(_slot) = cpu_slot().try_lock() else {
            return Ok(None);
        };
        let guard = match self.state.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(error)) => {
                discard_poisoned_guard(error.into_inner(), &self.state, &self.closed);
                return Ok(None);
            }
            Err(TryLockError::WouldBlock) => return Ok(None),
        };
        let mut state = StateLease {
            guard: Some(guard),
            mutex: &self.state,
            closed: &self.closed,
        };
        check_closed(&self.closed)?;
        let request: Request = serde_json::from_str(&request_json).map_err(json_error)?;
        // The SQL engine matches in batches: its workspace leaves every apply to `applyAsync`.
        if !is_cheap(&state, &request) || state.engine.engine == Engine::Sql {
            return Ok(None);
        }
        // The fast path has no parser, global queries, unbounded corpus or lock wait.
        // Stage its tiny file unit before committing, so a deadline miss is a true no-op.
        let Some(input) = request.files.into_iter().next() else {
            return Ok(None);
        };
        let file = inline_input(&mut state, input)?;
        check_revision(&state, request.revision)?;
        if !file_needs_update(&state, &file) {
            check_closed(&self.closed)?;
            state.result_page_index = None;
            state.revision = request.revision;
            let replaced = BTreeSet::from([file.path.clone()]);
            return serialize_result(&state, Metrics::default(), false, &replaced, false)
                .map(|result| Some(result.text));
        }
        let WorkspaceState {
            code_rules,
            candidate_sets,
            ..
        } = &mut *state;
        let staged = make_file(file, request.revision, code_rules, candidate_sets);
        let candidates_changed = state
            .files
            .get(&staged.input.path)
            .is_none_or(|current| !Arc::ptr_eq(&current.candidates, &staged.candidates));
        let result = (|| {
            // Own staging inside this scope so every early return drops it before reclamation.
            let mut staged = staged;
            let budget = MatchBudget::until(&self.closed, INLINE_TIME_BUDGET);
            let WorkspaceState {
                code_rules,
                grammars,
                ..
            } = &mut *state;
            let metrics = evaluate_file(
                &mut staged,
                code_rules,
                grammars,
                &self.closed,
                false,
                &budget,
                &mut None,
            )?;
            if budget.observed() {
                return Ok(None);
            }
            check_closed(&self.closed)?;
            check_revision(&state, request.revision)?;
            state.result_page_index = None;
            state.revision = request.revision;
            let path = staged.input.path.clone();
            state.insert_file(path.clone(), staged);
            let profile = metrics.clone();
            let serialization_started = Instant::now();
            let replaced = BTreeSet::from([path]);
            let result = serialize_result(&state, metrics, false, &replaced, false)?;
            profile_result(result, &profile, serialization_started.elapsed())
                .map(|result| Some(result.text))
        })();
        if candidates_changed {
            reclaim_candidate_sets(&mut state.candidate_sets);
        }
        result
    }

    #[napi]
    pub fn apply_async(&self, request_json: String) -> AsyncTask<WorkspaceApplyTask> {
        AsyncTask::new(WorkspaceApplyTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
            request_json,
            request_buffer: None,
            request_direct: None,
        })
    }

    #[napi]
    pub fn apply_buffer_async(&self, request: Buffer) -> AsyncTask<WorkspaceApplyTask> {
        AsyncTask::new(WorkspaceApplyTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
            request_json: String::new(),
            request_buffer: Some(request.to_vec()),
            request_direct: None,
        })
    }

    /// One bounded chunk of a repository index over the attached Git worktree (see `index`);
    /// the host calls it until the chunk reports `done`.
    #[napi]
    pub fn index_repository_async(
        &self,
        options: RepositoryIndexOptions,
    ) -> AsyncTask<WorkspaceIndexTask> {
        AsyncTask::new(WorkspaceIndexTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
            options,
        })
    }

    /// Record the current revision as reusable, on a libuv pool thread (`WorkspaceCompletionTask`).
    #[napi]
    pub fn complete_storage_revision_async(&self) -> AsyncTask<WorkspaceCompletionTask> {
        AsyncTask::new(self.completion_task())
    }

    pub(super) fn completion_task(&self) -> WorkspaceCompletionTask {
        WorkspaceCompletionTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
        }
    }

    /// One page of the current revision's ordered results, computed on a libuv pool thread: the
    /// page at cursor 0 builds the ordered result index, which the last page releases.
    #[napi]
    pub fn result_page_async(
        &self,
        cursor: u32,
        maximum_bytes: u32,
    ) -> AsyncTask<WorkspaceResultPageTask> {
        AsyncTask::new(self.result_page_task(cursor, maximum_bytes))
    }

    pub(super) fn result_page_task(
        &self,
        cursor: u32,
        maximum_bytes: u32,
    ) -> WorkspaceResultPageTask {
        WorkspaceResultPageTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
            cursor: cursor as usize,
            maximum_bytes: maximum_bytes as usize,
        }
    }

    #[napi]
    pub fn memory_profile(&self) -> Result<String> {
        let state = self
            .state
            .lock()
            .map_err(|_| invalid("policy workspace is unavailable"))?;
        serde_json::to_string(&workspace_memory_profile(&state)).map_err(json_error)
    }

    /// Bound the bytes of documents the storage contract keeps resident; loads beyond it evict the
    /// least recently used documents, which later loads bring back from the store or a reparse.
    #[napi]
    pub fn set_document_residency_limit(&self, bytes: u32) -> Result<()> {
        self.state
            .try_lock()
            .map_err(|_| invalid("policy workspace is evaluating"))?
            .documents
            .set_limit(bytes as usize);
        Ok(())
    }

    #[napi]
    pub fn evict_inactive(&self) -> Result<bool> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| invalid("policy workspace is evaluating"))?;
        // Cross-file clauses read the evidence index and the call graph, which eviction keeps: they
        // are summaries of the files, and an edit after eviction updates the graph in place.
        // A file without stored results stays only when a busy store left it unpublished.
        if state.storage.is_none()
            || state
                .files
                .values()
                .any(|file| file.stored.is_none() && !file.unpublished)
        {
            return Ok(false);
        }
        // A file the store lacks results of (`FileState::stays_resident`: a transient evaluation,
        // an unstored clause, or results a busy store left unpublished) has none to page; it stays.
        let evicted = state
            .files
            .values()
            .filter(|file| !file.stays_resident())
            .map(|file| {
                let mut input = file.input.clone();
                input.source = Arc::new(String::new());
                (documents::FileVersion::of(file), input)
            })
            .collect::<Vec<_>>();
        for (version, input) in evicted {
            state.versions.insert(input.path.clone(), version);
            state.evicted_inputs.insert(input.path.clone(), input);
        }
        state.files.retain(|_, file| file.stays_resident());
        state.documents.clear();
        state.candidate_sets = CandidateSets::new();
        state.grammars.clear();
        if let Some(storage) = &state.storage {
            storage
                .store
                .release_memory()
                .map_err(|error| invalid(error.to_string()))?;
        }
        release_allocator_memory();
        Ok(true)
    }

    /// A snapshot-qualified reference to the deepest node spanning exactly `startByte..endByte`
    /// in `path`'s current version, or null when no node spans exactly that range.
    #[napi]
    pub fn node_reference(
        &self,
        path: String,
        start_byte: u32,
        end_byte: u32,
    ) -> Result<Option<String>> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| invalid("policy workspace is evaluating"))?;
        documents::reference(&mut state, &path, start_byte as usize, end_byte as usize)?
            .map(|reference| serde_json::to_string(&reference).map_err(json_error))
            .transpose()
    }

    /// The node a reference names, as JSON, loading its file through the storage contract. A
    /// reference whose file version is no longer current is refused.
    #[napi]
    pub fn resolve_node_reference(&self, reference: String) -> Result<String> {
        let reference: documents::NodeReference =
            serde_json::from_str(&reference).map_err(json_error)?;
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| invalid("policy workspace is evaluating"))?;
        serde_json::to_string(&documents::resolve(&mut state, &reference)?).map_err(json_error)
    }

    #[napi]
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        clear_if_unlocked(&self.state);
        match self.scopes.lock() {
            Ok(mut scopes) => *scopes = Vec::new(),
            Err(error) => *error.into_inner() = Vec::new(),
        }
    }
}

// Closing never waits on the JS thread. When close sees a busy lock, the
// current lease clears it after unlocking; a new lease taking the lock first
// inherits that responsibility. The post-unlock check closes the handoff gap.
fn clear_if_unlocked(mutex: &Mutex<WorkspaceState>) {
    let clear = |state: &mut WorkspaceState| {
        // Removals a busy store kept would leave rows no later session collects: one attempt that
        // never waits, since close runs on the host's thread.
        if let Some(storage) = &state.storage
            && !state.pending_removals.is_empty()
        {
            let pending = state.pending_removals.iter().cloned().collect::<Vec<_>>();
            if let Err(error) = storage
                .store
                .remove_overlay_files_no_wait(storage.workspace_id, &pending)
                && !matches!(error, StoreError::Busy)
            {
                eprintln!(
                    "rerail: removing {} overlay files of workspace {:?} at close failed: {error}",
                    pending.len(),
                    storage.workspace_id
                );
            }
        }
        let storage = state.storage.take();
        *state = WorkspaceState::default();
        if let Some(storage) = storage {
            let _ = storage.store.release_memory();
        }
        release_allocator_memory();
    };
    match mutex.try_lock() {
        Ok(mut guard) => clear(&mut guard),
        Err(TryLockError::Poisoned(mut error)) => clear(error.get_mut()),
        Err(TryLockError::WouldBlock) => {}
    }
}

// Poisoned state cannot be evaluated, but its guard still owns the close
// handoff: release it, then retry cleanup if close lost the lock race.
fn discard_poisoned_guard(
    guard: MutexGuard<'_, WorkspaceState>,
    mutex: &Mutex<WorkspaceState>,
    closed: &AtomicBool,
) {
    drop(StateLease {
        guard: Some(guard),
        mutex,
        closed,
    });
}

pub(super) struct StateLease<'a> {
    pub(super) guard: Option<MutexGuard<'a, WorkspaceState>>,
    pub(super) mutex: &'a Mutex<WorkspaceState>,
    pub(super) closed: &'a AtomicBool,
}
impl std::ops::Deref for StateLease<'_> {
    type Target = WorkspaceState;
    fn deref(&self) -> &WorkspaceState {
        self.guard.as_ref().expect("active state lease")
    }
}
impl std::ops::DerefMut for StateLease<'_> {
    fn deref_mut(&mut self) -> &mut WorkspaceState {
        self.guard.as_mut().expect("active state lease")
    }
}
impl Drop for StateLease<'_> {
    fn drop(&mut self) {
        drop(self.guard.take());
        if self.closed.load(Ordering::Acquire) {
            clear_if_unlocked(self.mutex);
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Runs once, inside the next storage attach on this thread, while the attach still holds the
    /// workspace lock: a test lands `close` there.
    pub(super) static ON_STORAGE_ATTACHED: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// The storage retries and the retry wait (nanoseconds) the last apply on this thread
    /// reported: they reach the host only in profile output.
    pub(super) static LAST_APPLY_STORAGE_RETRIES: std::cell::Cell<(usize, u64)> =
        const { std::cell::Cell::new((0, 0)) };
    /// The files the last apply on this thread left unpublished because the store was busy.
    pub(super) static LAST_APPLY_UNPUBLISHED_FILES: std::cell::Cell<usize> =
        const { std::cell::Cell::new(0) };
}

/// What a storage attach did. No store attaches where Git cannot describe the workspace, nor
/// where the store stays busy past its write deadline; `reason` then says why, unless the root is
/// simply outside any Git repository. The milliseconds are the wall time the attach spent
/// discovering the Git checkout and opening the store to publish its snapshot (0 for a phase that
/// did not run), so a slow attach shows which phase it waited in.
#[napi(object)]
#[derive(Debug)]
pub struct StorageAttachment {
    pub attached: bool,
    pub reason: Option<String>,
    pub discovery_milliseconds: f64,
    pub publication_milliseconds: f64,
}

/// Attaches the content store to a workspace whose code rules may already be registered, any time
/// before its first apply: scopes answer `accepts_path` at once while discovery and snapshot
/// publication run on a pool thread.
pub struct WorkspaceStorageTask {
    state: Arc<Mutex<WorkspaceState>>,
    closed: Arc<AtomicBool>,
    root: String,
    /// The store file the host named.
    store_path: PathBuf,
}

impl Task for WorkspaceStorageTask {
    type Output = StorageAttachment;
    type JsValue = StorageAttachment;

    fn compute(&mut self) -> Result<StorageAttachment> {
        check_closed(&self.closed)?;
        // A workspace Git cannot describe (no repository, an unreadable untracked directory, a
        // corrupt index) attaches no store: the host walks it, reporting what it cannot read,
        // and interactive edits evaluate without reuse. Only a root outside any repository is
        // expected; any other cause is reported, so reuse is never lost silently.
        let discovery_started = Instant::now();
        let discovered = crate::git_workspace::discover(std::path::Path::new(&self.root));
        let discovery_milliseconds = discovery_started.elapsed().as_secs_f64() * 1000.0;
        let discovered = match discovered {
            Ok(discovered) => discovered,
            Err(error) => {
                return Ok(StorageAttachment {
                    attached: false,
                    reason: (!error.is_no_repository()).then(|| error.to_string()),
                    discovery_milliseconds,
                    publication_milliseconds: 0.0,
                });
            }
        };
        // A busy store is a cache miss: the workspace runs without reuse, and says why.
        let publication_started = Instant::now();
        let attached =
            attach_storage(&self.root, discovered, std::mem::take(&mut self.store_path))?;
        let publication_milliseconds = publication_started.elapsed().as_secs_f64() * 1000.0;
        let context = match attached {
            Attached::Storage(context) => context,
            Attached::Busy(reason) => {
                return Ok(StorageAttachment {
                    attached: false,
                    reason: Some(reason),
                    discovery_milliseconds,
                    publication_milliseconds,
                });
            }
        };
        let guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(error) => {
                discard_poisoned_guard(error.into_inner(), &self.state, &self.closed);
                return Err(invalid("policy workspace is unavailable"));
            }
        };
        // A close that lands while this holds the lock cannot clear the state; the lease does on drop.
        let mut state = StateLease {
            guard: Some(guard),
            mutex: &self.state,
            closed: &self.closed,
        };
        check_closed(&self.closed)?;
        if state.revision != 0 || state.storage.is_some() {
            return Err(invalid(
                "storage must be attached before the workspace's first apply",
            ));
        }
        state.storage = Some(context);
        #[cfg(test)]
        if let Some(on_attached) = ON_STORAGE_ATTACHED.take() {
            on_attached();
        }
        Ok(StorageAttachment {
            attached: true,
            reason: None,
            discovery_milliseconds,
            publication_milliseconds,
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

pub struct WorkspaceApplyTask {
    pub(super) state: Arc<Mutex<WorkspaceState>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) request_json: String,
    pub(super) request_buffer: Option<Vec<u8>>,
    pub(super) request_direct: Option<Request>,
}
pub(super) fn cpu_slot() -> &'static Mutex<()> {
    static SLOT: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    &SLOT
}
impl Task for WorkspaceApplyTask {
    type Output = ResultText;
    type JsValue = ResultText;
    fn compute(&mut self) -> Result<ResultText> {
        let _slot = cpu_slot()
            .try_lock()
            .map_err(|_| invalid("WORKSPACE_BUSY"))?;
        self.apply_with_slot()
    }
    fn resolve(&mut self, _env: Env, output: ResultText) -> Result<ResultText> {
        check_closed(&self.closed)?;
        Ok(output)
    }
    /// The response is in JavaScript and freed by now: purges its pages, which outlive the apply's
    /// own purge.
    fn finally(self, _env: Env) -> Result<()> {
        drop(self);
        crate::purge_allocator_pages();
        Ok(())
    }
}

impl WorkspaceApplyTask {
    /// The apply itself, for a caller that already holds `cpu_slot`.
    pub(super) fn apply_with_slot(&mut self) -> Result<ResultText> {
        check_closed(&self.closed)?;
        // Declared first, so it purges after everything else the apply holds is freed. The request
        // payload leaves the task and is freed once decoded; the response is purged in `finally`.
        let _purge = PurgeAfterApply;
        #[cfg(feature = "code-rule-profile")]
        let cpu_started = profile::thread_cpu_ns();
        let request = match self.request_direct.take() {
            Some(request) => request,
            None => {
                let json = std::mem::take(&mut self.request_json);
                match self.request_buffer.take() {
                    Some(buffer) if json.is_empty() => decode_buffer_request(&buffer)?,
                    None => serde_json::from_str(&json).map_err(json_error)?,
                    Some(_) => return Err(invalid("workspace request encoding is invalid")),
                }
            }
        };
        // The encoded request (a copy of every source it sends) is not read again: it is
        // released before the evaluation, not with the task.
        drop(std::mem::take(&mut self.request_json));
        drop(self.request_buffer.take());
        #[cfg(feature = "code-rule-profile")]
        let _profile = profile::ApplyProfile::start(request.revision, cpu_started);
        let guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(error) => {
                discard_poisoned_guard(error.into_inner(), &self.state, &self.closed);
                return Err(invalid("policy workspace is unavailable"));
            }
        };
        let mut state = StateLease {
            guard: Some(guard),
            mutex: &self.state,
            closed: &self.closed,
        };
        check_closed(&self.closed)?;
        // One write deadline for everything this apply writes to the store: removals, each
        // publication pass, promotion, lease renewal.
        let _busy_deadline = OperationDeadline::enter();
        let deferred = request.defer_global;
        let quiet = request.quiet;
        let mut replaced = request
            .files
            .iter()
            .map(|file| file.path.clone())
            .chain(request.removed.iter().cloned())
            .collect::<BTreeSet<_>>();
        let (refreshed, removal_busy) = apply_request(&mut state, request)?;
        replaced.extend(refreshed);
        let mut metrics = Metrics::default();
        let dirty = std::mem::take(&mut state.dirty_files);
        replaced.extend(dirty.iter().cloned());
        let mut persistence_paths = dirty.iter().cloned().collect::<Vec<_>>();
        let mut jobs = Vec::with_capacity(dirty.len());
        // Each dirty file is taken out for evaluation and put back (`insert_file`): its path
        // stays in the workspace, with the package it marks.
        for path in dirty {
            if let Some(file) = state.files.remove(&path) {
                jobs.push(Job {
                    path,
                    file,
                    consumes: false,
                    crossfile: None,
                });
            }
        }
        let code_rules = state.code_rules.clone();
        let storage = state.storage.clone();
        let sql = super::sql::engine_of(&mut state);
        let producer_batch =
            CrossFileScheduler::prepare(&mut state, &mut jobs, &code_rules, deferred);
        let context = JobContext {
            code_rules: &code_rules,
            storage: storage.as_ref(),
            closed: &self.closed,
            summarize: CrossFileScheduler::summarizes(&producer_batch),
            sql: sql.as_deref(),
        };
        let mut evaluation_error = None;
        let phase_a = match run_jobs(&mut jobs, &context) {
            Ok(phase_a) => phase_a,
            Err(error) => {
                for Job { path, file, .. } in jobs {
                    state.dirty_files.insert(path.clone());
                    state.insert_file(path, file);
                }
                return Err(error);
            }
        };
        for (path, file, work) in phase_a {
            match work {
                Ok(work) => metrics.merge(work),
                Err(error) if evaluation_error.is_none() => evaluation_error = Some(error),
                Err(_) => {}
            }
            CrossFileScheduler::record(&mut state, &producer_batch, &path, &file);
            state.insert_file(path, file);
        }
        if let Some(error) = evaluation_error {
            return Err(error);
        }
        let crossfile_evicted = CrossFileScheduler::refresh_consumers(
            &mut state,
            &producer_batch,
            ConsumerRefresh {
                deferred,
                replaced: &mut replaced,
                persistence_paths: &mut persistence_paths,
                metrics: &mut metrics,
                evaluation_error: &mut evaluation_error,
                context,
            },
        )?;
        if let Some(error) = evaluation_error {
            return Err(error);
        }
        CrossFileScheduler::release_producer_scopes(&mut state, &producer_batch);
        #[cfg(feature = "code-rule-profile")]
        let _storage_write_timer = profile::PhaseTimer::start(profile::Phase::StoreWrite);
        let storage_write_started = Instant::now();
        // A file a busy store left unpublished stays resident with its results, flagged until a
        // later apply publishes them: only their reuse is lost. An apply that finds the store
        // free deletes the removals earlier applies left in `pending_removals` first, so no
        // deletion lands on rows it publishes, even when it publishes nothing; one that
        // retries the files earlier applies left unpublished, so they can be evicted again,
        // even when it publishes nothing of its own. Once a removal or one chunk waited out the
        // deadline, later chunks do not wait, nor do later applies until the workspace's busy
        // window passes (`note_store_busy`).
        if !removal_busy || !persistence_paths.is_empty() {
            let listed = persistence_paths.iter().cloned().collect::<BTreeSet<_>>();
            persistence_paths.extend(
                state
                    .files
                    .iter()
                    .filter(|(path, file)| {
                        file.unpublished && !file.results_only && !listed.contains(*path)
                    })
                    .map(|(path, _)| path.clone()),
            );
        }
        if let Some(storage) = state.storage.clone() {
            let mut busy = removal_busy;
            if !busy && !state.pending_removals.is_empty() {
                check_closed(&self.closed)?;
                let pending = std::mem::take(&mut state.pending_removals)
                    .into_iter()
                    .collect::<Vec<_>>();
                if let Err(error) = storage
                    .store
                    .remove_overlay_files(storage.workspace_id, &pending)
                {
                    state.pending_removals.extend(pending);
                    if !matches!(error, StoreError::Busy) {
                        return Err(invalid(error.to_string()));
                    }
                    busy = true;
                    state.note_store_busy("while deleting removed files");
                }
            }
            for paths in persistence_paths.chunks(64) {
                check_closed(&self.closed)?;
                let persisted = if busy {
                    None
                } else {
                    Some(persist_files(
                        &storage,
                        paths,
                        &state.files,
                        &state.code_rules,
                        CrossFileScheduler::consumers(&state),
                        &self.closed,
                    )?)
                };
                let mut published = None;
                match persisted {
                    None => {}
                    // A publication that waited out the deadline still spent that time: its
                    // retries count though it stored nothing.
                    Some(Persisted::Busy { retries }) => {
                        metrics.storage_retries += retries.retries;
                        metrics.storage_retry_wait_nanoseconds += retries.wait_nanoseconds;
                        state.note_store_busy("while publishing results");
                    }
                    Some(Persisted::Published {
                        result,
                        busy_during,
                    }) => {
                        match busy_during {
                            Some(during) => {
                                state.note_store_busy(during);
                                busy = true;
                            }
                            None => state.note_store_published(),
                        }
                        published = Some(result);
                    }
                }
                let Some(publication) = published else {
                    busy = true;
                    metrics.storage_unpublished_files += paths.len();
                    for path in paths {
                        if let Some(file) = state.files.get_mut(path) {
                            file.unpublished = true;
                        }
                    }
                    continue;
                };
                metrics.storage_retries += publication.retries;
                metrics.storage_retry_wait_nanoseconds += publication.retry_wait_nanoseconds;
                metrics.storage_transaction_nanoseconds += publication.transaction_nanoseconds;
                metrics.storage_object_transaction_nanoseconds +=
                    publication.object_transaction_nanoseconds;
                metrics.storage_relationship_transaction_nanoseconds +=
                    publication.relationship_transaction_nanoseconds;
                metrics.storage_read_data_statements += publication.read_data_statements;
                metrics.storage_data_statements += publication.data_statements;
                metrics.storage_max_data_statements_per_pass = metrics
                    .storage_max_data_statements_per_pass
                    .max(publication.max_data_statements_per_pass);
                metrics.storage_transaction_statements += publication.transaction_statements;
                metrics.storage_reconciliation_passes += publication.reconciliation_passes;
                metrics.reused_sources += publication.reused_sources;
                metrics.reused_tapes += publication.reused_tapes;
                metrics.compressed_sources += publication.compressed_sources;
                metrics.compressed_tapes += publication.compressed_tapes;
                check_closed(&self.closed)?;
                for (path, stored) in paths.iter().zip(publication.files) {
                    if let Some(file) = state.files.get_mut(path) {
                        file.stored = Some(stored);
                        file.unpublished = false;
                        CrossFileScheduler::mark_persisted(file);
                        file.input.source = Arc::new(String::new());
                        file.tape = None;
                    }
                }
            }
            // A store that stayed busy keeps the files it was given, so what they hold resident
            // is bounded: past the budget their sources and tapes are freed.
            if busy {
                metrics.storage_released_files += state.release_unpublished_over_budget();
            }
        }
        #[cfg(test)]
        {
            LAST_APPLY_STORAGE_RETRIES.set((
                metrics.storage_retries,
                metrics.storage_retry_wait_nanoseconds,
            ));
            LAST_APPLY_UNPUBLISHED_FILES.set(metrics.storage_unpublished_files);
        }
        // Files made resident only for their consumers are evicted again once their results are
        // stored. One the store lacks results of (a transient evaluation, an unstored clause, or an
        // unpublished file) stays resident, as eviction would leave it; rebuilt for its consumers
        // alone, it holds their results only, so its other clauses' stored results join them and it
        // stays resident whole. A clause the store does not answer (a result it no longer holds, or
        // a read that fails) fails, unstored, and the file is transient, so the next apply that
        // offers it evaluates it in full.
        for path in crossfile_evicted {
            let Some(file) = state.files.get(&path) else {
                continue;
            };
            if file.stored.is_none() {
                continue;
            }
            let mut file = state.files.remove(&path).expect("checked above");
            if file.stays_resident() {
                let only = BTreeSet::from([path.clone()]);
                if let Ok(Some(stored)) =
                    evicted_file_states(&state, Some(&only), false).map(|mut stored| stored.pop())
                {
                    file.join_other_clauses(stored);
                }
                let all = make_file(
                    file.input.clone(),
                    state.revision.max(1),
                    &state.code_rules,
                    &mut CandidateSets::new(),
                )
                .candidates;
                for &id in all.regex.iter().chain(&all.ast) {
                    if !file.witnesses.contains_key(&id) && !file.failures.contains_key(&id) {
                        file.failures
                            .insert(id, ("adapter-error", STORED_RESULT_UNAVAILABLE));
                        file.unstored.insert(id);
                        file.transient_failure = true;
                    }
                }
                state.files.insert(path, file);
            }
        }
        metrics.storage_write_nanoseconds = storage_write_started.elapsed().as_nanos() as u64;
        #[cfg(feature = "code-rule-profile")]
        drop(_storage_write_timer);
        // A global clause's answer changes in files the apply did not touch: they are replaced too.
        let mut complete = false;
        if !deferred && !state.dirty_globals.is_empty() {
            let snapshot = Snapshot {
                revision: state.revision,
                files: Vec::new(),
            };
            let dirty = std::mem::take(&mut state.dirty_globals);
            for index in dirty {
                check_closed(&self.closed)?;
                let budget = MatchBudget::work(&self.closed, CLAUSE_WORK_BUDGET);
                let mut evaluation = evaluate_global(&state, index, &snapshot)?;
                #[cfg(test)]
                super::pass_deadline_at_test_phase("global-done");
                // Only exhaustion some step observed cut it short; a finished evaluation is kept.
                if budget.observed() {
                    evaluation =
                        overflow_evaluation("runtime", "global evaluation time budget exceeded");
                }
                metrics.visited_nodes += evaluation.metrics.visited_nodes;
                let previous = state.global_results.insert(index, evaluation);
                complete |= !replace_global_changes(
                    previous.as_ref(),
                    &state.global_results[&index],
                    &mut replaced,
                )?;
            }
        }
        check_closed(&self.closed)?;
        #[cfg(feature = "code-rule-profile")]
        record_workspace_residency(&state);
        if quiet {
            let profile = metrics.clone();
            let serialization_started = Instant::now();
            let output = serde_json::to_string(&json!({ "revision": state.revision,
                "evaluation": evaluation_json(Evaluation { metrics, ..Evaluation::default() }, None, &[], &[]) })).map_err(json_error)?;
            check_closed(&self.closed)?;
            return profile_result(
                ResultText::of(output),
                &profile,
                serialization_started.elapsed(),
            );
        }
        let profile = metrics.clone();
        let serialization_started = Instant::now();
        let output = serialize_result(&state, metrics, deferred, &replaced, complete)?;
        check_closed(&self.closed)?;
        profile_result(output, &profile, serialization_started.elapsed())
    }
}

/// Returns the allocator's freed pages once an apply has dropped its memory (`lib.rs`).
struct PurgeAfterApply;

impl Drop for PurgeAfterApply {
    fn drop(&mut self) {
        crate::purge_allocator_pages();
    }
}

pub struct WorkspaceResultPageTask {
    state: Arc<Mutex<WorkspaceState>>,
    closed: Arc<AtomicBool>,
    cursor: usize,
    maximum_bytes: usize,
}

impl Task for WorkspaceResultPageTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<String> {
        if self.maximum_bytes == 0 || self.maximum_bytes > 64 * 1024 * 1024 {
            return Err(invalid(
                "result page limit must be between 1 byte and 64 MiB",
            ));
        }
        check_closed(&self.closed)?;
        let guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(error) => {
                discard_poisoned_guard(error.into_inner(), &self.state, &self.closed);
                return Err(invalid("policy workspace is unavailable"));
            }
        };
        let mut state = StateLease {
            guard: Some(guard),
            mutex: &self.state,
            closed: &self.closed,
        };
        check_closed(&self.closed)?;
        serialize_result_page(&mut state, self.cursor, self.maximum_bytes)
    }

    fn resolve(&mut self, _env: Env, output: String) -> Result<String> {
        check_closed(&self.closed)?;
        Ok(output)
    }
}

/// Records a finished evaluation as reusable: the clean directories under the workspace root (or,
/// outside Git, the revision) are complete for the code rules' catalog. The writes wait on the
/// shared store's writer, which another process may hold for seconds, so the task runs on a libuv
/// pool thread, and all of them (the publication of files a busy store left unpublished, which
/// holds the workspace lock, and the completion) share one write deadline. Inside the
/// workspace's busy window (`note_store_busy`) it makes no write: it fails as a busy store does,
/// so the host still says the reuse was not recorded, and a write that finds the store busy
/// opens the window.
pub struct WorkspaceCompletionTask {
    state: Arc<Mutex<WorkspaceState>>,
    closed: Arc<AtomicBool>,
}

impl WorkspaceCompletionTask {
    fn lock_state(&self) -> Result<StateLease<'_>> {
        let guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(error) => {
                discard_poisoned_guard(error.into_inner(), &self.state, &self.closed);
                return Err(invalid("policy workspace is unavailable"));
            }
        };
        let state = StateLease {
            guard: Some(guard),
            mutex: &self.state,
            closed: &self.closed,
        };
        check_closed(&self.closed)?;
        Ok(state)
    }
}

/// Publishes the files a busy store left unpublished, after the removals earlier applies left
/// pending. A store still busy past its deadline opens the busy window and keeps them flagged.
fn publish_unpublished(
    state: &mut WorkspaceState,
    storage: &StorageContext,
    closed: &AtomicBool,
) -> Result<()> {
    let paths = state
        .files
        .iter()
        .filter(|(_, file)| file.unpublished && !file.results_only)
        .map(|(path, _)| path.clone())
        .collect::<Vec<_>>();
    if paths.is_empty() {
        return Ok(());
    }
    if !state.pending_removals.is_empty() {
        let pending = state.pending_removals.iter().cloned().collect::<Vec<_>>();
        match storage
            .store
            .remove_overlay_files(storage.workspace_id, &pending)
        {
            Ok(()) => state.pending_removals.clear(),
            Err(StoreError::Busy) => {
                state.note_store_busy("while deleting removed files");
                return Ok(());
            }
            Err(error) => return Err(invalid(error.to_string())),
        }
    }
    for chunk in paths.chunks(64) {
        check_closed(closed)?;
        let persisted = persist_files(
            storage,
            chunk,
            &state.files,
            &state.code_rules,
            CrossFileScheduler::consumers(state),
            closed,
        )?;
        let (publication, busy_during) = match persisted {
            Persisted::Busy { .. } => {
                state.note_store_busy("while publishing results");
                return Ok(());
            }
            Persisted::Published {
                result,
                busy_during,
            } => (result, busy_during),
        };
        check_closed(closed)?;
        for (path, stored) in chunk.iter().zip(publication.files) {
            if let Some(file) = state.files.get_mut(path) {
                file.stored = Some(stored);
                file.unpublished = false;
                CrossFileScheduler::mark_persisted(file);
                file.input.source = Arc::new(String::new());
                file.tape = None;
            }
        }
        match busy_during {
            Some(during) => {
                state.note_store_busy(during);
                return Ok(());
            }
            None => state.note_store_published(),
        }
    }
    Ok(())
}

impl Task for WorkspaceCompletionTask {
    type Output = ();
    type JsValue = ();

    fn compute(&mut self) -> Result<()> {
        check_closed(&self.closed)?;
        // One deadline for the publication under the lock and the completion write after it,
        // so an apply waiting on the lock waits at most one.
        let _deadline = OperationDeadline::enter();
        let (storage, catalog, revision) = {
            let mut state = self.lock_state()?;
            let Some(storage) = state.storage.clone() else {
                return Ok(());
            };
            if state.store_cooling_down() {
                return Err(invalid(StoreError::Busy.to_string()));
            }
            let catalog = candidate_catalog(&state.code_rules)?;
            // Files a busy store left unpublished are published before the completion is
            // recorded; a store that stays busy keeps them unpublished and reports busy.
            publish_unpublished(&mut state, &storage, &self.closed)?;
            if state.store_cooling_down() {
                return Err(invalid(StoreError::Busy.to_string()));
            }
            (storage, catalog, i64::from(state.revision))
        };
        let completed_at = unix_seconds()?;
        let written = if let Some(repository) = &storage.repository {
            let directories = reusable_directories(repository, &storage.repository_prefix);
            storage.store.complete_directories(
                WorkspaceCompletion {
                    workspace_id: storage.workspace_id,
                    revision,
                    completed_at,
                },
                catalog,
                &directories,
            )
        } else {
            storage.store.complete_workspace_revision(
                storage.workspace_id,
                revision,
                catalog,
                completed_at,
            )
        };
        if matches!(written, Err(StoreError::Busy)) {
            self.lock_state()?
                .note_store_busy("while recording repository reuse");
        }
        written.map_err(|error| invalid(error.to_string()))
    }

    fn resolve(&mut self, _env: Env, output: ()) -> Result<()> {
        Ok(output)
    }
}

pub(super) fn check_revision(state: &WorkspaceState, revision: u32) -> Result<()> {
    if revision <= state.revision {
        Err(invalid(format!(
            "out-of-order workspace revision {revision}; current revision is {}",
            state.revision
        )))
    } else {
        Ok(())
    }
}
