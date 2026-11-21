//! The N-API surface: the `CodeRuleDslWorkspace` class, the lease that guards its state, and the
//! storage, apply, completion and result page async tasks.

use super::crossfile::{ConsumerRefresh, CrossFileScheduler};
use super::documents;
use super::evaluate::{Job, JobContext, evaluate_file, run_jobs};
use super::global::evaluate_batched_relations;
use super::global::{compile_query_plan, compile_relation_producer, evaluate_global};
use super::index::{RepositoryIndexOptions, WorkspaceIndexTask, reusable_directories};
use super::ingest::{
    Request, apply_request, decode_buffer_request, file_needs_update, inline_input, is_cheap,
    is_file_local, make_file, reclaim_candidate_sets,
};
use super::memory::workspace_memory_profile;
use super::persisted::{Persisted, candidate_catalog, persist_files};
use super::results::{
    ResultText, evicted_file_states, profile_result, replace_global_changes, serialize_result,
    serialize_result_page,
};
use super::secret_names::{SecretNamePatterns, SecretNames};
use super::storage::{Attached, StorageContext, attach_storage, unix_seconds};
use super::{
    CLAUSE_WORK_BUDGET, CandidateSets, STORED_RESULT_UNAVAILABLE, WorkspaceState, check_closed,
};
use crate::code_rule_dsl::budget::MatchBudget;
use crate::code_rule_dsl::eval::evidence::Side;
use crate::code_rule_dsl::eval::{evaluation_json, overflow_evaluation};
use crate::code_rule_dsl::json::{invalid, json_error};
use crate::code_rule_dsl::language::{self, adapter::package_manifests};
use crate::code_rule_dsl::model::{
    ClauseKind, CodeRuleScope, CompiledCodeRule, Evaluation, Metrics, Query, Snapshot,
};
use crate::code_rule_dsl::placement::parse_placement;
use crate::code_rule_dsl::prof::release_allocator_memory;
use crate::code_rule_dsl::rule_api::CompiledCodeRuleDsl;
#[cfg(feature = "code-rule-profile")]
use crate::code_rule_dsl::{matcher::GrammarMatcher, profile};
use crate::content_store::{OperationDeadline, StoreError, WorkspaceCompletion};
use crate::content_store::analysis_scope::{self, AnalysisOwner, AnalysisRun};
use napi::bindgen_prelude::{AsyncTask, Buffer};
use napi::{Env, Result, Task};
use napi_derive::napi;
use serde_json::json;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::time::{Duration, Instant};

const INLINE_TIME_BUDGET: Duration = Duration::from_millis(5);

/// Distinct workspace instances in this process.
static WORKSPACE_INSTANCES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[napi]
pub struct CodeRuleDslWorkspace {
    #[napi(skip)]
    pub state: Arc<Mutex<WorkspaceState>>,
    /// Each registered code rule's scope, and whether the code rule reads text
    /// (`CompiledCodeRule::reads_text`), answered without the state lock.
    #[napi(skip)]
    pub scopes: Mutex<Vec<(CodeRuleScope, bool)>>,
    /// Whether a code rule reads `file.package.serverGraph`, answered without the state lock.
    pub(super) server_graph: AtomicBool,
    /// Whether a code rule has an AST clause in a family whose files read their nearest
    /// `package.json`, answered without the state lock.
    pub(super) package_manifests: AtomicBool,
    #[napi(skip)]
    pub closed: Arc<AtomicBool>,
    /// What the workspace's runs share by design (decision 0442): its own, as apart from another workspace's as one
    /// in another process.
    #[napi(skip)] pub shared: Arc<crate::code_rule_dsl::logic::profile::Shared>,
}

impl Default for CodeRuleDslWorkspace {
    fn default() -> Self {
        Self::new()
    }
}

/// One resolved commit or tree, pinned for the lifetime of a scan.
#[cfg(not(feature = "headless"))]
#[napi]
pub struct GitRevisionSource {
    snapshot: Arc<crate::git_workspace::snapshot::Snapshot>,
}

#[cfg(not(feature = "headless"))]
#[napi(object)]
pub struct GitDirectoryEntry {
    pub name: Option<String>,
    pub name_bytes: Option<Buffer>,
    pub mode: u32,
}

#[cfg(not(feature = "headless"))]
#[napi(object)]
pub struct GitSourceFailure {
    pub path: String,
    pub message: String,
    pub path_identity: GitPathIdentity,
}

#[cfg(not(feature = "headless"))]
#[napi(object)]
pub struct GitPathIdentity {
    pub encoding: String,
    pub value: String,
}

#[cfg(not(feature = "headless"))]
#[napi(object)]
pub struct GitSourceRead {
    pub status: String,
    pub bytes: Option<Buffer>,
}

#[cfg(not(feature = "headless"))]
#[napi(object)]
pub struct GitPathStatus {
    pub status: String,
    pub mode: Option<u32>,
}

#[cfg(not(feature = "headless"))]
fn file_byte_limit(limit: f64) -> Result<u64> {
    if !(0.0..=f64::from(u32::MAX)).contains(&limit) || limit.fract() != 0.0 {
        return Err(invalid("file byte limit must be an integer between 0 and 4294967295"));
    }
    Ok(limit as u64)
}

#[cfg(not(feature = "headless"))]
impl GitRevisionSource {
    pub fn pinned_snapshot(&self) -> Arc<crate::git_workspace::snapshot::Snapshot> {
        Arc::clone(&self.snapshot)
    }
}

#[cfg(not(feature = "headless"))]
#[napi]
impl GitRevisionSource {
    #[napi(constructor)]
    pub fn new(root: String, revision: String, analysis_scope: String, invocation: Option<String>) -> Result<Self> {
        let snapshot = crate::git_workspace::snapshot::Snapshot::scoped_from(
            Path::new(&root), &revision, &analysis_scope, Path::new(invocation.as_deref().unwrap_or(&root)),
        )
            .map_err(invalid)?;
        Ok(Self { snapshot: Arc::new(snapshot) })
    }

    #[napi(getter)]
    pub fn tree(&self) -> String {
        self.snapshot.tree()
    }

