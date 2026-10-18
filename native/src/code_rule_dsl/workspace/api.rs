//! The N-API surface: the `CodeRuleDslWorkspace` class, the lease that guards its state, and the
//! storage, apply, completion and result page async tasks.

#[cfg(test)]
use super::ExecutionEngine;
use super::crossfile::{ConsumerRefresh, CrossFileScheduler};
use super::documents;
use super::evaluate::{Job, JobContext, evaluate_file, run_jobs, run_sql_jobs, sql_plans};
use super::global::{compile_query_plan, compile_relation_producer, evaluate_global};
use super::index::{RepositoryIndexOptions, WorkspaceIndexTask, reusable_directories};
use super::ingest::{
    Request, apply_request, decode_buffer_request, file_needs_update, inline_input, is_cheap,
    is_file_local, make_file, reclaim_candidate_sets,
};
#[cfg(feature = "code-rule-profile")]
use super::memory::record_workspace_residency;
use super::memory::workspace_memory_profile;
use super::persisted::{candidate_catalog, persist_files};
use super::results::{
    ResultText, profile_result, replace_global_changes, serialize_result, serialize_result_page,
};
use super::storage::{attach_storage, unix_seconds};
use super::{CandidateSets, FILE_WORK_BUDGET, WorkspaceState, check_closed};
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::budget::MatchBudget;
use crate::code_rule_dsl::eval::{evaluation_json, overflow_evaluation};
use crate::code_rule_dsl::json::{invalid, json_error};
use crate::code_rule_dsl::model::{ClauseKind, CodeRuleScope, Evaluation, Metrics, Snapshot};
use crate::code_rule_dsl::placement::parse_placement;
use crate::code_rule_dsl::query;
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::{matcher::GrammarMatcher, model::Query, profile};
use crate::content_store::WorkspaceCompletion;
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

