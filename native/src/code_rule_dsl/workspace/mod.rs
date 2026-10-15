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
use std::time::{Duration, Instant};
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
const FILE_WORK_BUDGET: u64 = 50_000;
/// Persisted per-file clause results are reused only under a key covering every input the result
/// depends on: source content, file context (`file_context`) and engine build (`candidate_catalog`).
const REUSE_LOCAL_RESULTS: bool = true;
/// Directory reuse restores complete per-file results only after a tree-and-package-marker
/// completion record proves the entire clean directory was evaluated.
const REUSE_DIRECTORY_RESULTS: bool = true;
/// How long a workspace leaves a store alone after a write of its own waited out the write
/// deadline (`content_store`'s `BUSY_DEADLINE`): one maintenance wake (`storage.rs`), so a
/// scan's later batches do not each wait out the deadline again.
const STORE_BUSY_COOLDOWN: Duration = Duration::from_secs(30);
/// The source and tape bytes the files a busy store left unpublished may keep resident in all.
/// Past it, later files are released down to their results (`release_unpublished_over_budget`),
/// so a store that stays busy through a long index cannot make the workspace hold the whole
/// repository.
const UNPUBLISHED_RESIDENT_BYTES: usize = 64 * 1024 * 1024;
#[cfg(test)]
thread_local! {
    /// Tests shorten the per-file work budget on their own thread to force a transient timeout.
    static TEST_FILE_WORK_BUDGET: Cell<Option<u64>> = const { Cell::new(None) };
    /// Tests shrink the unpublished files' resident byte budget on their own thread.
    static TEST_UNPUBLISHED_RESIDENT_BYTES: Cell<Option<usize>> = const { Cell::new(None) };
    /// Tests move the busy window's clock forward on their own thread, so a window passes
    /// without a wait (`store_busy_now`).
    static TEST_STORE_BUSY_CLOCK_ADVANCE: Cell<Duration> = const { Cell::new(Duration::ZERO) };
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

fn unpublished_resident_bytes() -> usize {
    #[cfg(test)]
    if let Some(budget) = TEST_UNPUBLISHED_RESIDENT_BYTES.get() {
        return budget;
    }
    UNPUBLISHED_RESIDENT_BYTES
}

/// The clock the busy window is measured on.
fn store_busy_now() -> Instant {
    #[cfg(test)]
    return Instant::now() + TEST_STORE_BUSY_CLOCK_ADVANCE.get();
    #[cfg(not(test))]
    Instant::now()
}

impl WorkspaceState {
    /// Whether the store is inside the window a busy write opened: an apply then treats it as
    /// busy from the start, with no write to it. A window that passed is cleared, so the next
    /// write tries the store again and reopens it only if the store is still busy.
    fn store_cooling_down(&mut self) -> bool {
        match self.store_busy_until {
            Some(until) if store_busy_now() < until => true,
            Some(_) => {
                self.store_busy_until = None;
                false
            }
            None => false,
        }
    }

    /// Records that a write `during` an apply (or a completion) waited out the store's write
    /// deadline: opens the window, and says so on stderr the first time, as the attach does when
    /// it finds the store busy (`rerail: repository reuse is disabled: …`).
    fn note_store_busy(&mut self, during: &str) {
        self.store_busy_until = Some(store_busy_now() + STORE_BUSY_COOLDOWN);
        if !std::mem::replace(&mut self.store_busy_reported, true) {
            eprintln!(
                "rerail: repository reuse is paused: content store busy: another writer held it past the write deadline {during}; results stay in memory and the first apply after {} seconds publishes them",
                STORE_BUSY_COOLDOWN.as_secs()
            );
        }
    }

    /// Records that a publication succeeded: once the window passed (`store_cooling_down`), the
    /// next busy period says so on stderr again.
    fn note_store_published(&mut self) {
        if self.store_busy_until.is_none() {
            self.store_busy_reported = false;
        }
    }

    /// Frees the source and tape of the unpublished files that no longer fit the resident
    /// budget, in path order, and returns how many it freed. Such a file keeps its results
    /// (paging, eviction and the busy window treat it as before) but is never published again:
    /// the store would need its source. Its source comes back from the store, or from the
    /// worktree if it still has the recorded hash, when something needs it (`documents::load`).
    fn release_unpublished_over_budget(&mut self) -> usize {
        let budget = unpublished_resident_bytes();
        let mut resident = 0_usize;
        let mut released = 0;
        for file in self
            .files
            .values_mut()
            .filter(|file| file.unpublished && !file.results_only)
        {
            let bytes = file.input.source.len()
                + file
                    .tape
                    .as_ref()
                    .map_or(0, |(_, image)| image.estimated_bytes());
            if resident.saturating_add(bytes) <= budget {
                resident += bytes;
                continue;
            }
            file.input.source = Arc::new(String::new());
            file.tape = None;
            file.results_only = true;
            released += 1;
        }
        released
    }
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
    /// The store does not hold the results of the file's last evaluation: their publication found
    /// the store busy past its write deadline. However much of the file `stored` says the store
    /// holds, it stays resident through every eviction until a later apply publishes them.
    unpublished: bool,
    /// The unpublished file's source and tape were freed to bound what a busy store keeps
    /// resident (`release_unpublished_over_budget`): its results stand, it is never published,
    /// and its source is empty.
    results_only: bool,
    candidates: Arc<Candidates>,
    witnesses: Witnesses,
    failures: BTreeMap<ClauseId, LocalFailure>,
    /// `source-partial` message when the last parse had syntax errors and AST clauses ran around them.
    syntax: Option<String>,
    /// Adapter-owned per-file cross-file state, retained with the file across persistence.
    crossfile: crossfile::CrossFileFileState,
}

impl FileState {
    /// Reports `clauses` unknown on a file whose source a busy store made the workspace free
    /// (`results_only`) and that nothing can supply again: the reevaluation that needed it cannot
    /// run, so the clauses' earlier answers are dropped and the audit lists the file instead of
    /// the apply failing.
    fn fail_released_source(&mut self, clauses: impl IntoIterator<Item = ClauseId>) {
        for id in clauses {
            self.witnesses.remove(&id);
            self.failures.insert(
                id,
                (
                    "source-incomplete",
                    "source released while the content store was busy and no longer available",
                ),
            );
        }
    }
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

#[derive(Default)]
struct WorkspaceState {
    code_rules: Vec<Arc<CompiledCodeRule>>,
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
    /// Removed paths whose overlay rows a busy store kept (`ingest::apply_request`): the next
    /// apply that publishes deletes them before it publishes its files.
    pending_removals: BTreeSet<String>,
    /// Until when an apply treats the store as busy: a write of the workspace's own waited out
    /// the write deadline (`note_store_busy`), so later applies do not wait it out again.
    store_busy_until: Option<Instant>,
    /// Whether the workspace already said on stderr that the store was busy.
    store_busy_reported: bool,
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