    #[napi(getter)]
    pub fn commit(&self) -> Option<String> {
        self.snapshot.commit()
    }

    #[napi]
    pub fn invalid_paths(&self) -> Vec<String> {
        self.snapshot.invalid_paths()
    }

    #[napi]
    pub fn source_failures(&self) -> Vec<GitSourceFailure> {
        use crate::git_workspace::snapshot::PathIdentity;
        self.snapshot.source_failures().into_iter().map(|failure| {
            let (encoding, value) = match failure.path_identity {
                PathIdentity::Utf8(path) => ("utf8", path),
                PathIdentity::Hex(bytes) => ("hex", bytes),
            };
            GitSourceFailure { path: failure.path, message: failure.message, path_identity: GitPathIdentity { encoding: encoding.into(), value } }
        }).collect()
    }

    #[napi]
    pub fn contains_path(&self, path: String) -> Result<bool> {
        self.snapshot.contains_path(Path::new(&path)).map_err(|err| invalid(err.to_string()))
    }

    #[napi]
    pub fn path_status(&self, path: String) -> Result<GitPathStatus> {
        let (status, mode) = self.snapshot.path_status(Path::new(&path)).map_err(|err| invalid(err.to_string()))?;
        Ok(GitPathStatus { status: status.to_owned(), mode: mode.map(u32::from) })
    }

    #[napi]
    pub fn read_directory(&self, path: String) -> Result<Option<Vec<GitDirectoryEntry>>> {
        let entries = self.snapshot.directory_entries(&path).map_err(|err| invalid(err.to_string()))?;
        Ok(entries.map(|entries| entries.into_iter()
            .map(|(name, mode)| {
                let (name, name_bytes) = match String::from_utf8(name) {
                    Ok(name) => (Some(name), None),
                    Err(error) => (None, Some(error.into_bytes().into())),
                };
                GitDirectoryEntry { name, name_bytes, mode: u32::from(mode) }
            })
            .collect()))
    }

    /// An oversized file has no result. A read failure is an error.
    #[napi]
    pub fn read_file(&self, path: String, limit: f64) -> Result<Option<Buffer>> {
        match self.snapshot.read_source(&path, file_byte_limit(limit)?) {
            Ok(bytes) => Ok(Some(bytes.into())),
            Err(err) if err.kind() == std::io::ErrorKind::FileTooLarge => Ok(None),
            Err(err) => Err(invalid(err.to_string())),
        }
    }

    #[napi]
    pub fn read_source_checked(&self, path: String, limit: f64) -> Result<GitSourceRead> {
        let (status, bytes) = match self.snapshot.read_source(&path, file_byte_limit(limit)?) {
            Ok(bytes) => ("ok", Some(bytes.into())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => ("absent", None),
            Err(err) if err.kind() == std::io::ErrorKind::FileTooLarge => ("too-large", None),
            Err(err) => return Err(invalid(format!("cannot read {path}: {err}"))),
        };
        Ok(GitSourceRead { status: status.to_owned(), bytes })
    }

    #[napi]
    pub fn read_source(&self, path: String) -> Option<napi::bindgen_prelude::Buffer> {
        self.snapshot.read_source(&path, 32_000_000).ok().map(Into::into)
    }

    #[napi]
    pub fn restrict_paths(&self, paths: Vec<String>) -> Result<()> {
        self.snapshot.restrict_paths(paths).map_err(|err| invalid(err.to_string()))
    }

    #[napi]
    pub fn set_source_languages(&self, json: String) -> Result<()> {
        self.snapshot.set_source_languages(serde_json::from_str(&json).map_err(json_error)?);
        Ok(())
    }
}

#[napi]
pub struct AnalysisScopeRun(Mutex<AnalysisRun>);

fn analysis_generation(value: f64) -> Result<i64> {
    if !(0.0..=analysis_scope::MAX_GENERATION as f64).contains(&value) || value.fract() != 0.0 {
        return Err(invalid("analysis generation must be a nonnegative safe integer"));
    }
    Ok(value as i64)
}

#[napi]
impl AnalysisScopeRun {
    #[napi(constructor)]
    pub fn new(store_path: String, value: String, directory: bool, id: String, generation: f64, home: Option<String>, expected_store: Option<String>) -> Result<Self> {
        let generation = analysis_generation(generation)?;
        let scope = if directory { analysis_scope::directory_in(Path::new(&value), home.as_deref()) }
            else { analysis_scope::named(&value).ok_or_else(|| invalid("analysis scope name must not be empty"))? };
        let store = crate::content_store::ContentStore::open(Path::new(&store_path)).map_err(|e| invalid(e.to_string()))?;
        check_store_identity(&store, expected_store.as_deref()).map_err(|err| invalid(err.to_string()))?;
        let mut run = AnalysisRun::begin_at(store, &scope, &id, generation).map_err(|e| invalid(e.to_string()))?;
        if directory { run.set_directory_scope(Path::new(&value)); }
        Ok(Self(Mutex::new(run)))
    }

    #[cfg(not(feature = "headless"))]
    #[napi]
    pub fn select_source(&self, root: String, source: Option<&GitRevisionSource>) -> Result<()> {
        let selected = source.map_or_else(|| analysis_scope::RunSource::directory(Path::new(&root)), |view| view.snapshot.run_source());
        self.0.lock().unwrap_or_else(|e| e.into_inner()).select_source(selected).map_err(|e| invalid(e.to_string()))
    }

    #[napi]
    pub fn finish(&self, succeeded: bool) -> Result<Option<String>> {
        let mut run = self.0.lock().unwrap_or_else(|err| err.into_inner());
        run.finish(succeeded).map_err(|e| invalid(e.to_string()))?;
        Ok(run.publication_warning())
    }

    #[napi(getter)]
    pub fn publication(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|err| err.into_inner()).publication.map(|p| p.as_str().to_owned())
    }

    #[napi(getter)]
    pub fn store_identity(&self) -> String {
        self.0.lock().unwrap_or_else(|err| err.into_inner()).store_identity().to_string()
    }