#[napi]
impl CodeRuleDslWorkspace {
    #[napi(constructor)]
    pub fn new() -> Self {
        let state = WorkspaceState {
            instance: WORKSPACE_INSTANCES.fetch_add(1, Ordering::Relaxed),
            ..WorkspaceState::default()
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            scopes: Mutex::new(Vec::new()),
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Tests can select the semantic oracle. Production always uses SQLite and the packed virtual
    /// table, with no engine selector and no matcher fallback.
    #[cfg(test)]
    pub fn set_execution_engine(&self, engine: String) -> Result<()> {
        let engine = match engine.as_str() {
            "matcher" => ExecutionEngine::Matcher,
            "sqlite" => ExecutionEngine::Sqlite,
            _ => return Err(invalid("test engine must equal matcher or sqlite")),
        };
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| invalid("policy workspace is evaluating"))?;
        if self.closed.load(Ordering::Acquire) || state.revision != 0 {
            return Err(invalid("test engine must be selected before evaluation"));
        }
        state.execution_engine = engine;
        state.sql_plans = None;
        Ok(())
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
    /// `{"folder", "mode"}` entries that places this workspace's copy of the code rule per folder
    /// of the checkout (`mode: null` is off there); the compiled code rule itself is never changed.
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
                        let subject = compile_relation_producer(subject, clause, index, code_rule);
                        let evidence =
                            compile_relation_producer(evidence, clause, index, code_rule);
                        state
                            .relation_plans
                            .insert((index, clause.id.clone()), vec![subject, evidence]);
                    }
                    ClauseKind::Budget { query, .. } => {
                        let id = compile_relation_producer(query, clause, index, code_rule);
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
        state.sql_plans = None;
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
        if !is_cheap(&state, &request) {
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
            let sql = sql_plans(&mut state);
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
                sql.as_deref(),
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
    pub fn maintain_storage(&self) -> Result<String> {
        let storage = self
            .state
            .lock()
            .map_err(|_| invalid("policy workspace is unavailable"))?
            .storage
            .clone();
        let Some(storage) = storage else {
            return Ok(json!({ "enabled": false }).to_string());
        };
        let now = unix_seconds()?;
        storage
            .store
            .renew_workspace_lease(storage.workspace_id, &storage.owner, now + 120)
            .map_err(|error| invalid(error.to_string()))?;
        let report = storage
            .store
            .collect(now, crate::content_store::MAX_COLLECTION_BATCH)
            .map_err(|error| invalid(error.to_string()))?;
        Ok(json!({
            "enabled": true,
            "collected": {
                "expiredPins": report.expired_pins,
                "retiredWorkspaces": report.retired_workspaces,
                "snapshotFiles": report.snapshot_files,
                "snapshots": report.snapshots,
                "candidateRows": report.candidate_rows,
                "candidateChecks": report.candidate_checks,
                "candidateCatalogs": report.candidate_catalogs,
                "tapes": report.tapes,
                "sources": report.sources,
            }
        })
        .to_string())
    }

    #[napi]
    pub fn evict_inactive(&self) -> Result<bool> {
        let mut state = self
            .state
            .try_lock()
            .map_err(|_| invalid("policy workspace is evaluating"))?;
        // Cross-file clauses read the evidence index and the call graph, which eviction keeps: they
        // are summaries of the files, and an edit after eviction updates the graph in place.
        if state.storage.is_none() || state.files.values().any(|file| file.stored.is_none()) {
            return Ok(false);
        }
        // A file whose last evaluation was transient has no stored results to page; it stays.
        let evicted = state
            .files
            .values()
            .filter(|file| !file.transient_failure)
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
        state.files.retain(|_, file| file.transient_failure);
        state.documents.clear();
        state.candidate_sets = CandidateSets::new();
        state.grammars.clear();
        if let Some(storage) = &state.storage {
            storage
                .store
                .release_memory()
                .map_err(|error| invalid(error.to_string()))?;
        }
        query::release_connections();
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
        let storage = state.storage.take();
        *state = WorkspaceState::default();
        if let Some(storage) = storage {
            let _ = storage.store.release_memory();
        }
        query::release_connections();
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

/// What a storage attach did. No store attaches where Git cannot describe the workspace; `reason`
/// then says why, unless the root is simply outside any Git repository.
#[napi(object)]
#[derive(Debug)]
pub struct StorageAttachment {
    pub attached: bool,
    pub reason: Option<String>,
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
        let discovered = match crate::git_workspace::discover(std::path::Path::new(&self.root)) {
            Ok(discovered) => discovered,
            Err(error) => {
                return Ok(StorageAttachment {
                    attached: false,
                    reason: (!error.is_no_repository()).then(|| error.to_string()),
                });
            }
        };
        let context = attach_storage(&self.root, discovered, std::mem::take(&mut self.store_path))?;
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
        let deferred = request.defer_global;
        let quiet = request.quiet;
        let mut replaced = request
            .files
            .iter()
            .map(|file| file.path.clone())
            .chain(request.removed.iter().cloned())
            .collect::<BTreeSet<_>>();
        let refreshed = apply_request(&mut state, request)?;
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
        let sql = sql_plans(&mut state);
        let storage = state.storage.clone();
        if let Some(sql) = sql.as_deref() {
            let context = JobContext {
                code_rules: &code_rules,
                storage: storage.as_ref(),
                closed: &self.closed,
                summarize: false,
            };
            let error = match run_sql_jobs(&mut jobs, &context, sql) {
                Err(error) => error,
                Ok(_) => invalid("packed SQL apply publication handoff is not implemented"),
            };
            for Job { path, file, .. } in jobs {
                state.dirty_files.insert(path.clone());
                state.insert_file(path, file);
            }
            return Err(error);
        }
        let producer_batch =
            CrossFileScheduler::prepare(&mut state, &mut jobs, &code_rules, deferred);
        let context = JobContext {
            code_rules: &code_rules,
            storage: storage.as_ref(),
            closed: &self.closed,
            summarize: CrossFileScheduler::summarizes(&producer_batch),
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
        if let Some(storage) = state.storage.clone() {
            for paths in persistence_paths.chunks(64) {
                check_closed(&self.closed)?;
                let publication = persist_files(
                    &storage,
                    paths,
                    &state.files,
                    &state.code_rules,
                    CrossFileScheduler::consumers(&state),
                    &self.closed,
                )?;
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
                        CrossFileScheduler::mark_persisted(file);
                        file.input.source = Arc::new(String::new());
                        file.tape = None;
                    }
                }
            }
        }
        // Files made resident only for their consumers are evicted again once their results are
        // stored; a transient evaluation stays resident, as eviction would leave it.
        for path in crossfile_evicted {
            if state
                .files
                .get(&path)
                .is_some_and(|file| file.stored.is_some() && !file.transient_failure)
            {
                state.files.remove(&path);
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
                let budget = MatchBudget::work(&self.closed, FILE_WORK_BUDGET);
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
/// outside Git, the revision) are complete for the code rules' catalog. The write waits on the
/// shared store's writer, which another process may hold for seconds, so it runs on a libuv pool
/// thread and holds the workspace lock only to read what it records.
pub struct WorkspaceCompletionTask {
    state: Arc<Mutex<WorkspaceState>>,
    closed: Arc<AtomicBool>,
}

impl Task for WorkspaceCompletionTask {
    type Output = ();
    type JsValue = ();

    fn compute(&mut self) -> Result<()> {
        check_closed(&self.closed)?;
        let (storage, catalog, revision) = {
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
            let Some(storage) = state.storage.clone() else {
                return Ok(());
            };
            let catalog = candidate_catalog(&state.code_rules)?;
            (storage, catalog, i64::from(state.revision))
        };
        let completed_at = unix_seconds()?;
        if let Some(repository) = &storage.repository {
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
        }
        .map_err(|error| invalid(error.to_string()))
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
