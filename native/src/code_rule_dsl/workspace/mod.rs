//! The incremental code-rule workspace: a host's files, their per-file clause results and the
//! cross-file clauses over them, optionally backed by the shared content store.
//!
//! - `api`: the N-API class and its async tasks.
//! - `ingest`: requests into file state; `index`: repository indexing.
//! - `evaluate`: per-file evaluation; `global`: cross-file clauses.
//! - `crossfile`: the cross-file scheduling seam; `calls_state`: the `calls` adapter's summaries
//!   and closure.
//! - `persisted` and `storage`: stored results and the content store.
//! - `results` and `memory`: reporting and memory accounting.

mod api;
mod calls_state;
mod crossfile;
mod documents;
mod evaluate;
mod global;
mod index;
mod ingest;
mod memory;
mod persisted;
mod results;
mod storage;

use super::json::invalid;
use super::model::{CompiledClause, CompiledCodeRule, Evaluation, FileInput};
use super::query;
use super::scope::ScopeTable;
use super::tape;
use super::violations::Witness;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::content_store::StoredFile;
use global::{Partition, QueryPlan};
use napi::Result;
use results::ResultPageIndex;
#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
#[cfg(test)]
use std::time::Duration;
use storage::StorageContext;

type ClauseId = (usize, usize);
type LocalFailure = (&'static str, &'static str);
type CandidateSets =
    HashMap<(Option<GrammarDialect>, Vec<ClauseId>, Vec<ClauseId>), Arc<Candidates>>;
/// Budget polls a file's evaluation (or a global clause's) may spend, the same on any machine and
/// under any load. Measured over whole corpora (laravel and a Python and a TypeScript application,
/// release build, Apple silicon), evaluations poll 190 (Python packs)
/// to 900 (TypeScript packs) times a millisecond, so this is about 55 to 260 ms of their work; a
/// file whose work polls rarely runs longer (a generated 29,000-line TanStack Query module spends
/// 17,327 units in 520 ms under the TypeScript packs).
#[cfg(not(feature = "no-file-deadline"))]
const FILE_WORK_BUDGET: u64 = 50_000;
/// Benchmark-only: no production evaluation can reach this limit. This reproduces the historical
/// whole-tree full scan without dropping findings from expensive files.
#[cfg(feature = "no-file-deadline")]
const FILE_WORK_BUDGET: u64 = u64::MAX;
/// Persisted per-file clause results are reused only under a key covering every input the result
/// depends on: source content, file context (`file_context`) and engine build (`candidate_catalog`).
const REUSE_LOCAL_RESULTS: bool = true;
/// Directory reuse restores complete per-file results only after a tree-and-package-marker
/// completion record proves the entire clean directory was evaluated.
const REUSE_DIRECTORY_RESULTS: bool = true;
#[cfg(test)]
thread_local! {
    /// Tests shorten the per-file work budget on their own thread to force a transient timeout.
    static TEST_FILE_WORK_BUDGET: Cell<Option<u64>> = const { Cell::new(None) };
    /// Tests stand in another engine build on their own thread.
    static TEST_ENGINE_BUILD_ID: Cell<Option<&'static str>> = const { Cell::new(None) };
    /// Tests inject a failure in the batch local-result storage read.
    static TEST_STORAGE_PREFETCH_FAILURE: Cell<bool> = const { Cell::new(false) };
    /// Tests pass the deadline on their own thread, unpolled, once evaluation reaches this phase
    /// (`pass_deadline_at_test_phase`).
    static TEST_DEADLINE_PASSES_AT: Cell<Option<&'static str>> = const { Cell::new(None) };
}

fn file_work_budget() -> u64 {
    #[cfg(test)]
    if let Some(budget) = TEST_FILE_WORK_BUDGET.get() {
        return budget;
    }
    FILE_WORK_BUDGET
}

#[cfg(test)]
fn pass_deadline_at_test_phase(phase: &str) {
    if TEST_DEADLINE_PASSES_AT.get() == Some(phase) {
        crate::code_rule_dsl::budget::pass_deadline_unpolled();
    }
}

#[cfg(test)]
thread_local! {
    static TEST_PHASE_GATE: RefCell<Option<TestPhaseGate>> = const { RefCell::new(None) };
}