    #[napi(getter)]
    pub fn acquisition(&self) -> String {
        self.0.lock().unwrap_or_else(|err| err.into_inner()).acquisition().to_string()
    }

    #[napi]
    pub fn warn(&self, message: String) {
        self.0.lock().unwrap_or_else(|err| err.into_inner()).owner.warn(message);
    }
}

#[cfg(not(feature = "headless"))]
impl AnalysisScopeRun {
    pub fn owner_for(&self, store: &Path, root: &Path, source: Option<&GitRevisionSource>) -> Result<AnalysisOwner> {
        let mut run = self.0.lock().unwrap_or_else(|err| err.into_inner());
        if !run.is_open() {
            return Err(invalid("analysis scope run has finished"));
        }
        if store != run.store_path() && !store.canonicalize().ok().zip(run.store_path().canonicalize().ok())
            .is_some_and(|(requested, owned)| requested == owned) {
            return Err(invalid("analysis scope run belongs to another content store"));
        }
        let selected = source.map_or_else(|| analysis_scope::RunSource::directory(root), |s| s.snapshot.run_source());
        run.select_source(selected).map_err(|e| invalid(e.to_string()))?;
        Ok(run.owner.clone())
    }
}

fn check_store_identity(store: &crate::content_store::ContentStore, expected: Option<&str>) -> std::result::Result<(), StoreError> {
    if expected.is_some_and(|id| id.parse::<i64>().ok() != Some(store.identity())) {
        return Err(StoreError::Invalid(format!("analysis store identity changed at {}", store.database_path().display())));
    }
    Ok(())
}

/// The Node environment's machine (decision 0442). The addon's caller is the environment: it makes one machine and
/// keeps it in its instance data, so every workspace it makes shares its pool, its CPU slot and its learned model,
/// each workspace on a shared object of its own. Another environment (a worker thread) has its own. Its pool's
/// threads are all its own, the loading thread none of them: JavaScript may park that thread waiting on a background
/// call, so even a pool of one thread has its own worker (98c76de865; the CLI's `main` alone takes its thread,
/// `Machine::here`).
fn environment_machine(env: &napi::Env) -> Result<Arc<crate::code_rule_dsl::logic::profile::Machine>> {
    if let Some(machine) = env.get_instance_data::<Arc<crate::code_rule_dsl::logic::profile::Machine>>()? {
        return Ok(Arc::clone(machine));
    }
    let machine = crate::code_rule_dsl::logic::profile::Machine::new();
    env.set_instance_data(Arc::clone(&machine), (), |_| {})?;
    Ok(machine)
}

#[napi]
impl CodeRuleDslWorkspace {
    /// A workspace on the tests' shared object (a product build's: one of its own), for a caller with no Node
    /// environment.
    pub fn new() -> Self {
        Self::on(crate::code_rule_dsl::logic::profile::default_shared())
    }

    fn on(shared: Arc<crate::code_rule_dsl::logic::profile::Shared>) -> Self {
        let state = WorkspaceState {
            instance: WORKSPACE_INSTANCES.fetch_add(1, Ordering::Relaxed),
            ..WorkspaceState::default()
        };
        Self {
            state: Arc::new(Mutex::new(state)),
            scopes: Mutex::new(Vec::new()),
            server_graph: AtomicBool::new(false),
            package_manifests: AtomicBool::new(false),
            closed: Arc::new(AtomicBool::new(false)),
            shared,
        }
    }
}

#[napi]
impl CodeRuleDslWorkspace {
    /// A workspace on a shared object of its own, on the environment's machine (`environment_machine`).
    #[napi(constructor)]
    pub fn construct(env: napi::Env) -> Result<Self> {
        Ok(Self::on(crate::code_rule_dsl::logic::profile::Shared::on(&environment_machine(&env)?)))
    }

    /// Attaches the content store file at `store_path` to the Git repository containing `root`.
    /// The host chooses the file (each build owns its own); the binding never derives one. The
    /// store never holds a copy of a file `secret_names` names: such a file is evaluated as any
    /// other and stays resident instead.
    /// Not in a `headless` build, where `#[napi(object)]` derives nothing (napi-derive `noop`).
    #[cfg(not(feature = "headless"))]
    #[napi]
    pub fn enable_storage_async(
        &self,
        root: String,
        store_path: String,
        secret_names: SecretNamePatterns,
        analysis: Option<&AnalysisScopeRun>,
        generation: Option<f64>,
        home: Option<String>,
        expected_store: Option<String>,
    ) -> Result<AsyncTask<WorkspaceStorageTask>> {
        let generation = analysis_generation(generation.unwrap_or_default())?;
        let owner = analysis.map(|run| run.owner_for(Path::new(&store_path), Path::new(&root), None)).transpose()?;
        let mut task = self.storage_task(root, store_path, &secret_names)?;
        task.analysis_owner = owner;
        task.analysis_generation = generation;
        task.analysis_home = home;
        task.expected_store = expected_store;
        Ok(AsyncTask::new(task))
    }

    #[cfg(not(feature = "headless"))]
    #[napi]
    pub fn enable_revision_storage_async(
        &self,
        source: &GitRevisionSource,
        store_path: String,
        secret_names: SecretNamePatterns,
        analysis: Option<&AnalysisScopeRun>,
    ) -> Result<AsyncTask<WorkspaceStorageTask>> {
        let owner = analysis.map(|run| run.owner_for(Path::new(&store_path), source.snapshot.root(), Some(source))).transpose()?;
        let mut task = self.storage_task(
            source.snapshot.root().to_string_lossy().into_owned(),
            store_path,
            &secret_names,
        )?;
        task.snapshot = Some(Arc::clone(&source.snapshot));
        task.analysis_owner = owner;
        Ok(AsyncTask::new(task))
    }