#[cfg(test)]
struct TestPhaseGate {
    phase: &'static str,
    reached: std::sync::mpsc::Sender<std::sync::Weak<String>>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
fn pause_at_test_phase(phase: &str, source: &Arc<String>) {
    TEST_PHASE_GATE.with(|gate| {
        if let Some(gate) = gate.borrow().as_ref().filter(|gate| gate.phase == phase) {
            gate.reached
                .send(Arc::downgrade(source))
                .expect("phase observer");
            gate.release.recv().expect("phase release");
        }
    });
}

#[derive(Default)]
struct Candidates {
    regex: Vec<ClauseId>,
    ast: Vec<ClauseId>,
    by_kind: Vec<Vec<usize>>,
    unrestricted: Vec<usize>,
}

struct FileState {
    input: FileInput,
    source_digest: [u8; 32],
    source_revision: u32,
    tape: Option<(u32, tape::TapeImage)>,
    /// The scope tables `resolves` built for this source revision (one per grammar and annotation
    /// reading), reused by later evaluations.
    scopes: Option<(u32, Vec<Arc<ScopeTable>>)>,
    stored: Option<StoredFile>,
    /// Every remaining AST terminal was absent from a complete, error-free selector index.
    /// Its empty result is retained for persistence without restoring the tape.
    selector_skipped: bool,
    tape_cache_hit: bool,
    tape_decompressed: bool,
    restored_local_results: usize,
    restored_result_ids: HashSet<ClauseId>,
    /// The last evaluation hit its deadline or was cancelled; see `evaluate_file`.
    transient_failure: bool,
    candidates: Arc<Candidates>,
    witnesses: Witnesses,
    failures: BTreeMap<ClauseId, LocalFailure>,
    /// `source-partial` message when the last parse had syntax errors and AST clauses ran around them.
    syntax: Option<String>,
    /// Adapter-owned per-file cross-file state, retained with the file across persistence.
    crossfile: crossfile::CrossFileFileState,
}

/// A file's witnesses by clause, and their violations as a complete result serializes them
/// (`results::rendered`), rendered once: any mutable access drops the rendering.
#[derive(Default)]
struct Witnesses {
    by_clause: BTreeMap<ClauseId, Vec<Witness>>,
    rendered: OnceLock<results::Rendered>,
}

impl std::ops::Deref for Witnesses {
    type Target = BTreeMap<ClauseId, Vec<Witness>>;

    fn deref(&self) -> &Self::Target {
        &self.by_clause
    }
}

impl std::ops::DerefMut for Witnesses {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.rendered.take();
        &mut self.by_clause
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum ExecutionEngine {
    /// The semantic matcher is a test oracle. Production has no engine selector or fallback.
    #[cfg(test)]
    Matcher,
    Sqlite,
}

#[allow(
    clippy::derivable_impls,
    reason = "tests default to the matcher oracle; production defaults to SQLite"
)]
impl Default for ExecutionEngine {
    fn default() -> Self {
        #[cfg(test)]
        {
            Self::Matcher
        }
        #[cfg(not(test))]
        {
            Self::Sqlite
        }
    }
}

impl ExecutionEngine {
    fn as_str(self) -> &'static str {
        match self {
            #[cfg(test)]
            Self::Matcher => "matcher",
            Self::Sqlite => "sqlite",
        }
    }
}

#[derive(Default)]
struct WorkspaceState {
    code_rules: Vec<Arc<CompiledCodeRule>>,
    execution_engine: ExecutionEngine,
    /// Compiled SQLite statements for the registered rules, built on first evaluation.
    sql_plans: Option<Arc<query::SqlPlans>>,
    candidate_sets: CandidateSets,
    globals: BTreeMap<usize, Vec<CompiledClause>>,
    global_results: BTreeMap<usize, Evaluation>,
    dirty_globals: BTreeSet<usize>,
    query_plans: BTreeMap<(usize, String), QueryPlan>,
    /// A cross-file `require_each` or `budget` clause's evidence producers: subject then evidence,
    /// or the budget's query.
    relation_plans: BTreeMap<(usize, String), Vec<ClauseId>>,
    /// Per synthetic clause, the files it has evidence in: witnesses, or unknown. It outlives the
    /// files' residency, so eviction keeps cross-file clauses answerable.
    evidence: BTreeMap<ClauseId, BTreeMap<String, Partition>>,
    files: BTreeMap<String, FileState>,
    /// Directories containing a current Python package marker.
    package_directories: BTreeSet<String>,
    dirty_files: BTreeSet<String>,
    grammars: BTreeMap<String, Arc<tape::Grammar>>,
    storage: Option<StorageContext>,
    /// The repository index the host is driving, chunk by chunk (see `index`).
    repository_index: Option<index::RepositoryIndex>,
    revision: u32,
    /// The current version of every file the workspace evaluated or restored, kept through
    /// eviction so node references can be checked and documents reloaded (see `documents`).
    versions: BTreeMap<String, documents::FileVersion>,
    /// The input (without source) of every evicted file, so complete results can be paged from its
    /// stored results (see `evicted_file_states`).
    evicted_inputs: BTreeMap<String, FileInput>,
    /// Documents loaded through the storage contract, bounded in bytes.
    documents: documents::DocumentCache,
    /// Tells this workspace's node references from another's.
    instance: u64,
    /// Ordered reporting entries, retained for the current revision so result paging does not
    /// repeatedly restore and sort the entire workspace.
    result_page_index: Option<ResultPageIndex>,
    /// Cross-file adapters and their shared derived state.
    crossfile: crossfile::CrossFileScheduler,
}

fn check_closed(closed: &AtomicBool) -> Result<()> {
    if closed.load(Ordering::Acquire) {
        Err(invalid("policy workspace is closed"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod context_tests;
#[cfg(test)]
mod crossfile_tests;
#[cfg(test)]
mod placement_tests;
#[cfg(test)]
mod storage_tests;
#[cfg(test)]
mod tests;