    pub fn storage_task(
        &self,
        root: String,
        store_path: String,
        secret_names: &SecretNamePatterns,
    ) -> Result<WorkspaceStorageTask> {
        if store_path.is_empty() {
            return Err(invalid("content store path is required"));
        }
        Ok(WorkspaceStorageTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
            shared: Arc::clone(&self.shared),
            root,
            snapshot: None,
            analysis_owner: None,
            analysis_generation: 0,
            expected_store: None,
            analysis_home: None,
            store_path: PathBuf::from(store_path),
            secret_names: Arc::new(SecretNames::compile(secret_names)?),
        })
    }

    /// Registers `compiled` with this workspace. `placement`, when given, is the JSON array of
    /// `{"folder", "mode", "suppressions"?}` entries that places this workspace's copy of the code
    /// rule per folder of the checkout (`mode: null` is off there; `suppressions: "ignore"` reports
    /// violations a linter's suppression comment would silence, `"honour"` or no `suppressions`
    /// lets the comment silence them); the compiled code rule itself is never changed.
    #[napi(catch_unwind)]
    pub fn add_code_rule(
        &mut self,
        compiled: &CompiledCodeRuleDsl,
        placement: Option<String>,
    ) -> Result<()> {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        let placement = placement.as_deref().map(parse_placement).transpose()?;
        let guard = match self.state.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(error)) => {
                return Err(close_poisoned(
                    error.into_inner(),
                    &self.state,
                    &self.closed,
                ));
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
                    // P's relational producers (test-only, open item O18).
                    ClauseKind::Relational(plan) => {
                        let ids = plan
                            .inputs
                            .iter()
                            .map(|(_, input)| {
                                let id = compile_relation_producer(
                                    input,
                                    Side::Counted,
                                    clause,
                                    index,
                                    code_rule,
                                );
                                if plan.per_file {
                                    let producer = &mut code_rule.clauses[id.1];
                                    producer.decision = true;
                                    producer.violation.project.clear();
                                    producer.violation.each = None;
                                    producer.violation.node_regexes.clear();
                                    producer.violation.gap_tests.clear();
                                }
                                id
                            })
                            .collect();
                        state.relation_plans.insert((index, clause.id.clone()), ids);
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
            .push((resident.scope.clone(), resident.reads_text()));
        if resident.reads_server_graph() {
            self.server_graph.store(true, Ordering::Release);
        }
        if reads_package_manifests(&resident) {
            self.package_manifests.store(true, Ordering::Release);
        }
        #[cfg(feature = "code-rule-profile")]
        for clause in &resident.clauses {
            if let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind {
                for GrammarMatcher {
                    language,
                    matcher,
                    anchors,
                    ..
                } in matchers.iter()
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
        CrossFileScheduler::register(&mut state, index, &mut resident)?;
        // Global clauses read import targets through their synthetic file-local leaves.
        for (clause_index, clause) in resident.clauses.iter().enumerate() {
            if matches!(&clause.kind, ClauseKind::Forbid(query) if is_file_local(clause) && query.reads_import_targets())
            {
                state.import_clauses.insert((index, clause_index));
            }
        }
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

    #[napi(catch_unwind)]
    pub fn accepts_path(&self, path: String) -> bool {
        self.scope_holds(&path, false)
    }

    /// `accepts_path` over the code rules that read text alone: whether a file no code-rule
    /// language lists is read, as `text`.
    #[napi]
    pub fn accepts_text_path(&self, path: String) -> bool {
        self.scope_holds(&path, true)
    }

    /// Whether an open workspace's code rule has `path` in scope; with `text`, a code rule that
    /// reads text.
    fn scope_holds(&self, path: &str, text: bool) -> bool {
        !self.closed.load(Ordering::Acquire)
            && self.scopes.lock().is_ok_and(|scopes| {
                scopes
                    .iter()
                    .any(|(scope, reads_text)| (!text || *reads_text) && scope.matches(path))
            })
    }

    /// Whether a code rule reads `file.package.serverGraph`: a host then sends each file of a
    /// package naming `next` its membership (`serverGraph`).
    #[napi]
    pub fn reads_server_graph(&self) -> bool {
        self.server_graph.load(Ordering::Acquire)
    }

    /// Whether a code rule has an AST clause in a family whose files read their nearest
    /// `package.json` (ECMA): a host then treats `package.json` and `tsconfig`/`jsconfig` edits as
    /// inputs, since they change those files' `package` facts.
    #[napi]
    pub fn reads_package_manifests(&self) -> bool {
        self.package_manifests.load(Ordering::Acquire)
    }

    #[napi(catch_unwind)]
    pub fn try_apply(&self, request_json: String) -> Result<Option<String>> {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        if self.closed.load(Ordering::Acquire) || request_json.len() > 8192 {
            return Ok(None);
        }
        let Some(_slot) = cpu_slot().try_lock_arc() else {
            return Ok(None);
        };
        // Upkeep yields to the apply (decision 0190 §8).
        let _run = rerail_accel::upkeep::run_begins(&[]);
        let guard = match self.state.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(error)) => {
                // Closed now, so it answers as a closed workspace does: the async apply fails.
                drop(close_poisoned(
                    error.into_inner(),
                    &self.state,
                    &self.closed,
                ));
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
            shared: Arc::clone(&self.shared),
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
            shared: Arc::clone(&self.shared),
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
            shared: Arc::clone(&self.shared),
            options,
        })
    }

    /// Record the current revision as reusable, on a libuv pool thread (`WorkspaceCompletionTask`).
    #[napi]
    pub fn complete_storage_revision_async(&self) -> AsyncTask<WorkspaceCompletionTask> {
        AsyncTask::new(self.completion_task())
    }

    #[napi]
    pub fn retain_analysis_scope_async(&self) -> AsyncTask<WorkspaceCompletionTask> {
        let mut task = self.completion_task();
        task.retain_scope = true;
        AsyncTask::new(task)
    }

    pub fn completion_task(&self) -> WorkspaceCompletionTask {
        WorkspaceCompletionTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
            retain_scope: false,
            shared: Arc::clone(&self.shared),
        }
    }

    /// One page of the current revision's ordered results, computed on a libuv pool thread: the
    /// page at cursor 0 builds the ordered result index, which the last page releases.
    #[napi]
    pub fn result_page_async(
        &self,
        cursor: u32,
        maximum_bytes: u32,
        include_fingerprints: Option<bool>,
    ) -> AsyncTask<WorkspaceResultPageTask> {
        let mut task = self.result_page_task(cursor, maximum_bytes);
        task.include_fingerprints = include_fingerprints.unwrap_or_default();
        AsyncTask::new(task)
    }

    pub fn result_page_task(&self, cursor: u32, maximum_bytes: u32) -> WorkspaceResultPageTask {
        WorkspaceResultPageTask {
            state: Arc::clone(&self.state),
            closed: Arc::clone(&self.closed),
            shared: Arc::clone(&self.shared),
            cursor: cursor as usize,
            maximum_bytes: maximum_bytes as usize,
            include_fingerprints: false,
        }
    }

    #[napi(catch_unwind)]
    pub fn memory_profile(&self) -> Result<String> {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        let state = match self.state.lock() {
            Ok(guard) => StateLease {
                guard: Some(guard),
                mutex: &self.state,
                closed: &self.closed,
            },
            Err(error) => {
                return Err(close_poisoned(
                    error.into_inner(),
                    &self.state,
                    &self.closed,
                ));
            }
        };
        serde_json::to_string(&workspace_memory_profile(&state)).map_err(json_error)
    }

    /// Bound the bytes of documents the storage contract keeps resident; loads beyond it evict the
    /// least recently used documents, which later loads bring back from the store or a reparse.
    #[napi(catch_unwind)]
    pub fn set_document_residency_limit(&self, bytes: u32) -> Result<()> {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        self.state_now()?.documents.set_limit(bytes as usize);
        Ok(())
    }

    /// The absolute folder on disk the workspace's paths are relative to: import target
    /// resolution reads the files below it the workspace does not hold, and nothing above it.
    /// Without one, targets resolve among the held files alone. A changed root makes the next
    /// apply that defers nothing evaluate again the files whose clauses read import targets.
    #[napi(catch_unwind)]
    pub fn set_root(&self, root: String) -> Result<()> {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        let root = PathBuf::from(root);
        if !root.is_absolute() {
            return Err(invalid("workspace root must be an absolute path"));
        }
        let mut state = self.state_now()?;
        if state.import_root.as_ref() != Some(&root) {
            state.import_root = Some(root);
            state.import_generation += 1;
        }
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn set_scan_fingerprint_prefix(&self, prefix: String) -> Result<()> {
        let mut state = self.state_now()?;
        state.scan_fingerprints = Some(super::scan_fingerprints::ScanFingerprints::new(prefix));
        Ok(())
    }

    #[napi(catch_unwind)]
    pub fn evict_inactive(&self) -> Result<bool> {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        let mut state = self.state_now()?;
        // Cross-file clauses read the evidence index and the call graph, which eviction keeps: they
        // are summaries of the files, and an edit after eviction updates the graph in place.
        // A file without stored results stays only when a busy store left it unpublished, or the
        // store never holds it (`withheld`).
        if state.storage.is_none()
            || state
                .files
                .values()
                .any(|file| file.stored.is_none() && !file.unpublished && !file.withheld)
        {
            return Ok(false);
        }
        // A file the store lacks results of (`FileState::stays_resident`: a transient evaluation,
        // an unstored clause, results a busy store left unpublished, or a withheld file) has none
        // to page; it stays.
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
    #[napi(catch_unwind)]
    pub fn node_reference(
        &self,
        path: String,
        start_byte: u32,
        end_byte: u32,
    ) -> Result<Option<String>> {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        let mut state = self.state_now()?;
        documents::reference(&mut state, &path, start_byte as usize, end_byte as usize)?
            .map(|reference| serde_json::to_string(&reference).map_err(json_error))
            .transpose()
    }

    /// The node a reference names, as JSON, loading its file through the storage contract. A
    /// reference whose file version is no longer current is refused.
    #[napi(catch_unwind)]
    pub fn resolve_node_reference(&self, reference: String) -> Result<String> {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        let reference: documents::NodeReference =
            serde_json::from_str(&reference).map_err(json_error)?;
        let mut state = self.state_now()?;
        serde_json::to_string(&documents::resolve(&mut state, &reference)?).map_err(json_error)
    }

    #[napi(catch_unwind)]
    pub fn close(&self) {
        // On the workspace's shared object (decision 0442).
        let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
        self.closed.store(true, Ordering::Release);
        clear_if_unlocked(&self.state);
        match self.scopes.lock() {
            Ok(mut scopes) => *scopes = Vec::new(),
            Err(error) => *error.into_inner() = Vec::new(),
        }
    }

    /// The state without waiting: an evaluation holding it answers "evaluating", which the host
    /// may retry; a state a panic poisoned closes the workspace (`close_poisoned`).
    fn state_now(&self) -> Result<StateLease<'_>> {
        match self.state.try_lock() {
            Ok(guard) => Ok(StateLease {
                guard: Some(guard),
                mutex: &self.state,
                closed: &self.closed,
            }),
            Err(TryLockError::Poisoned(error)) => Err(close_poisoned(
                error.into_inner(),
                &self.state,
                &self.closed,
            )),
            Err(TryLockError::WouldBlock) => Err(invalid("policy workspace is evaluating")),
        }
    }
}

/// Whether an AST clause of `code_rule` matches in a family whose files read their nearest
/// `package.json` (`Adapter::PACKAGE_MANIFESTS`).
fn reads_package_manifests(code_rule: &CompiledCodeRule) -> bool {
    fn in_manifest_family(query: &Query) -> bool {
        match query {
            Query::Ast { matchers, .. } => matchers
                .iter()
                .any(|matcher| language::label(matcher.language).is_some_and(package_manifests)),
            Query::SelectWhen { select, exists } => {
                in_manifest_family(select) || in_manifest_family(exists)
            }
            Query::Regex { .. } | Query::File => false,
        }
    }
    code_rule.clauses.iter().any(|clause| match &clause.kind {
        ClauseKind::Forbid(query) => in_manifest_family(query),
        ClauseKind::RequireEach { subject, evidence } => {
            in_manifest_family(&subject.ast.query) || in_manifest_family(&evidence.ast.query)
        }
        ClauseKind::Budget { query, .. } => in_manifest_family(&query.ast.query),
        ClauseKind::Llm => false,
        ClauseKind::Relational(plan) => plan
            .inputs
            .iter()
            .any(|(_, input)| in_manifest_family(&input.ast.query)),
    })
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

/// A panic poisoned the state, so nothing proves it consistent: the workspace closes, and this
/// call and every later one fail as on a closed workspace, which no host retries, rather than
/// answer busy. The guard still owns the close handoff: dropped as a lease, it clears the state,
/// or leaves that to whichever lease holds the lock by then.
pub(super) fn close_poisoned(
    guard: MutexGuard<'_, WorkspaceState>,
    mutex: &Mutex<WorkspaceState>,
    closed: &AtomicBool,
) -> napi::Error {
    closed.store(true, Ordering::Release);
    drop(StateLease {
        guard: Some(guard),
        mutex,
        closed,
    });
    invalid("policy workspace is closed")
}

pub struct StateLease<'a> {
    pub guard: Option<MutexGuard<'a, WorkspaceState>>,
    pub mutex: &'a Mutex<WorkspaceState>,
    pub closed: &'a AtomicBool,
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

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// Runs once, inside the next storage attach on this thread, while the attach still holds the
    /// workspace lock: a test lands `close` there.
    pub static ON_STORAGE_ATTACHED: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    /// The storage retries and the retry wait (nanoseconds) the last apply on this thread
    /// reported: they reach the host only in profile output.
    pub static LAST_APPLY_STORAGE_RETRIES: std::cell::Cell<(usize, u64)> =
        const { std::cell::Cell::new((0, 0)) };
    /// The files the last apply on this thread left unpublished because the store was busy.
    pub static LAST_APPLY_UNPUBLISHED_FILES: std::cell::Cell<usize> =
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
    pub cache_unavailable: bool,
    pub discovery_milliseconds: f64,
    pub publication_milliseconds: f64,
}

/// Attaches the content store to a workspace whose code rules may already be registered, any time
/// before its first apply: scopes answer `accepts_path` at once while discovery and snapshot
/// publication run on a pool thread.
pub struct WorkspaceStorageTask {
    state: Arc<Mutex<WorkspaceState>>,
    closed: Arc<AtomicBool>,
    /// The workspace's shared object (decision 0442): the task's run is on it.
    shared: Arc<crate::code_rule_dsl::logic::profile::Shared>,
    root: String,
    snapshot: Option<Arc<crate::git_workspace::snapshot::Snapshot>>,
    analysis_owner: Option<AnalysisOwner>,
    analysis_generation: i64,
    expected_store: Option<String>,
    analysis_home: Option<String>,
    /// The store file the host named.
    store_path: PathBuf,
    /// The names whose files the store never holds.
    secret_names: Arc<SecretNames>,
}

#[cfg(not(feature = "headless"))]
impl Task for WorkspaceStorageTask {
    type Output = StorageAttachment;
    type JsValue = StorageAttachment;

    fn compute(&mut self) -> Result<StorageAttachment> {
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || {
            // The task's run, on the workspace's shared object (decision 0442).
            let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
            check_closed(&self.closed)?;
            // A workspace Git cannot describe (no repository, an unreadable untracked directory, a
            // corrupt index) attaches no store: the host walks it, reporting what it cannot read,
            // and interactive edits evaluate without reuse. Only a root outside any repository is
            // expected; any other cause is reported, so reuse is never lost silently.
            let discovery_started = Instant::now();
            let cache_failed = |error: StoreError| StorageAttachment {
                attached: false,
                reason: Some(error.to_string()),
                cache_unavailable: true,
                discovery_milliseconds: discovery_started.elapsed().as_secs_f64() * 1000.0,
                publication_milliseconds: 0.0,
            };
            // The revision is the workspace's from here (decision 0442): this task's reads under its root go to it,
            // and the workspace's state keeps the pin once storage is attached.
            let pinned = self.snapshot.as_ref().map(|snapshot| snapshot.pin(&self.shared));
            let deferred = self.snapshot.is_some() || self.analysis_owner.is_some();
            let (scope_root, scope_home) = (self.root.clone(), self.analysis_home.clone());
            let scope = || crate::git_workspace::snapshot::analysis_scope(Path::new(&scope_root))
                .unwrap_or_else(|| analysis_scope::directory_in(Path::new(&scope_root), scope_home.as_deref()));
            // A pinned revision's inventory is read under its owner, so its scope begins first, on the store it
            // names. A working tree's begins on the store the attach opens, once Git has described the root: a root
            // outside any repository makes no store, and a busy store is waited for once, under the attach's
            // deadline, and reported as the attach reports it.
            let owner = if let Some(owner) = self.analysis_owner.take() {
                Some(owner)
            } else if self.store_path == Path::new(":memory:") || self.snapshot.is_none() {
                None
            } else {
                let store = match crate::content_store::ContentStore::open(&self.store_path) {
                    Ok(store) => store,
                    Err(error) => return Ok(cache_failed(error)),
                };
                if let Err(error) = check_store_identity(&store, self.expected_store.as_deref()) { return Ok(cache_failed(error)); }
                match store.begin_analysis_scope_at(&scope(), self.analysis_generation) {
                    Ok(owner) => Some(owner),
                    Err(error) => return Ok(cache_failed(error)),
                }
            };
            let scoped_after = owner.is_none() && self.snapshot.is_none() && self.store_path != Path::new(":memory:");
            let discovered = if let Some(snapshot) = &self.snapshot {
                use crate::git_workspace::snapshot::InventoryError;
                match snapshot.workspace(&self.store_path, owner.as_ref()) {
                    Ok(discovered) => Ok(discovered),
                    Err(InventoryError::Source(error)) => Err(crate::git_workspace::DiscoveryError::Failed(error)),
                    Err(InventoryError::Cache(error)) => return Ok(cache_failed(error)),
                }
            } else {
                // Only the open is under a Git read's time limit (`git_workspace::discover`): the walk
                // and status of a large checkout may take longer, as they always could.
                crate::git_workspace::discover(std::path::Path::new(&self.root))
            };
            let discovery_milliseconds = discovery_started.elapsed().as_secs_f64() * 1000.0;
            let discovered = match discovered {
                Ok(discovered) => discovered,
                Err(error) => {
                    return Ok(StorageAttachment {
                        attached: false,
                        reason: (!error.is_no_repository()).then(|| error.to_string()),
                        cache_unavailable: false,
                        discovery_milliseconds,
                        publication_milliseconds: 0.0,
                    });
                }
            };
            // A busy store is a cache miss: the workspace runs without reuse, and says why.
            let publication_started = Instant::now();
            let attached = attach_storage(
                &self.root,
                discovered,
                std::mem::take(&mut self.store_path),
                Arc::clone(&self.secret_names),
                deferred,
            )?;
            let publication_milliseconds = publication_started.elapsed().as_secs_f64() * 1000.0;
            let context = match attached {
                Attached::Storage(context) => context,
                Attached::Unavailable(reason) => {
                    return Ok(StorageAttachment {
                        attached: false,
                        reason: Some(reason),
                        cache_unavailable: true,
                        discovery_milliseconds,
                        publication_milliseconds,
                    });
                }
            };
            let owner = if scoped_after {
                if let Err(error) = check_store_identity(&context.store, self.expected_store.as_deref()) { return Ok(cache_failed(error)); }
                match context.store.begin_analysis_scope_at(&scope(), self.analysis_generation) {
                    Ok(owner) => Some(owner),
                    Err(error) => return Ok(cache_failed(error)),
                }
            } else {
                owner
            };
            let guard = match self.state.lock() {
                Ok(guard) => guard,
                Err(error) => {
                    return Err(close_poisoned(
                        error.into_inner(),
                        &self.state,
                        &self.closed,
                    ));
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
            if self.snapshot.is_some() {
                state.import_root = Some(PathBuf::from(&self.root));
                state.import_generation += 1;
                state.git_source = pinned;
            }
            if let Some(owner) = owner {
                if deferred {
                    state.pending_analysis = Some((owner, self.root.clone()));
                } else {
                    context.store.own_analysis_snapshot(&owner, &self.root, context.snapshot_id)
                        .map_err(|err| invalid(err.to_string()))?;
                }
            }
            state.storage = Some(context);
            #[cfg(any(test, feature = "test-hooks"))]
            if let Some(on_attached) = ON_STORAGE_ATTACHED.take() {
                on_attached();
            }
            Ok(StorageAttachment {
                attached: true,
                reason: None,
                cache_unavailable: false,
                discovery_milliseconds,
                publication_milliseconds,
            })
        })
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}

pub struct WorkspaceApplyTask {
    pub state: Arc<Mutex<WorkspaceState>>,
    pub closed: Arc<AtomicBool>,
    /// The workspace's shared object (decision 0442): the task's run is on it.
    pub shared: Arc<crate::code_rule_dsl::logic::profile::Shared>,
    pub request_json: String,
    pub request_buffer: Option<Vec<u8>>,
    pub request_direct: Option<Request>,
}
/// The message a workspace task fails with while another apply holds `cpu_slot`: the host reads this
/// constant (`isWorkspaceBusy`), pauses and calls again.
#[napi]
pub const WORKSPACE_BUSY: &str = "WORKSPACE_BUSY";

/// An operation failed in the store, after the workspace had already attached it.
const WORKSPACE_STORE_UNAVAILABLE: &str = "WORKSPACE_STORE_UNAVAILABLE";

pub(super) fn store_unavailable(error: StoreError) -> napi::Error {
    invalid(format!("{WORKSPACE_STORE_UNAVAILABLE}\n{error}"))
}

/// One apply at a time across the machine's workspaces (decision 0442: the machine's, so a Node environment's
/// workspaces share it). It guards no data, so it is a mutex a panic does not poison: an apply that panics while
/// holding it frees it for the next one.
pub fn cpu_slot() -> Arc<parking_lot::Mutex<()>> {
    #[derive(Default)]
    struct CpuSlot(Arc<parking_lot::Mutex<()>>);
    Arc::clone(&crate::code_rule_dsl::logic::profile::machine::<CpuSlot>().0)
}
impl Task for WorkspaceApplyTask {
    type Output = ResultText;
    type JsValue = ResultText;
    fn compute(&mut self) -> Result<ResultText> {
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || {
            // The task's run, on the workspace's shared object (decision 0442).
            let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
            let Some(_slot) = cpu_slot().try_lock_arc() else {
                return Err(invalid(WORKSPACE_BUSY));
            };
            // Upkeep yields to the apply (decision 0190 §8): its steps stop, their work left due.
            let _run = rerail_accel::upkeep::run_begins(&[]);
            self.apply_with_slot()
        })
    }
    fn resolve(&mut self, _env: Env, output: ResultText) -> Result<ResultText> {
        check_closed(&self.closed)?;
        Ok(output)
    }
    /// The response is in JavaScript and freed by now: purges its pages, which outlive the apply's
    /// own purge.
    fn finally(self, _env: Env) -> Result<()> {
        drop(self);
        crate::code_rule_dsl::prof::purge_allocator_pages();
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
                return Err(close_poisoned(
                    error.into_inner(),
                    &self.state,
                    &self.closed,
                ));
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
        let producer_batch =
            CrossFileScheduler::prepare(&mut state, &mut jobs, &code_rules, deferred);
        #[cfg(all(any(test, feature = "test-hooks"), feature = "p-anchor-corpus"))]
        let anchor_corpus = Arc::clone(&state.anchor_corpus);
        // Every job reads the import targets of the files the workspace holds now, over disk
        // below its root as this apply finds it.
        let import_clauses = state.import_clauses.clone();
        let import_generation = state.import_generation;
        let import_view = (!import_clauses.is_empty())
            .then(|| state.import_files.view(state.import_root.as_deref()));
        let context = JobContext {
            #[cfg(all(any(test, feature = "test-hooks"), feature = "p-anchor-corpus"))]
            anchor_corpus: &anchor_corpus,
            code_rules: &code_rules,
            storage: storage.as_ref(),
            closed: &self.closed,
            summarize: CrossFileScheduler::summarizes(&producer_batch),
            import_view: import_view.as_ref(),
            import_clauses: &import_clauses,
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
        for (path, mut file, work) in phase_a {
            match work {
                Ok(work) => metrics.merge(work),
                Err(error) if evaluation_error.is_none() => evaluation_error = Some(error),
                Err(_) => {}
            }
            file.import_generation = import_generation;
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
        super::scan_fingerprints::capture_local(&mut state, &replaced)?;
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
            // A file named like one holding secrets is evaluated but never published: the store,
            // a cache shared by every checkout, keeps no copy of it, so it stays resident.
            persistence_paths.retain(|path| {
                if !storage.withholds(path) {
                    return true;
                }
                if let Some(file) = state.files.get_mut(path) {
                    file.withheld = true;
                }
                false
            });
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
                        return Err(store_unavailable(error));
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
                metrics.written_sources += publication.written_sources;
                metrics.written_tapes += publication.written_tapes;
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
        #[cfg(any(test, feature = "test-hooks"))]
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
                // A read that failed comes back transient: its clauses are marked below.
                if let Ok(Some(stored)) =
                    evicted_file_states(&state, Some(&only), false).map(|mut stored| stored.pop())
                    && !stored.transient_failure
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
            // Compatibility relational clauses remain batched across dirty rules.
            let mut relational =
                evaluate_batched_relations(&mut state, &dirty, &snapshot, &self.closed)?;
            for index in dirty {
                check_closed(&self.closed)?;
                let budget = MatchBudget::work(&self.closed, CLAUSE_WORK_BUDGET);
                let mut evaluation = evaluate_global(
                    &mut state,
                    index,
                    &snapshot,
                    relational.remove(&index).unwrap_or_default(),
                )?;
                #[cfg(any(test, feature = "test-hooks"))]
                super::pass_deadline_at_test_phase("global-done");
                // Only exhaustion some step observed cut it short; a finished evaluation is kept.
                if budget.observed() {
                    evaluation =
                        overflow_evaluation("runtime", "global evaluation time budget exceeded");
                }
                metrics.visited_nodes += evaluation.metrics.visited_nodes;
                metrics.observe_native_work(&evaluation.metrics);
                super::scan_fingerprints::capture_global(&mut state, &evaluation.violations)?;
                let previous = state.global_results.insert(index, evaluation);
                complete |= !replace_global_changes(
                    previous.as_ref(),
                    &state.global_results[&index],
                    &mut replaced,
                )?;
            }
        }
        check_closed(&self.closed)?;
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
        crate::code_rule_dsl::prof::purge_allocator_pages();
    }
}

pub struct WorkspaceResultPageTask {
    state: Arc<Mutex<WorkspaceState>>,
    closed: Arc<AtomicBool>,
    /// The workspace's shared object (decision 0442): the task's run is on it.
    shared: Arc<crate::code_rule_dsl::logic::profile::Shared>,
    cursor: usize,
    maximum_bytes: usize,
    include_fingerprints: bool,
}

impl Task for WorkspaceResultPageTask {
    type Output = String;
    type JsValue = String;

    fn compute(&mut self) -> Result<String> {
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || {
            // The task's run, on the workspace's shared object (decision 0442).
            let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
            if self.maximum_bytes == 0 || self.maximum_bytes > 64 * 1024 * 1024 {
                return Err(invalid(
                    "result page limit must be between 1 byte and 64 MiB",
                ));
            }
            check_closed(&self.closed)?;
            let guard = match self.state.lock() {
                Ok(guard) => guard,
                Err(error) => {
                    return Err(close_poisoned(
                        error.into_inner(),
                        &self.state,
                        &self.closed,
                    ));
                }
            };
            let mut state = StateLease {
                guard: Some(guard),
                mutex: &self.state,
                closed: &self.closed,
            };
            check_closed(&self.closed)?;
            serialize_result_page(&mut state, self.cursor, self.maximum_bytes, self.include_fingerprints)
        })
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
    retain_scope: bool,
    /// The workspace's shared object (decision 0442): the task's run is on it.
    shared: Arc<crate::code_rule_dsl::logic::profile::Shared>,
}

impl WorkspaceCompletionTask {
    fn lock_state(&self) -> Result<StateLease<'_>> {
        let guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(error) => {
                return Err(close_poisoned(
                    error.into_inner(),
                    &self.state,
                    &self.closed,
                ));
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
        crate::panic_boundary::contain_panic(std::any::type_name::<Self>(), || {
            // The task's run, on the workspace's shared object (decision 0442).
            let _in_run = crate::code_rule_dsl::logic::profile::within(Arc::clone(&self.shared));
            check_closed(&self.closed)?;
            if self.retain_scope {
                let state = self.lock_state()?;
                if let Some((owner, workspace)) = &state.pending_analysis {
                    let storage = state.storage.as_ref().ok_or_else(|| invalid("Repository storage did not attach"))?;
                    storage.store.own_analysis_snapshot(owner, workspace, storage.snapshot_id).map_err(|err| invalid(err.to_string()))?;
                }
                return Ok(());
            }
            // One deadline for the publication under the lock and the completion write after it,
            // so an apply waiting on the lock waits at most one.
            let _deadline = OperationDeadline::enter();
            let (storage, catalog, revision, server_graph) = {
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
                let server_graph = state
                    .code_rules
                    .iter()
                    .any(|code_rule| code_rule.reads_server_graph());
                (storage, catalog, i64::from(state.revision), server_graph)
            };
            let completed_at = unix_seconds()?;
            let written = if let Some(repository) = &storage.repository {
                let mut manifests = super::ingest::repository_context(
                    repository.worktree_root.join(&storage.repository_prefix),
                    server_graph,
                );
                let directories =
                    reusable_directories(repository, &storage.repository_prefix, &mut *manifests);
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
        })
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
